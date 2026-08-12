//! Regression tests for the two bugs this project exists in order not to have,
//! plus the layout chunker.
//!
//! These use the public API only — if any of them needs an internal, the API is
//! wrong.

use carrel_core::{
    BlockIdx, Document, chunk_count, cluster_width, cols_for_doc_range, search, wrap, wrap_chunk,
};

/// Doc bytes actually painted for a search, at a given width.
fn highlighted(doc: &Document, needle: &str, width: u16) -> Vec<(u32, u32)> {
    let m = search(doc, needle, true);
    let mut out = Vec::new();
    for b in 0..doc.block_count() {
        wrap(doc, BlockIdx(b as u32), width, &cluster_width, |row| {
            for r in &m.ranges {
                if r.end <= row.doc.start || r.start >= row.doc.end {
                    continue;
                }
                let lo = r.start.max(row.doc.start);
                let hi = r.end.min(row.doc.end);
                out.push((lo, hi));
            }
        });
    }
    out
}

/// mdfried #53 — "Search doesn't match line-wrapped strings".
///
/// The cause there was running the matcher over already-wrapped rows
/// (`re.find_iter(&line_string)` per row), which makes a cross-wrap match
/// structurally impossible. Here the matcher runs over unwrapped display text,
/// so the wrap point is invisible to it.
#[test]
fn mdfried_53_a_phrase_spanning_a_soft_wrap_is_still_found() {
    let doc = Document::parse("the quick brown fox jumps over the lazy dog");

    // One match, regardless of where the line happens to break.
    let m = search(&doc, "brown fox", true);
    assert_eq!(m.len(), 1, "the phrase is one match in doc space");

    // At width 16 the rows are "the quick brown" / "fox jumps over" / ...,
    // so the phrase straddles a boundary and must paint on both rows.
    let spans = highlighted(&doc, "brown fox", 16);
    assert_eq!(spans.len(), 2, "painted on two rows: {spans:?}");
    assert!(spans[0].1 <= spans[1].0, "the two halves are disjoint");
}

/// mdfried #52 — "Search matches are lost after resizing the terminal".
///
/// The cause there was storing matches in display coordinates
/// (`LineExtra::SearchMatch(start_col, end_col, ..)` inside a `ratatui::Line`),
/// destroyed by every relayout. Here matches are doc-space byte ranges, so a
/// width change cannot touch them.
#[test]
fn mdfried_52_matches_are_bit_for_bit_identical_across_widths() {
    let src = "The quick brown fox jumps over the lazy dog. \
               The quick brown fox does it again, and again.";
    let doc = Document::parse(src);

    let at_80 = search(&doc, "quick brown", true);
    let at_40 = search(&doc, "quick brown", true);
    assert_eq!(at_80.ranges, at_40.ranges, "the match set is width-free");
    assert_eq!(at_80.len(), 2);

    // And the same characters are painted at every width, even though the rows
    // they land on differ completely.
    // Whitespace is excluded: a space inside a match that lands exactly on a
    // wrap boundary is elided and genuinely not painted, and where the
    // boundaries fall is the one thing width does change.
    let chars_at = |w: u16| -> String {
        highlighted(&doc, "quick brown", w)
            .iter()
            .flat_map(|(a, b)| doc.text[*a as usize..*b as usize].chars())
            .filter(|c| !c.is_whitespace())
            .collect()
    };
    assert_eq!(chars_at(80), chars_at(13));
    assert_eq!(chars_at(80), chars_at(7));
}

/// The current-match index is an index into a doc-space list, so `n`/`N` and the
/// "7 of 42" indicator keep working across a resize with no restoration step.
#[test]
fn the_current_match_index_needs_no_restoration_after_a_resize() {
    let doc = Document::parse("alpha beta alpha beta alpha");
    let mut m = search(&doc, "alpha", true);
    m.current = Some(1);

    // A resize touches layout only. Nothing here has a width in it to update.
    assert_eq!(m.position(), Some((2, 3)));
}

#[test]
fn an_ordinary_block_is_a_single_chunk() {
    let doc = Document::parse("a short paragraph");
    assert_eq!(chunk_count(&doc, BlockIdx(0)), 1);
}

/// A paragraph larger than `CHUNK_BYTES`: chunking must not lose, duplicate, or
/// reorder content, and per-chunk row counts must sum to the whole-block count.
#[test]
fn a_huge_paragraph_is_chunked_without_losing_content() {
    let src = "word ".repeat(20_000); // 100 KB, one paragraph
    let doc = Document::parse(&src);
    let block = BlockIdx(0);
    let width = 80;

    let chunks = chunk_count(&doc, block);
    assert!(chunks > 1, "expected several chunks, got {chunks}");

    let mut whole = Vec::new();
    let total = wrap(&doc, block, width, &cluster_width, |r| whole.push(r.doc));

    let mut per_chunk = Vec::new();
    let mut first = true;
    let mut summed = 0u32;
    for c in 0..chunks {
        summed += wrap_chunk(&doc, block, c, width, &cluster_width, &mut first, |r| {
            per_chunk.push(r.doc);
        });
    }

    assert_eq!(total, summed, "per-chunk row counts must sum to the whole");
    assert_eq!(whole, per_chunk, "chunk-by-chunk equals whole-block");

    // Rows are ordered, non-overlapping, and cover every non-whitespace byte.
    for w in whole.windows(2) {
        assert!(w[0].end <= w[1].start, "overlap at {:?}", w[0]);
    }
    let node = doc.node_for_block(block);
    let covered: usize = whole.iter().map(|r| (r.end - r.start) as usize).sum();
    let non_ws = doc.text[node.doc.start as usize..node.doc.end as usize]
        .chars()
        .filter(|c| !c.is_whitespace())
        .count();
    assert!(covered >= non_ws, "content lost: {covered} < {non_ws}");
}

/// Only the first row of a block carries the marker, or a wrapped list item
/// would repeat its bullet down the left margin.
#[test]
fn only_the_first_row_of_a_wrapped_item_is_marked_first_in_block() {
    let doc = Document::parse("- alpha beta gamma delta epsilon zeta eta theta\n");
    let mut firsts = Vec::new();
    wrap(&doc, BlockIdx(0), 16, &cluster_width, |r| {
        if let carrel_core::RowKind::Text { first_in_block, .. } = r.kind {
            firsts.push(first_in_block);
        }
    });
    assert!(firsts.len() > 1, "the item must actually wrap: {firsts:?}");
    assert!(firsts[0]);
    assert!(firsts[1..].iter().all(|f| !f));
}

/// Hanging indent: every row of a wrapped list item is inset by the marker's
/// width, so continuation lines align under the text rather than the bullet.
#[test]
fn wrapped_list_rows_all_carry_the_marker_width_as_indent() {
    let doc = Document::parse("10. alpha beta gamma delta epsilon zeta\n");
    let mut indents = Vec::new();
    wrap(&doc, BlockIdx(0), 16, &cluster_width, |r| {
        indents.push(r.indent);
    });
    assert!(indents.len() > 1);
    assert!(
        indents.iter().all(|i| *i == 4),
        "\"10. \" is 4 cells on every row: {indents:?}"
    );
}

#[test]
fn highlight_columns_account_for_the_indent_of_a_list_row() {
    let doc = Document::parse("- alpha\n");
    let m = search(&doc, "alpha", true);
    let r = &m.ranges[0];
    let mut cols = None;
    wrap(&doc, BlockIdx(0), 40, &cluster_width, |row| {
        let text = &doc.text[row.doc.start as usize..row.doc.end as usize];
        cols = Some(cols_for_doc_range(text, row.doc.start, row.indent, r));
    });
    assert_eq!(cols, Some((2, 7)), "shifted right by the \"- \" prefix");
}

/// The continuation rule is universal, so prose must be byte-identical to what
/// it was before continuations existed. Prose has no interior leading
/// whitespace, so no row's indent may ever differ from the block's.
#[test]
fn prose_and_lists_wrap_exactly_as_they_did_before_continuations() {
    let doc = Document::parse(
        "A paragraph long enough to wrap several times at a narrow width, with \
         ordinary words and no leading whitespace anywhere inside it.\n\n\
         - a list item that also wraps at a narrow width and must hang\n\
         - another\n\n> a quoted paragraph that wraps too\n",
    );
    for width in [12u16, 20, 40, 80] {
        for b in 0..doc.block_count() {
            let block = BlockIdx(b as u32);
            let node = doc.node_for_block(block);
            let expected = node.indent;
            wrap(&doc, block, width, &cluster_width, |r| {
                assert_eq!(
                    r.indent, expected,
                    "prose row indent changed at width {width}, block {b}",
                );
            });
        }
    }
}
