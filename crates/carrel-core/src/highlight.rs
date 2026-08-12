//! Syntax highlighting as **semantic tokens**.
//!
//! The core classifies syntect's `TextMate` scopes into a small [`TokenKind`]
//! enum; each frontend maps kinds to its own colours. No colour and no ANSI
//! ever appears here — discipline #3 — and research Q28 lists "syntax
//! highlighting *as semantic scopes*" among the components a GTK frontend
//! reuses verbatim.
//!
//! # Cost
//!
//! The bundled syntax set loads once per process, lazily. Blocks over
//! [`MAX_HIGHLIGHT_BYTES`] are not highlighted at all: `fancy-regex` can
//! backtrack, and a pathological megabyte of "code" must not stall parse.

use std::ops::Range;
use std::sync::OnceLock;

use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxSet};

/// Blocks larger than this render plain. A clamp, not an error.
///
/// Sized from measurement, not taste: highlighting through `fancy-regex` runs
/// at ~230 KiB/s on dense Rust, and tokens are computed lazily on a block's
/// FIRST PAINT — so this cap is the worst-case first-view stutter. 32 KiB
/// ≈ 140 ms, once, for a block that is already too big to read as code.
pub const MAX_HIGHLIGHT_BYTES: usize = 32 * 1024;

/// What a run of code *is*, semantically. Frontends map these to colours.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Keyword,
    String,
    Comment,
    Number,
    Function,
    Type,
    Punctuation,
    /// Unclassified. Never stored — a gap between tokens *is* `Plain`.
    Plain,
}

/// A classified run over [`Document::text`](crate::Document::text).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub doc: Range<u32>,
    pub kind: TokenKind,
}

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    // `newlines` variant: lines are fed with their '\n' attached, which is how
    // the block text already arrives.
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

/// The classification table, as pre-parsed scope atoms.
///
/// `Scope::is_prefix_of` compares packed atom representations with **no
/// allocation** — building scope strings per region measured at 108 KiB/s,
/// which made a 25 KB code block cost 220 ms at parse time.
struct Prefixes {
    comment: Scope,
    string: Scope,
    ordered: Vec<(Scope, TokenKind)>,
}

fn prefixes() -> &'static Prefixes {
    static P: OnceLock<Prefixes> = OnceLock::new();
    P.get_or_init(|| {
        let s = |t: &str| Scope::new(t).expect("static scope");
        Prefixes {
            comment: s("comment"),
            string: s("string"),
            // First match wins; more specific prefixes come first.
            ordered: vec![
                (s("constant.numeric"), TokenKind::Number),
                (s("constant.language"), TokenKind::Keyword),
                (s("constant"), TokenKind::Number),
                (s("keyword.operator"), TokenKind::Punctuation),
                (s("keyword"), TokenKind::Keyword),
                (s("storage"), TokenKind::Keyword),
                (s("entity.name.function"), TokenKind::Function),
                (s("support.function"), TokenKind::Function),
                (s("entity.name.type"), TokenKind::Type),
                (s("entity.name.class"), TokenKind::Type),
                (s("entity.name.struct"), TokenKind::Type),
                (s("entity.name.enum"), TokenKind::Type),
                (s("support.type"), TokenKind::Type),
                (s("support.class"), TokenKind::Type),
                (s("punctuation"), TokenKind::Punctuation),
            ],
        }
    })
}

/// Two passes over the stack, because `TextMate` grammars nest delimiters
/// *inside* what they delimit: `//` is `punctuation.definition.comment` inside
/// `comment.line`, and a quote is `punctuation.definition.string` inside
/// `string.quoted`. Visually the delimiter belongs to its container, so
/// comment/string containers are checked against the WHOLE stack first; only
/// then does innermost-first specificity decide the rest.
fn classify(stack: &ScopeStack) -> TokenKind {
    let p = prefixes();
    for scope in stack.as_slice() {
        if p.comment.is_prefix_of(*scope) {
            return TokenKind::Comment;
        }
        if p.string.is_prefix_of(*scope) {
            return TokenKind::String;
        }
    }
    for scope in stack.as_slice().iter().rev() {
        for (prefix, kind) in &p.ordered {
            if prefix.is_prefix_of(*scope) {
                return *kind;
            }
        }
    }
    TokenKind::Plain
}

/// Highlight one code block's text into doc-space tokens.
///
/// `text` is the block's slice of `Document::text`; `doc_base` is where it
/// starts in doc space. An unknown `lang` yields no tokens, as does a line
/// syntect cannot parse — highlighting degrades to plain, never to an error.
#[must_use]
pub fn highlight(lang: &str, text: &str, doc_base: u32) -> Vec<Token> {
    if text.len() > MAX_HIGHLIGHT_BYTES {
        return Vec::new();
    }
    let ss = syntaxes();
    let Some(syntax) = ss.find_syntax_by_token(lang) else {
        return Vec::new();
    };

    let mut parse = ParseState::new(syntax);
    let mut stack = ScopeStack::new();
    let mut out: Vec<Token> = Vec::new();
    let mut line_base = 0usize;

    let push = |out: &mut Vec<Token>, start: usize, end: usize, kind: TokenKind| {
        if start >= end || kind == TokenKind::Plain {
            return;
        }
        let (start, end) = (doc_base + start as u32, doc_base + end as u32);
        // Merge with the previous token when contiguous and same-kind, so the
        // list stays small and the painter's run-splitting stays cheap.
        if let Some(last) = out.last_mut()
            && last.kind == kind
            && last.doc.end == start
        {
            last.doc.end = end;
            return;
        }
        out.push(Token {
            doc: start..end,
            kind,
        });
    };

    for line in text.split_inclusive('\n') {
        let Ok(ops) = parse.parse_line(line, ss) else {
            // This line defeated the grammar; render it plain and continue.
            line_base += line.len();
            continue;
        };
        let mut prev = 0usize;
        for (off, op) in ops {
            push(
                &mut out,
                line_base + prev,
                line_base + off,
                classify(&stack),
            );
            let _ = stack.apply(&op);
            prev = off;
        }
        push(
            &mut out,
            line_base + prev,
            line_base + line.len(),
            classify(&stack),
        );
        line_base += line.len();
    }

    debug_assert!(
        out.iter()
            .all(|t| t.doc.end as usize <= doc_base as usize + text.len()),
        "token escaped the block"
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds_of(lang: &str, text: &str) -> Vec<(String, TokenKind)> {
        highlight(lang, text, 0)
            .into_iter()
            .map(|t| {
                (
                    text[t.doc.start as usize..t.doc.end as usize].to_string(),
                    t.kind,
                )
            })
            .collect()
    }

    #[test]
    fn rust_keywords_strings_and_comments_classify() {
        let toks = kinds_of(
            "rust",
            "fn main() {\n    // greet\n    let s = \"hi\";\n}\n",
        );
        let find = |needle: &str| {
            toks.iter()
                .find(|(s, _)| s.contains(needle))
                .unwrap_or_else(|| panic!("{needle:?} not tokenised: {toks:?}"))
                .1
        };
        assert_eq!(find("fn"), TokenKind::Keyword);
        assert_eq!(find("let"), TokenKind::Keyword);
        assert_eq!(find("// greet"), TokenKind::Comment);
        assert_eq!(find("hi"), TokenKind::String);
    }

    #[test]
    fn an_unknown_language_yields_no_tokens() {
        assert!(highlight("not-a-language", "fn main() {}", 0).is_empty());
    }

    #[test]
    fn tokens_are_sorted_non_overlapping_and_inside_the_text() {
        let text = "def f(x):\n    return x + 1  # done\n";
        let toks = highlight("python", text, 100);
        assert!(!toks.is_empty(), "python should classify something");
        for t in &toks {
            assert!(
                t.doc.start >= 100 && t.doc.end as usize <= 100 + text.len(),
                "{t:?}"
            );
            assert!(t.doc.start < t.doc.end, "empty token {t:?}");
        }
        for w in toks.windows(2) {
            assert!(w[0].doc.end <= w[1].doc.start, "overlap: {w:?}");
        }
    }

    #[test]
    fn an_oversized_block_is_left_plain() {
        let big = "x = 1\n".repeat(MAX_HIGHLIGHT_BYTES / 6 + 1);
        assert!(highlight("python", &big, 0).is_empty());
    }

    #[test]
    fn doc_base_offsets_every_token() {
        let toks = highlight("rust", "fn f() {}", 500);
        assert!(toks.iter().all(|t| t.doc.start >= 500), "{toks:?}");
    }
}
