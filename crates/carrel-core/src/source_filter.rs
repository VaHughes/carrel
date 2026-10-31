//! Could the reader find this needle in this file? Answered from the SOURCE,
//! without building a document.
//!
//! A multi-file search wants to parse only the files that can match, but the
//! reader searches [`Document::text`](crate::Document::text), the display
//! text, and the display text is not the source. An entity shows as what it
//! names (`&ndash;` is `–`), markup between two words vanishes (`**foo**
//! bar` reads `foo bar`), a link's destination is never shown, list markers
//! and table pipes are decoration, and inline math is rendered (`$x^2$` reads
//! `x²`). A filter that runs the reader's own pattern over the source misses
//! every one of those, and the file list then disagrees with the open file.
//!
//! [`SourceFilter::may_match`] is a SUPERSET test: it is `false` only when
//! the reader's [`search`](crate::search) finds nothing in
//! `Document::parse(src)`. A `true` costs the caller one parse; a wrong
//! `false` costs a file the reader would have found, so every relaxation
//! here leans toward `true`. Three passes, cheapest first:
//!
//! 1. **Exact.** The reader's own pattern ([`content_pattern`]) over the
//!    source. Most files that match at all match here.
//! 2. **Loose.** The needle cut into pieces — one per character, or per
//!    glyph smart punctuation makes — each of which may also be spelled as
//!    any entity, with a GAP allowed between any two: any run of characters
//!    that are not ASCII letters or digits, plus the hidden constructs that
//!    DO hold letters (an entity, an HTML tag, comment or `<script>` body, a
//!    link's destination and title, a reference label or definition, a
//!    wikilink's target, a task box, an ordered-list number, a fence's info
//!    string, a table cell past the header's column count). Every construct
//!    that puts source between two characters the reader shows side by side
//!    is one of those, so the gap covers it; the needle's own whitespace is
//!    only a gap, because a display space can come from a table cell, a
//!    block break or a `<br>` the source spells without one. The gap is a
//!    regex alternation, so it may absorb a construct or read through it —
//!    both are tried, which is what makes this a superset rather than a
//!    guess about which `[` opens a link.
//! 3. **Math.** Inline math is rendered, and rendering reorders and invents
//!    (`\sqrt[3]{x}` shows `x√3`, `\frac{a}{b}` shows `a/b`), so no pattern
//!    over the source can stand in for it. A file that can hold inline math
//!    at all — a `$` that may open one and a later `$` that may close it, by
//!    pulldown-cmark's own flanking rule — is tokenized (no document is
//!    built), every inline math span is replaced by exactly what the reader
//!    shows for it, and the loose pass runs again over that. The spans are
//!    the parser's own, so this pass is exact rather than a superset. That
//!    view ([`math_view`]) depends on the source alone, so a caller that
//!    searches the same files again can keep it
//!    ([`SourceFilter::may_match_with`]).
//!
//! Two edges, neither of them a miss: a needle past `MAX_PIECES` pieces is
//! filtered by its prefix (looser, still a superset), and a pattern that
//! will not compile makes every file a "maybe".
//!
//! The count the caller reports must still come from parsing: this answers
//! "maybe", never "how many".

use regex::Regex;
use regex::bytes::{Regex as BytesRegex, RegexBuilder};

use crate::search::{content_pattern, content_pattern_soft};

/// Any one entity or character reference, as `CommonMark` reads them: up to
/// seven decimal digits, six hex digits, or a name. The longest HTML5 name
/// is 31 characters (`CounterClockwiseContourIntegral`).
const ENT: &str = r"&(?:#[0-9]{1,7}|#[xX][0-9A-Fa-f]{1,6}|[A-Za-z][A-Za-z0-9]{1,31});";

/// What may sit in the source between two characters the reader shows side
/// by side. Each alternative is a superset of a construct the parser hides;
/// the comment beside it names the construct.
const GAP_ATOMS: &[&str] = &[
    // Anything that is not an ASCII letter or digit: whitespace and every
    // block marker, emphasis and strike delimiters, backticks, escapes,
    // pipes, brackets, quote bars, soft hyphens, CR. Non-ASCII letters too —
    // looser than needed, which costs a parse, never a miss.
    r"[^0-9A-Za-z]",
    ENT,
    // An HTML tag, to its `>` or not (an `<img alt="…">` shows what sits
    // inside it, so the gap may stop anywhere in a tag).
    r"<[^>]*",
    // And the rest of one after a quoted value it shows (`" src=x.png>`).
    r#"["'][^>]*>"#,
    // A whole tag whose quoted values hold a `>` (`<b title="1 > 0">`). The
    // two above stop at that `>` and leave the rest stranded; this matters
    // after an inline `<script>`/`<style>`, whose stripper then swallows
    // every later tag whole while the text between them is still shown.
    r#"<(?:[^>"']|"[^"]*"|'[^']*')*>?"#,
    // An HTML comment, to any later `-->`, line end, or the end: an HTML
    // block can end before its comment does (a quote around it closes),
    // and then the comment hid only the rest of that block.
    r"<!--(?s:.)*?(?:-->|\n|\z)",
    // A `<script>` or `<style>` body, which the stripper drops whole — to
    // its closer, or a line end for the same reason.
    r"<(?:script|style)(?s:.)*?(?:</(?:script|style)[^>]*>|\n|\z)",
    // An inline link's destination and title: `](…)`, through any text up
    // to a blank line (a title may hold `)` and may wrap).
    r"\]\((?:[^\n]|\n[ \t]*[^ \t\n])*",
    // A full or collapsed reference: `][label]`.
    r"\]\[(?:[^\]\\]|\\(?s:.))*\]",
    // A reference or footnote definition's label, `[label]:` or `[^n]:`,
    // with whatever follows it on its lines.
    r"\[(?:[^\]\\]|\\(?s:.))*\]:(?:[^\n]|\n[ \t]*[^ \t\n])*",
    // A wikilink's target before the `|` that names its shown text.
    r"\[\[[^\]|]*\|",
    // A task box.
    r"\[[xX]\]",
    // An ordered-list number.
    r"[0-9]{1,9}[.)]",
    // A fence and its info string.
    r"(?:```|~~~)[^\n]*",
    // A table row's cells past the header's column count, which GFM drops:
    // from a pipe to the end of its row.
    r"\|[^\n]*(?:\n|\z)",
];

/// The second code points of the HTML5 entities that decode to TWO
/// characters (`&nlt;` is `<⃒`): a needle holding one may have it from the
/// same entity as the character before, so it is optional. The only letter
/// pair, `&fjlig;`, is handled where pieces are cut.
const ENTITY_TAILS: &[char] = &[
    '\u{331}', '\u{333}', '\u{338}', '\u{20D2}', '\u{20E5}', '\u{FE00}',
];

/// A needle longer than this is filtered by its first `MAX_PIECES` pieces:
/// a display text that holds the needle holds its prefix, so the cut keeps
/// the superset, and it bounds the compiled pattern for a pasted paragraph.
const MAX_PIECES: usize = 32;

/// See the module docs.
#[derive(Clone, Debug)]
pub struct SourceFilter {
    /// The reader's own pattern, for a source with no soft hyphen.
    plain: Regex,
    /// The reader's own pattern, for one with a soft hyphen.
    soft: Regex,
    /// The loose pass. `None` when it would not compile, which makes every
    /// file a "maybe".
    loose: Option<BytesRegex>,
}

impl SourceFilter {
    /// `None` for an empty or whitespace-only needle, as [`content_pattern`].
    #[must_use]
    pub fn new(needle: &str) -> Option<Self> {
        let plain = content_pattern(needle, true)?;
        let soft = content_pattern_soft(needle, true)?;
        let gap = format!("(?-u:{})*", GAP_ATOMS.join("|"));
        let loose = RegexBuilder::new(&pieces(needle, &gap).join(&gap))
            // Always insensitive: a superset of smart-case. The limits are
            // raised because every gap repeats the whole alternation;
            // MAX_PIECES keeps the total bounded.
            .case_insensitive(true)
            .size_limit(64 << 20)
            .dfa_size_limit(64 << 20)
            .build()
            .ok();
        Some(Self { plain, soft, loose })
    }

    /// `false` only when the reader's search for this needle finds nothing
    /// in `Document::parse(src)`.
    #[must_use]
    pub fn may_match(&self, src: &str) -> bool {
        self.may_match_with(src, || math_view(src))
    }

    /// [`may_match`](Self::may_match), with the math view supplied by the
    /// caller — called at most once, and only when the first two passes
    /// fail. It must return what [`math_view`] returns for `src`; it exists
    /// so a caller searching the same files again can keep the views,
    /// which do not depend on the needle and are the filter's costliest
    /// step.
    #[must_use]
    pub fn may_match_with<V: AsRef<str>>(
        &self,
        src: &str,
        math_view: impl FnOnce() -> Option<V>,
    ) -> bool {
        let exact = if src.contains('\u{AD}') {
            &self.soft
        } else {
            &self.plain
        };
        if exact.is_match(src) {
            return true;
        }
        let Some(loose) = &self.loose else {
            return true;
        };
        loose.is_match(src.as_bytes())
            || math_view().is_some_and(|v| loose.is_match(v.as_ref().as_bytes()))
    }
}

/// `src` with every inline math span replaced by exactly what the reader
/// shows for it, or `None` when it holds none. Depends on the source alone,
/// never on a needle, so a caller may keep it for as long as the file is
/// unchanged. A source that cannot hold inline math (see
/// [`may_hold_inline_math`]) costs a byte scan; one that can costs a
/// tokenizer pass, no document build.
#[must_use]
pub fn math_view(src: &str) -> Option<String> {
    if may_hold_inline_math(src) {
        crate::document::inline_math_rendered(src)
    } else {
        None
    }
}

/// Whether pulldown-cmark could find inline math in `src`: a `$` followed
/// by a non-blank (one that may open), and a later `$` preceded by one (one
/// that may close). Its own flanking rule, read without the rest of the
/// parse, so it can only say yes too often — which costs a tokenizer pass.
fn may_hold_inline_math(src: &str) -> bool {
    let b = src.as_bytes();
    let blank = |c: Option<&u8>| c.is_none_or(u8::is_ascii_whitespace);
    let Some(open) = (0..b.len()).find(|&i| b[i] == b'$' && !blank(b.get(i + 1))) else {
        return false;
    };
    (open + 1..b.len()).any(|i| b[i] == b'$' && !blank(b.get(i - 1)))
}

/// The needle as regex pieces, one per character it shows — or per glyph
/// smart punctuation makes, read greedily as [`content_pattern`] reads them
/// — each also accepting any entity. Whitespace and soft hyphens make no
/// piece: the gap stands for them.
fn pieces(needle: &str, gap: &str) -> Vec<String> {
    let alt = |c: char| format!("(?:{}|{ENT})", regex::escape(c.encode_utf8(&mut [0; 4])));
    // A run of ASCII characters the display may show as one glyph, or as
    // the run itself with markup between (`-\-` shows `--`).
    let run = |ascii: char, n: usize, glyph: char| {
        let each: Vec<String> = (0..n).map(|_| alt(ascii)).collect();
        format!("(?:{}|{glyph}|{ENT})", each.join(gap))
    };
    let mut out = Vec::new();
    let mut rest = needle;
    let mut prev: Option<char> = None;
    while let Some(c) = rest.chars().next() {
        if out.len() >= MAX_PIECES {
            break;
        }
        let (piece, len) = match c {
            _ if c.is_whitespace() || c == '\u{AD}' => (None, c.len_utf8()),
            '\'' | '‘' | '’' => (Some(format!("(?:['‘’]|{ENT})")), c.len_utf8()),
            '"' | '“' | '”' => (Some(format!("(?:[\"“”]|{ENT})")), c.len_utf8()),
            '…' => (Some(run('.', 3, '…')), c.len_utf8()),
            '.' if rest.starts_with("...") => (Some(run('.', 3, '…')), 3),
            '—' => (Some(run('-', 3, '—')), c.len_utf8()),
            '–' => (Some(run('-', 2, '–')), c.len_utf8()),
            '-' if rest.starts_with("---") => (Some(run('-', 3, '—')), 3),
            '-' if rest.starts_with("--") => (Some(run('-', 2, '–')), 2),
            // `&fjlig;` shows `fj`: the `f`'s entity may stand for the `j`.
            'j' | 'J' if matches!(prev, Some('f' | 'F')) => (Some(format!("{}?", alt(c))), 1),
            _ if ENTITY_TAILS.contains(&c) => (Some(format!("{}?", alt(c))), c.len_utf8()),
            _ => (Some(alt(c)), c.len_utf8()),
        };
        out.extend(piece);
        prev = Some(c);
        rest = &rest[len..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Document, search};

    /// The superset property, stated directly: any needle cut out of a
    /// document's DISPLAY text is passed by the filter over its SOURCE.
    #[track_caller]
    fn passes(src: &str, needle: &str) {
        let doc = Document::parse(src);
        assert!(
            !search(&doc, needle, true).is_empty(),
            "fixture: reader finds no {needle:?} in {:?}",
            doc.text
        );
        let f = SourceFilter::new(needle).unwrap();
        assert!(f.may_match(src), "{needle:?} missed in {src:?}");
    }

    /// Found by a cold review's proptest: after an inline `<style>` or
    /// `<script>`, a later tag with a `>` inside a quoted value is hidden
    /// whole, so the text either side of it reads side by side.
    #[test]
    fn a_tag_with_a_quoted_angle_bracket_after_a_script_is_bridged() {
        passes("a<style><b class=\"x>y\">z\n", "az");
        passes(
            "See <script>\n\nThe word <span title=\"1 > 0\">here\n",
            "word here",
        );
        passes("x <style><i title='a>b'>y\n", "x y");
    }

    #[test]
    fn a_file_without_the_needle_is_refused() {
        let f = SourceFilter::new("zebra").unwrap();
        assert!(!f.may_match("# Animals\n\nA lion, a **tiger**, [a bear](b.md).\n"));
        assert!(!f.may_match("cost $5 and $10, no stripes\n"));
    }

    /// Only a `$` that can open and a later one that can close send a file
    /// to the tokenizer — pulldown-cmark's flanking rule, so prices and
    /// shell variables mostly do not.
    #[test]
    fn only_a_source_that_can_hold_inline_math_is_tokenized() {
        assert!(may_hold_inline_math("a $x$ b"));
        assert!(
            may_hold_inline_math("$HOME/$USER"),
            "a superset: `$HOME/$` can pair"
        );
        assert!(!may_hold_inline_math("cost $5 and $10"));
        assert!(!may_hold_inline_math("a $ b $ c"));
        assert!(!may_hold_inline_math("x$ then $y"));
        assert!(!may_hold_inline_math("one $"));
    }

    /// The math view is the parser's: a `$` pair it does not read as math
    /// is left alone, and one it does is replaced by what the reader shows.
    #[test]
    fn the_math_view_replaces_exactly_the_parsers_spans() {
        use crate::document::inline_math_rendered;
        assert_eq!(
            inline_math_rendered("a $x^2$ and `$y$` b\n").as_deref(),
            Some("a x² and `$y$` b\n")
        );
        assert_eq!(inline_math_rendered("cost $5 and $10\n"), None);
    }

    #[test]
    fn math_that_reorders_its_source_is_still_passed() {
        passes("a $\\sqrt[3]{x}$ b\n", "x√3");
        passes("a $x^{ab}_{c}$ b\n", "x_c^(ab)");
        passes("a $\\binom{n}{k}$ b\n", "(n/k)");
        passes("a $\\pmod{n}$ b\n", "(modn)");
        passes("$a$ and $b$\n", "a and b");
    }

    #[test]
    fn a_long_needle_compiles_and_still_passes() {
        let words = "the quick brown fox jumps over the lazy dog ".repeat(6);
        let src = format!("**{}**\n", words.trim());
        passes(&src, words.trim());
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(
            std::env::var("SOURCE_FILTER_CASES").ok().and_then(|s| s.parse().ok()).unwrap_or(512)
        ))]
        /// Every substring of the display text that the reader matches is
        /// passed. The parts are the constructs that make display and
        /// source differ, and the loose characters that make them (or fail
        /// to); the needle is cut from the parsed text itself.
        #[test]
        fn every_needle_cut_from_the_display_text_passes(
            parts in proptest::collection::vec(proptest::sample::select(&[
                "foo", "bar", "q", "7", " ", "\n", "\n\n", "**", "_", "~~", "`", "\\*", "\\-",
                "&amp;", "&ndash;", "&#8212;", "&#x41;", "&fjlig;", "&nbsp;", "&nlt;",
                "&#0;", "&", ";", "#",
                "[a](x.md)", "[b][r]", "\n\n[r]: http://e.x \"t\"\n\n", "[[t|al]]",
                "[", "]", "(", ")", "![", "<", ">", "\\", "|", "{", "}", "^", "~", "*",
                "<br>", "<span class=\"c\">", "</span>", "<!-- c -->", "<!--", "-->",
                "<img alt=\"pic\">", "<img src=\"p.png\" alt='pic' width=2>", "\n\n<script>s()</script>\n\n",
                "<details><summary>s</summary>", "</details>", "<?p x ?>",
                "\n- ", "\n1. ", "\n- [x] ", "\n> ", "\n## ", "\n\n```rs\nfn x\n```\n\n",
                "\n    code\n", "\n===\n", "\n***\n", "\n+ ", "\r\n",
                "| c | d |\n|---|---|\n| e | f |\n", "\n|---|\n", "| x |",
                "x^2^", "H~2~O", "$x^2$", "$\\alpha$", "$", "$$",
                "$\\frac{a}{b}$", "$\\sqrt[3]{y}$", "$\\mathbb{R}$", "$a_1^2$",
                "--", "---", "...", "'", "\"",
                "[^n]", "\n\n[^n]: note\n\n", "> [!NOTE]\n> ", "\t", "\u{AD}",
                "www.ex.com/p", "<a@b.c>", "<http://h.x>", "\nterm\n: def\n",
            ][..]), 1..20),
            from in 0usize..400,
            len in 1usize..12,
        ) {
            let src = parts.concat();
            let doc = Document::parse(&src);
            let text = &doc.text;
            // Cut from a non-blank character, so nearly every case tests.
            let chars: Vec<(usize, char)> = text
                .char_indices()
                .filter(|(_, c)| !c.is_whitespace())
                .collect();
            if chars.is_empty() {
                return Ok(());
            }
            let start = chars[from % chars.len()].0;
            let end = text[start..]
                .char_indices()
                .nth(len)
                .map_or(text.len(), |(i, _)| start + i);
            let needle = &text[start..end];
            // The reader may still find nothing — a needle that starts or
            // ends mid-glyph-run reads differently once folded — and then
            // there is no claim to test.
            if search(&doc, needle, true).is_empty() {
                return Ok(());
            }
            let f = SourceFilter::new(needle).unwrap();
            proptest::prop_assert!(f.may_match(&src), "{:?} missed in {:?}", needle, src);
        }
    }
}
