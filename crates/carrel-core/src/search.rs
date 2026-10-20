//! Search — the headline feature.
//!
//! See architecture.md (private notes repo) §3.1 and §7.
//!
//! # Why this survives reflow and resize
//!
//! [`Matches`] contains **no row, no column, no width, no block index, and no
//! node id**. It holds doc-space byte ranges and an index. `apply_resize` in the
//! TUI therefore touches it not at all, which is the entire fix for
//! [mdfried #52](https://github.com/benjajaja/mdfried/issues/52) — where matches
//! are stored as display columns inside `ratatui::text::Line` objects that get
//! rebuilt on every resize.
//!
//! And because the regex runs **once, over the flattened unwrapped
//! [`Document::text`]**, a wrap boundary is not a boundary in the searched string
//! at all. Matching across a soft wrap is not a feature that needs implementing;
//! it is the absence of a bug —
//! [mdfried #53](https://github.com/benjajaja/mdfried/issues/53), where the
//! matcher runs per already-wrapped row and so can never match across two of them.

use std::ops::Range;

use regex::{Regex, RegexBuilder};

use crate::Document;

/// A compiled search and its results.
///
/// Every field is expressed in **space 2** (doc bytes). Nothing here is
/// invalidated by a width change.
#[derive(Clone, Debug)]
pub struct Matches {
    /// The needle, kept so a reload can re-run the search.
    pub pattern: String,
    /// Whether runs of whitespace in the needle match runs of whitespace in the
    /// document. See [`search`] for why this defaults on.
    pub flexible_ws: bool,
    /// Doc-space byte ranges, sorted by `start`, non-overlapping, all non-empty.
    pub ranges: Vec<Range<u32>>,
    /// Index into `ranges`. **Survives resize untouched** — this is what makes
    /// `n`/`N` keep working and what backs the "7 of 42" indicator.
    pub current: Option<usize>,
}

impl Matches {
    #[must_use]
    pub fn len(&self) -> usize {
        self.ranges.len()
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }
    /// `(current, total)`, 1-based, for the status line.
    #[must_use]
    pub fn position(&self) -> Option<(usize, usize)> {
        self.current.map(|i| (i + 1, self.ranges.len()))
    }

    /// Advance to the next or previous match, wrapping around.
    pub fn step(&mut self, forward: bool) -> Option<Range<u32>> {
        if self.ranges.is_empty() {
            return None;
        }
        let n = self.ranges.len();
        self.current = Some(match (self.current, forward) {
            (None, true) => 0,
            (None, false) => n - 1,
            (Some(i), true) => (i + 1) % n,
            (Some(i), false) => (i + n - 1) % n,
        });
        self.current.map(|i| self.ranges[i].clone())
    }

    /// The matches intersecting a half-open doc range, as indices into `ranges`.
    ///
    /// The two comparisons **are** the wrap-affinity rule: a match ending exactly
    /// at `row.start` belongs to the previous row (end biases Left) and a match
    /// starting exactly at `row.end` belongs to the next row (start biases
    /// Right). No affinity field is consulted. See architecture.md (private notes repo) §3.3.
    ///
    /// O(log M + k).
    pub fn intersecting(&self, row: &Range<u32>) -> impl Iterator<Item = (usize, &Range<u32>)> {
        let first = self.ranges.partition_point(|r| r.end <= row.start);
        self.ranges[first..]
            .iter()
            .take_while(move |r| r.start < row.end)
            .enumerate()
            .map(move |(k, r)| (first + k, r))
    }
}

/// Run a search over the document's flattened display text.
///
/// `flexible_ws` should normally be `true`. It makes `two three` match
/// `two  three`, and match across a source-level newline inside a paragraph —
/// which the parser renders as a single space. Without it a user searching for a
/// phrase they can plainly see on screen fails whenever the author happened to
/// hard-wrap the source in the middle of it.
///
/// Zero-width matches (`\b`, `x*`) are filtered: they have no visible extent and
/// would paint as a zero-cell rectangle.
#[must_use]
pub fn search(doc: &Document, pattern: &str, flexible_ws: bool) -> Matches {
    // The soft-hyphen allowance costs the literal prefilter (about 2x per
    // keystroke), so only a text that holds one pays for it.
    let soft = doc.text.contains(SOFT_HYPHEN);
    let ranges = build_pattern(pattern, flexible_ws, soft).map_or_else(Vec::new, |re| {
        re.find_iter(&doc.text)
            .filter(|m| m.end() > m.start())
            .map(|m| m.start() as u32..m.end() as u32)
            .collect()
    });

    Matches {
        pattern: pattern.to_owned(),
        flexible_ws,
        ranges,
        current: None,
    }
}

/// The compiled pattern [`search`] uses on a text with no soft hyphen,
/// exposed so a frontend's multi-file grep matches EXACTLY what the reader
/// will find after opening the file: same escaping, same `flexible_ws`, same
/// smart-case. Returns `None` for an empty needle. A text that holds a soft
/// hyphen (U+00AD) takes [`content_pattern_soft`] instead, as `search` does.
#[must_use]
pub fn content_pattern(needle: &str, flexible_ws: bool) -> Option<Regex> {
    build_pattern(needle, flexible_ws, false)
}

/// [`content_pattern`] for a text that holds a soft hyphen (U+00AD): one
/// may sit between any two characters of the needle, so a word the author
/// divided is found whole.
#[must_use]
pub fn content_pattern_soft(needle: &str, flexible_ws: bool) -> Option<Regex> {
    build_pattern(needle, flexible_ws, true)
}

/// Build the literal (escaped) pattern. Returns `None` for an empty needle.
fn build_pattern(needle: &str, flexible_ws: bool, soft: bool) -> Option<Regex> {
    if needle.trim().is_empty() {
        return None;
    }
    let fold = |w: &str| fold_punctuation(w, soft);
    let body = if flexible_ws {
        needle
            .split_whitespace()
            .map(fold)
            .collect::<Vec<_>>()
            .join(r"\s+")
    } else {
        fold(needle)
    };
    // Smart-case: an all-lowercase needle matches any case; one uppercase
    // letter makes it exact. The vim/less convention, with zero UI.
    let exact = needle.chars().any(char::is_uppercase);
    RegexBuilder::new(&body)
        .case_insensitive(!exact)
        .build()
        .ok()
}

/// U+00AD, the author's own division point.
const SOFT_HYPHEN: char = '\u{AD}';

/// Escape `literal` for the regex, widening each piece of punctuation that
/// smart punctuation rewrites into a class holding both spellings.
///
/// The parser turns `'` `"` `...` `--` `---` into `’` `”` `…` `–` `—` in
/// [`Document::text`] — but not inside code, and not in the source file the
/// home screen greps. So the PATTERN takes both forms and the text is left
/// alone: a typed `don't` finds the displayed `don’t`, a straight quote in a
/// code block, and the source line the grep reads; a curly quote pasted off
/// the screen finds all three as well.
///
/// Hyphen runs read greedily (`---` before `--`), the order the parser uses.
/// A single `-` stays a hyphen: the parser never makes a dash out of one, so
/// widening it would only let the reader find what the grep, reading source,
/// cannot.
fn fold_punctuation(literal: &str, soft: bool) -> String {
    const SINGLE: &str = "['‘’]";
    const DOUBLE: &str = "[\"“”]";
    const ELLIPSIS: &str = r"(?:\.\.\.|…)";
    const EN: &str = "(?:--|–)";
    const EM: &str = "(?:---|—)";
    let mut out = String::with_capacity(literal.len() * 4 + 8);
    let mut rest = literal;
    while let Some(c) = rest.chars().next() {
        // Between any two characters, the author may have written a soft
        // hyphen (U+00AD): invisible mid-row, a division where a row ends
        // at it. The word reads whole, so it is found whole. One typed or
        // pasted into the needle is that allowance, so it is dropped.
        if c == SOFT_HYPHEN {
            rest = &rest[c.len_utf8()..];
            continue;
        }
        if soft && !out.is_empty() {
            out.push_str("\u{AD}*");
        }
        let (piece, len) = match c {
            '\'' | '‘' | '’' => (SINGLE, c.len_utf8()),
            '"' | '“' | '”' => (DOUBLE, c.len_utf8()),
            '…' => (ELLIPSIS, c.len_utf8()),
            '.' if rest.starts_with("...") => (ELLIPSIS, 3),
            '—' => (EM, c.len_utf8()),
            '–' => (EN, c.len_utf8()),
            '-' if rest.starts_with("---") => (EM, 3),
            '-' if rest.starts_with("--") => (EN, 2),
            _ => {
                out.push_str(&regex::escape(&rest[..c.len_utf8()]));
                rest = &rest[c.len_utf8()..];
                continue;
            }
        };
        out.push_str(piece);
        rest = &rest[len..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_across_an_authors_hard_wrapped_source_line() {
        // The source has a newline between "quick" and "brown". The reader shows
        // one paragraph, so searching the visible phrase must work. This is the
        // shape of mdfried #53.
        let d = Document::parse("the quick\nbrown fox");
        let m = search(&d, "quick brown", true);
        assert_eq!(m.len(), 1, "text was {:?}", d.text);
    }

    #[test]
    fn finds_text_across_inline_markup() {
        let d = Document::parse("**bold**text here");
        assert_eq!(search(&d, "boldtext", true).len(), 1);
    }

    #[test]
    fn does_not_match_invisible_link_urls() {
        let d = Document::parse("read [the docs](https://example.com/secret)");
        assert_eq!(search(&d, "secret", true).len(), 0);
        assert_eq!(search(&d, "the docs", true).len(), 1);
    }

    #[test]
    fn ranges_are_sorted_and_non_overlapping() {
        let d = Document::parse("aa aa aa aa");
        let m = search(&d, "aa", true);
        assert_eq!(m.len(), 4);
        for w in m.ranges.windows(2) {
            assert!(w[0].end <= w[1].start);
        }
    }

    #[test]
    fn empty_and_whitespace_needles_yield_nothing() {
        let d = Document::parse("some text");
        assert!(search(&d, "", true).is_empty());
        assert!(search(&d, "   ", true).is_empty());
    }

    #[test]
    fn step_wraps_in_both_directions() {
        let d = Document::parse("x x x");
        let mut m = search(&d, "x", true);
        assert_eq!(m.position(), None);
        m.step(true);
        assert_eq!(m.position(), Some((1, 3)));
        m.step(true);
        m.step(true);
        assert_eq!(m.position(), Some((3, 3)));
        m.step(true);
        assert_eq!(m.position(), Some((1, 3)), "should wrap forward");
        m.step(false);
        assert_eq!(m.position(), Some((3, 3)), "should wrap backward");
    }

    #[test]
    fn intersecting_applies_the_wrap_affinity_rule() {
        let d = Document::parse("abcdefghij");
        let m = Matches {
            pattern: String::new(),
            flexible_ws: false,
            // touches [0,3), exactly abuts 3, exactly abuts 6
            ranges: vec![0..3, 3..6, 6..9],
            current: None,
        };
        // A row covering [3,6): the match ending exactly at 3 belongs to the
        // PREVIOUS row, and the one starting exactly at 6 to the NEXT row.
        let hit: Vec<usize> = m.intersecting(&(3..6)).map(|(i, _)| i).collect();
        assert_eq!(hit, vec![1], "text {:?}", d.text);
    }

    #[test]
    fn a_match_spanning_a_row_boundary_appears_on_both_rows() {
        let m = Matches {
            pattern: String::new(),
            flexible_ws: false,
            // A Vec holding ONE Range is the point — that is what
            // `Matches::ranges` is. `single_range_in_vec_init` reads it as a
            // mistyped `(2..8).collect()`.
            #[allow(clippy::single_range_in_vec_init)]
            ranges: vec![2..8],
            current: None,
        };
        assert_eq!(m.intersecting(&(0..5)).count(), 1);
        assert_eq!(m.intersecting(&(5..10)).count(), 1);
    }
    #[test]
    fn a_phrase_spanning_two_table_cells_matches_across_the_padding() {
        // The user SEES "alpha   beta" side by side; flexible whitespace search
        // must find the phrase across the alignment padding, exactly as it
        // matches across a soft-wrapped source line.
        let d = crate::Document::parse("| alpha | beta |\n|---|---|\n| one | two |\n");
        assert_eq!(search(&d, "alpha beta", true).len(), 1);
        assert_eq!(search(&d, "one two", true).len(), 1);
    }

    #[test]
    fn a_lowercase_needle_is_case_insensitive_and_an_uppercase_one_is_exact() {
        let doc = Document::parse("Hello HELLO hello\n");
        assert_eq!(
            search(&doc, "hello", true).ranges.len(),
            3,
            "smart-case: insensitive"
        );
        assert_eq!(
            search(&doc, "Hello", true).ranges.len(),
            1,
            "one capital turns it exact"
        );
        assert_eq!(search(&doc, "HELLO", true).ranges.len(), 1);
    }

    /// The text each match covers, so a test asserts the range is exactly
    /// the substituted glyphs and not a byte to either side.
    fn hits<'a>(d: &'a Document, needle: &str) -> Vec<&'a str> {
        search(d, needle, true)
            .ranges
            .iter()
            .map(|r| &d.text[r.start as usize..r.end as usize])
            .collect()
    }

    /// Smart punctuation rewrites what the author typed before it reaches
    /// `Document::text`, so a reader typing the keys the author typed found
    /// nothing: `don't` against a displayed `don’t`.
    #[test]
    fn typed_ascii_punctuation_finds_its_smart_form() {
        let d =
            Document::parse("don't say \"hello\" or 'hi'. Wait... pages 10--20 --- an aside.\n");
        assert!(
            d.text.contains("don’t"),
            "fixture lost its smart quote: {:?}",
            d.text
        );
        assert_eq!(hits(&d, "don't"), ["don’t"]);
        assert_eq!(hits(&d, "\"hello\""), ["“hello”"]);
        assert_eq!(hits(&d, "'hi'"), ["‘hi’"]);
        assert_eq!(hits(&d, "wait..."), ["Wait…"]);
        assert_eq!(hits(&d, "10--20"), ["10–20"]);
        assert_eq!(hits(&d, "20 --- an"), ["20 — an"]);
        // A run reads greedily: `---` is the em dash, never an en dash and
        // a hyphen.
        assert!(hits(&d, "10---20").is_empty());
    }

    /// The reverse: a curly quote pasted from the screen must still find a
    /// straight one, which is what a code block holds — smart punctuation
    /// never runs there.
    #[test]
    fn a_typed_smart_form_finds_both_spellings() {
        let d = Document::parse("don't\n\n```\ndon't -- ... \"x\"\n```\n");
        assert_eq!(hits(&d, "don’t"), ["don’t", "don't"]);
        assert_eq!(hits(&d, "don‘t"), ["don’t", "don't"]);
        assert_eq!(hits(&d, "–"), ["--"]);
        assert_eq!(hits(&d, "…"), ["..."]);
        assert_eq!(hits(&d, "“x”"), ["\"x\""]);
    }

    /// Inside code, ASCII is what is there, and an ASCII needle still finds
    /// it: the fold widens a needle, it never replaces it.
    #[test]
    fn ascii_punctuation_in_code_still_matches_itself() {
        let d = Document::parse("```\nlet s = 'a'; x--; y...z\n```\n");
        assert_eq!(hits(&d, "'a'"), ["'a'"]);
        assert_eq!(hits(&d, "x--"), ["x--"]);
        assert_eq!(hits(&d, "y...z"), ["y...z"]);
    }

    /// A single typed hyphen stays a hyphen. Smart punctuation only makes a
    /// dash out of two or more, so a lone `-` in the display text is always
    /// one the author typed — and widening it would let `10-20` find a
    /// displayed `10–20` that the home screen's grep, reading the source
    /// `10--20`, can never find.
    #[test]
    fn a_single_hyphen_is_not_a_dash() {
        let d = Document::parse("pages 10--20 and well-known\n");
        assert!(hits(&d, "10-20").is_empty());
        assert_eq!(hits(&d, "well-known"), ["well-known"]);
    }

    /// A needle with nothing to fold compiles to exactly the pattern it did
    /// before folding existed — so ordinary searches keep their literal
    /// prefilter and the measured per-keystroke cost, by construction.
    #[test]
    fn a_needle_with_nothing_to_fold_is_plainly_escaped() {
        for n in [
            "the",
            "a.b*c",
            "(x)|[y]",
            "well-known",
            "x.y..z",
            "über ß",
            "-",
        ] {
            assert_eq!(fold_punctuation(n, false), regex::escape(n), "{n:?}");
        }
    }

    /// The fold must not disturb escaping, flexible whitespace or smart-case.
    #[test]
    fn folding_keeps_escaping_flexible_whitespace_and_smart_case() {
        let d = Document::parse("It's (a.b) [c]* here...\n\nit's\n");
        assert_eq!(hits(&d, "(a.b) [c]*"), ["(a.b) [c]*"]);
        assert!(hits(&d, "(a?b)").is_empty(), "metacharacters stay literal");
        assert_eq!(hits(&d, "it's"), ["It’s", "it’s"]);
        assert_eq!(hits(&d, "It's"), ["It’s"], "a capital still makes it exact");
        assert_eq!(hits(&d, "[c]*   here..."), ["[c]* here…"]);
    }

    /// A word the author divided with soft hyphens (U+00AD) reads whole,
    /// so it is found whole — and a needle pasted with one in it is the
    /// same needle.
    #[test]
    fn a_soft_hyphen_never_stops_a_match() {
        let d =
            Document::parse("super\u{AD}cali\u{AD}fragi\u{AD}listic and supercalifragilistic\n");
        assert_eq!(
            hits(&d, "supercalifragilistic"),
            [
                "super\u{AD}cali\u{AD}fragi\u{AD}listic",
                "supercalifragilistic"
            ]
        );
        assert_eq!(hits(&d, "cali\u{AD}fragi").len(), 2);
        assert_eq!(
            hits(&d, "i").len(),
            8,
            "a soft hyphen is never a hit of its own"
        );
        // A text with none keeps the plain, prefiltered pattern.
        assert_eq!(
            content_pattern("supercali", true).unwrap().as_str(),
            "supercali"
        );
    }
}
