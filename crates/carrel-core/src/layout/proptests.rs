//! The reflow layer's structural invariants, over random text × random width.
//!
//! Hand-written cases cover the inputs we thought of. These cover the ones we
//! didn't: the `text × width` space is large, and the interesting failures live
//! in combinations nobody enumerates.
//!
//! Six invariants, per the design doc §9.2.

use proptest::prelude::*;
use unicode_segmentation::UnicodeSegmentation;

use super::{RowKind, cluster_width, display_width, wrap_text, wrap_text_soft};
use crate::document::Document;
use crate::position::BlockIdx;
use crate::search::search;

/// Pieces chosen to reach the cases that break line breakers: wide characters,
/// clusters made of several codepoints, whitespace runs, hard newlines, and
/// words longer than any plausible viewport.
///
/// Tabs are deliberately absent: they are expanded at parse and cannot reach
/// layout.
const PIECES: &[&str] = &[
    "a",
    "the",
    "quick",
    "brown",
    "antidisestablishmentarianism",
    // Words the patterns divide, so the soft-break path is actually taken.
    "documentation",
    "international",
    "(readable),",
    " ",
    "  ",
    "\n",
    "日",
    "本語",
    "e\u{301}",                                    // combining acute
    "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}", // ZWJ family
    "🇯🇵",                                          // regional indicators
    "—",
    // Lines with leading whitespace, so per-row budgets are actually exercised
    // once continuation indents exist. Without these, every generated row has
    // indent 0 and the tightened fit invariant degenerates to the old one.
    "\n    ",
    "\n        ",
    // The author's own division points: zero cells mid-row, a hyphen where
    // a row ends at one.
    "\u{AD}",
    "Donau\u{AD}dampf\u{AD}schiff",
];

fn text() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(PIECES), 0..40).prop_map(|parts| parts.concat())
}

/// Wrap and return `(row_text, row_range, row_indent)` triples.
fn rows(text: &str, width: u16) -> Vec<(String, std::ops::Range<u32>, u16)> {
    marked_rows(text, width)
        .into_iter()
        .map(|(s, r, indent, _)| (s, r, indent))
        .collect()
}

/// [`rows`], with whether each row ends in a continuation mark.
fn marked_rows(text: &str, width: u16) -> Vec<(String, std::ops::Range<u32>, u16, bool)> {
    let mut out = Vec::new();
    wrap_text(text, 0, BlockIdx(0), width, 0, &cluster_width, |r| {
        let RowKind::Text { split, hyphen, .. } = r.kind else {
            unreachable!("wrap emits only text rows")
        };
        let row = &text[r.doc.start as usize..r.doc.end as usize];
        assert!(
            !hyphen || row.ends_with('\u{AD}'),
            "plain wrapping divided a word the author did not: {row:?}"
        );
        // An author's division reserves its hyphen's cell like a cut does.
        let split = split || hyphen;
        out.push((
            text[r.doc.start as usize..r.doc.end as usize].to_string(),
            r.doc.clone(),
            r.indent,
            split,
        ));
    });
    out
}

/// Wrap with word division on: `(row_text, row_range, row_indent, hyphen)`.
fn soft_rows(text: &str, width: u16) -> Vec<(String, std::ops::Range<u32>, u16, bool)> {
    let mut out = Vec::new();
    wrap_text_soft(text, 0, BlockIdx(0), width, 0, &cluster_width, true, |r| {
        let RowKind::Text { hyphen, split, .. } = r.kind else {
            unreachable!("wrap emits only text rows")
        };
        assert!(!(hyphen && split), "one mark per row");
        // Both reserve the same one cell, so the invariants below treat a
        // continuation mark exactly as they treat a hyphen.
        let hyphen = hyphen || split;
        out.push((
            text[r.doc.start as usize..r.doc.end as usize].to_string(),
            r.doc.clone(),
            r.indent,
            hyphen,
        ));
    });
    out
}

/// Clusters that take a cell. A lone wide cluster may overhang a row too
/// narrow for it; a soft hyphen beside it takes nothing, so it does not make
/// the overhang a second cluster's fault.
fn visible_clusters(s: &str) -> usize {
    s.graphemes(true).filter(|g| cluster_width(g) > 0).count()
}

/// Doc bytes a match set actually paints at `width`, ignoring whitespace.
///
/// Whitespace is excluded because a break elides it: a space inside a match
/// that happens to fall on a wrap boundary is genuinely not painted, and where
/// the boundaries fall is exactly what changes with width.
fn highlighted_non_ws(doc: &Document, needle: &str, width: u16) -> Vec<u32> {
    let m = search(doc, needle, false);
    let mut out = Vec::new();
    for b in 0..doc.block_count() {
        let block = BlockIdx(b as u32);
        super::wrap(doc, block, width, &cluster_width, |row| {
            for r in &m.ranges {
                // The two half-open comparisons ARE the wrap-affinity rule.
                if r.end <= row.doc.start || r.start >= row.doc.end {
                    continue;
                }
                let lo = r.start.max(row.doc.start);
                let hi = r.end.min(row.doc.end);
                for (off, ch) in doc.text[lo as usize..hi as usize].char_indices() {
                    if !ch.is_whitespace() {
                        out.push(lo + off as u32);
                    }
                }
            }
        });
    }
    out.sort_unstable();
    out.dedup();
    out
}

proptest! {
    /// 1. No row's content exceeds the space its own indent leaves it — unless
    ///    it is a single cluster that cannot fit at all, which overhangs rather
    ///    than vanishing. The budget is per-row, not per-block, because a
    ///    continuation row hangs under its logical line's own indentation.
    ///    A row cut inside a run with no break counts its continuation mark:
    ///    the mark has a cell of its own, or there is no mark.
    #[test]
    fn no_row_exceeds_the_viewport(t in text(), width in 1u16..=200) {
        for (s, _, indent, split) in marked_rows(&t, width) {
            let budget = width.saturating_sub(indent).max(1);
            let cells = display_width(&s) + u16::from(split);
            let fits = cells <= budget;
            let single_cluster = visible_clusters(&s) <= 1 && !split;
            prop_assert!(
                fits || single_cluster,
                "row {s:?} (mark {split}) is {cells} cells with indent {indent} at width {width}",
            );
        }
    }

    /// 8. A cut is not a break: the row after a marked row starts at the
    ///    very byte the marked row ends at — the mark replaces nothing and
    ///    elides nothing — and the run it cut really had no break there.
    #[test]
    fn a_marked_cut_elides_nothing(t in text(), width in 1u16..=200) {
        let rs = marked_rows(&t, width);
        for w in rs.windows(2) {
            if w[0].3 {
                prop_assert_eq!(w[0].1.end, w[1].1.start, "a marked row left a gap");
                prop_assert!(
                    !t[..w[0].1.end as usize].ends_with(char::is_whitespace),
                    "a mark at a space: {:?}", &w[0].0,
                );
            }
        }
    }

    /// 2. Rows are ordered and non-overlapping. Gaps are expected, exactly
    ///    where wrapping elided whitespace.
    #[test]
    fn rows_are_ordered_and_non_overlapping(t in text(), width in 1u16..=200) {
        let rs = rows(&t, width);
        for w in rs.windows(2) {
            prop_assert!(
                w[0].1.end <= w[1].1.start,
                "overlap: {:?} then {:?}", &w[0].1, &w[1].1,
            );
        }
    }

    /// 3. Every non-whitespace byte of the block lands in exactly one row.
    ///    This is what catches a dropped cluster.
    #[test]
    fn every_non_whitespace_byte_appears_exactly_once(t in text(), width in 1u16..=200) {
        let mut count = vec![0u8; t.len()];
        for (_, r, _) in rows(&t, width) {
            for i in r.start..r.end {
                count[i as usize] += 1;
            }
        }
        for (i, ch) in t.char_indices() {
            if !ch.is_whitespace() {
                prop_assert_eq!(
                    count[i], 1,
                    "byte {} ({:?}) covered {} times at width {}", i, ch, count[i], width,
                );
            }
        }
    }

    /// 4. The height pass and the row pass agree. They share one function, and
    ///    this is what makes that a fact rather than an intention.
    #[test]
    fn counting_and_collecting_sinks_agree(t in text(), width in 1u16..=200) {
        let mut counted = 0u32;
        let n = wrap_text(&t, 0, BlockIdx(0), width, 0, &cluster_width, |_| counted += 1);
        prop_assert_eq!(n, counted);
    }

    /// 5. Wrapping is deterministic. Catches iterator state that survives a call.
    #[test]
    fn wrapping_twice_gives_the_same_rows(t in text(), width in 1u16..=200) {
        prop_assert_eq!(rows(&t, width), rows(&t, width));
    }

    /// 7. Dividing words changes none of the above. A divided row fits WITH
    ///    its hyphen, rows stay ordered, and every non-whitespace byte still
    ///    lands in exactly one row — a division adds no character to the
    ///    text and drops none.
    #[test]
    fn dividing_words_keeps_every_invariant(t in text(), width in 1u16..=200) {
        let rs = soft_rows(&t, width);
        let mut count = vec![0u8; t.len()];
        for (s, r, indent, hyphen) in &rs {
            let budget = width.saturating_sub(*indent).max(1);
            let cells = display_width(s) + u16::from(*hyphen);
            prop_assert!(
                cells <= budget || (visible_clusters(s) <= 1 && !hyphen),
                "row {s:?} (hyphen {hyphen}) is {cells} cells in {budget} at width {width}",
            );
            for i in r.start..r.end {
                count[i as usize] += 1;
            }
        }
        for w in rs.windows(2) {
            prop_assert!(w[0].1.end <= w[1].1.start, "overlap: {:?} then {:?}", &w[0].1, &w[1].1);
            // A division is the one break with no gap: it elides nothing.
            if w[0].3 {
                prop_assert_eq!(w[0].1.end, w[1].1.start, "a hyphenated row left a gap");
            }
        }
        for (i, ch) in t.char_indices() {
            if !ch.is_whitespace() {
                prop_assert_eq!(count[i], 1, "byte {} ({:?}) at width {}", i, ch, width);
            }
        }
        // The height pass and the row pass agree with division on, too.
        let n = wrap_text_soft(&t, 0, BlockIdx(0), width, 0, &cluster_width, true, |_| {});
        prop_assert_eq!(n as usize, rs.len());
    }

    /// 6. **The project's thesis.** A search hit recorded at one width paints
    ///    the same characters at any other width, because no search state is
    ///    ever expressed in display coordinates.
    #[test]
    fn search_results_survive_reflow(
        t in text(),
        needle in prop::sample::select(&["a", "the", "日", "e\u{301}", "quick"][..]),
        a in 1u16..=200,
        b in 1u16..=200,
    ) {
        let doc = Document::parse(&t);
        prop_assert_eq!(
            highlighted_non_ws(&doc, needle, a),
            highlighted_non_ws(&doc, needle, b),
            "needle {:?} highlights different characters at widths {} and {}", needle, a, b,
        );
    }

    /// 9. Placing every hit of a row in one walk gives each hit the cells
    ///    the one-hit walk gives it — over any sorted, disjoint ranges,
    ///    including ones that start or end mid-cluster, share a cluster, or
    ///    miss the row altogether.
    #[test]
    fn many_ranges_in_one_walk_match_one_range_at_a_time(
        t in text(),
        base in 0u32..4,
        indent in 0u16..4,
        cuts in prop::collection::vec(0u32..400, 0..24),
    ) {
        let mut cuts = cuts;
        cuts.sort_unstable();
        cuts.dedup();
        let ranges: Vec<_> = cuts.as_chunks::<2>().0.iter().map(|[a, b]| *a..*b).collect();
        let row = t.as_str();
        let many = super::cols_for_doc_ranges(row, base, indent, &ranges);
        prop_assert_eq!(many.len(), ranges.len());
        for (r, got) in ranges.iter().zip(many) {
            let r = r.start.max(base)..r.end;
            let want = super::cols_for_doc_range(row, base, indent, &r);
            prop_assert_eq!(got, want, "range {:?} of {:?}", r, row);
        }
    }
}
