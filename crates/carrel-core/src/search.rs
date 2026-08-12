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
    let ranges = build_pattern(pattern, flexible_ws).map_or_else(Vec::new, |re| {
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

/// The compiled pattern [`search`] uses, exposed so a frontend's multi-file
/// grep matches EXACTLY what the reader will find after opening the file:
/// same escaping, same `flexible_ws`, same smart-case. Returns `None` for an
/// empty needle.
#[must_use]
pub fn content_pattern(needle: &str, flexible_ws: bool) -> Option<Regex> {
    build_pattern(needle, flexible_ws)
}

/// Build the literal (escaped) pattern. Returns `None` for an empty needle.
fn build_pattern(needle: &str, flexible_ws: bool) -> Option<Regex> {
    if needle.trim().is_empty() {
        return None;
    }
    let body = if flexible_ws {
        needle
            .split_whitespace()
            .map(regex::escape)
            .collect::<Vec<_>>()
            .join(r"\s+")
    } else {
        regex::escape(needle)
    };
    // Smart-case: an all-lowercase needle matches any case; one uppercase
    // letter makes it exact. The vim/less convention, with zero UI.
    let exact = needle.chars().any(char::is_uppercase);
    RegexBuilder::new(&body)
        .case_insensitive(!exact)
        .build()
        .ok()
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
}
