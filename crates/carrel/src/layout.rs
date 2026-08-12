//! Per-width layout: how tall each block is, and which block owns a row.
//!
//! **Valid for exactly one width.** Rebuilt wholesale on resize; nothing here
//! is ever persisted. NO RATATUI — `scripts/check-discipline.sh` rule 6.
//!
//! # There is no row cache
//!
//! `architecture.md` §2.3 specifies an LRU of materialised rows. It is
//! deliberately absent: `wrap` measures 90 MiB/s, so re-wrapping a 50-row
//! viewport costs ~40 µs — 0.25% of a 16 ms frame. The cache would guard a cost
//! that no longer exists, while adding invalidation on width change and an
//! interaction with `wrap_chunk`. If benchmark item 2 (resize latency at p95)
//! ever says otherwise, §2.3's LRU is the escape hatch, and
//! `chunk_count`/`wrap_chunk` already permit keying on `(block, chunk)`.

use std::collections::HashMap;

use carrel_core::{
    Affinity, BlockIdx, Document, Node, NodeKind, Row, RowKind, cluster_width, display_width, wrap,
    wrap_range,
};

/// One blank row between blocks. Markdown renders with paragraph spacing
/// everywhere else it is read; packed-tight rows were a readability bug.
/// The gap belongs to the block BEFORE it.
pub const BLOCK_GAP: u32 = 1;

/// Breathing room after the LAST line — at the bottom of the document the
/// final text sits this many rows above the status bar instead of flush
/// against it. Rides the same gap machinery as block spacing, so anchors,
/// heights, and painting all agree without a special case.
pub const BOTTOM_MARGIN: u32 = 2;

/// Block heights at one width, as a prefix sum.
#[derive(Debug)]
pub struct Layout {
    width: u16,
    /// `len() == block_count + 1`. Heights are differences of adjacent entries,
    /// so there is exactly one source of truth for a block's height.
    block_row_start: Vec<u32>,
    /// Row-height overrides for image blocks whose pixels have arrived.
    /// Plain numbers computed by the frontend from dims × font × width —
    /// no protocol type comes anywhere near this file.
    image_rows: HashMap<BlockIdx, u32>,
    /// When `false` (the default), a table whose aligned form overflows
    /// `width` lays out as cards instead of wrapping in place. `t` in the
    /// reader flips `App::wrap_tables`, which is threaded through here on
    /// every relayout.
    wrap_tables: bool,
}

impl Layout {
    #[must_use]
    pub fn new(doc: &Document, width: u16) -> Self {
        Self::with_images(doc, width, HashMap::new(), false)
    }

    /// Lay out with row-height overrides for ready images.
    ///
    /// An image block whose dimensions are known occupies that many decoration
    /// rows; one whose pixels have not arrived falls back to its wrapped alt
    /// text, which is also the loading, failure, and remote-URL rendering.
    ///
    /// `wrap_tables` selects the table policy: `false` cards an overflowing
    /// table (§3 of the card-view spec), `true` wraps it in place as before.
    #[must_use]
    pub fn with_images(
        doc: &Document,
        width: u16,
        image_rows: HashMap<BlockIdx, u32>,
        wrap_tables: bool,
    ) -> Self {
        let mut block_row_start = Vec::with_capacity(doc.block_count() + 1);
        let mut acc = 0u32;
        block_row_start.push(0);
        for i in 0..doc.block_count() {
            let b = BlockIdx(i as u32);
            let h = if let Some(rows) = image_rows.get(&b) {
                (*rows).max(1)
            } else {
                // The SAME functions the row pass uses, with a counting sink,
                // so the height pass and the row pass cannot disagree.
                let node = doc.node_for_block(b);
                if !wrap_tables && table_overflows(node, width) {
                    card_rows(doc, b, width, |_| {})
                } else {
                    wrap(doc, b, width, &cluster_width, |_| {})
                }
            };
            acc = acc
                .saturating_add(h)
                .saturating_add(self::gap_after(doc, b));
            block_row_start.push(acc);
        }
        Self {
            width,
            block_row_start,
            image_rows,
            wrap_tables,
        }
    }

    #[must_use]
    pub fn width(&self) -> u16 {
        self.width
    }

    /// The table policy this layout was built with. Paint must read THIS,
    /// not `App::wrap_tables` — the two are in sync today, but a layout in
    /// hand answers for itself, and a future caller cannot desynchronise
    /// what it cannot choose.
    #[must_use]
    pub fn wrap_tables(&self) -> bool {
        self.wrap_tables
    }

    #[must_use]
    pub fn total_rows(&self) -> u32 {
        self.block_row_start.last().copied().unwrap_or(0)
    }

    #[must_use]
    pub fn row_start(&self, b: BlockIdx) -> u32 {
        self.block_row_start.get(b.get()).copied().unwrap_or(0)
    }

    #[must_use]
    pub fn height(&self, b: BlockIdx) -> u32 {
        self.block_row_start
            .get(b.get() + 1)
            .copied()
            .unwrap_or(0)
            .saturating_sub(self.row_start(b))
    }

    /// Which block owns an absolute visual row. O(log B).
    #[must_use]
    pub fn block_at_row(&self, row: u32) -> BlockIdx {
        // Entry 0 is always 0, so partition_point returns at least 1 for any
        // row; stepping back one lands on the owning block.
        let i = self.block_row_start.partition_point(|&s| s <= row);
        BlockIdx(i.saturating_sub(1) as u32)
    }

    /// Materialise one block's rows into a caller-owned buffer.
    ///
    /// Takes `&mut Vec` rather than returning one so a frame allocates nothing
    /// after the first block.
    pub fn rows_for(&self, doc: &Document, b: BlockIdx, out: &mut Vec<Row>) {
        out.clear();
        if b.get() >= doc.block_count() {
            return;
        }
        if let Some(rows) = self.image_rows.get(&b) {
            // A ready image is N decoration rows. Their doc ranges are empty
            // at the node's start — which is exactly the §3.5 case the
            // anchor-restore fallback was built for.
            let node = doc.node_for_block(b);
            debug_assert!(matches!(node.kind, NodeKind::Image { .. }));
            for _ in 0..(*rows).max(1) {
                out.push(Row {
                    block: b,
                    doc: node.doc.start..node.doc.start,
                    indent: node.indent,
                    kind: RowKind::Decoration,
                });
            }
        } else {
            let node = doc.node_for_block(b);
            if !self.wrap_tables && table_overflows(node, self.width) {
                card_rows(doc, b, self.width, |r| out.push(r));
            } else {
                wrap(doc, b, self.width, &cluster_width, |r| out.push(r));
            }
        }
        // The spacing row. Anchored (empty) at the block's START so the
        // reanchor fallback and this row agree on where the reader "is".
        let node = doc.node_for_block(b);
        for _ in 0..gap_after(doc, b) {
            out.push(Row {
                block: b,
                doc: node.doc.start..node.doc.start,
                indent: node.indent,
                kind: RowKind::Decoration,
            });
        }
    }

    /// Which row of block `b` a doc-space anchor lands on.
    #[must_use]
    pub fn visual_row_of(&self, doc: &Document, b: BlockIdx, anchor: u32, aff: Affinity) -> u32 {
        let mut rows = Vec::new();
        self.rows_for(doc, b, &mut rows);
        if rows.is_empty() {
            return 0;
        }
        let idx = rows
            .iter()
            .position(|r| anchor < r.doc.end)
            .unwrap_or(rows.len() - 1);
        // END biases LEFT: an anchor exactly on a wrap point belongs to the
        // previous row. See `carrel_core::Affinity`.
        let idx = match aff {
            Affinity::Left if idx > 0 && anchor <= rows[idx].doc.start => idx - 1,
            _ => idx,
        };
        idx as u32
    }

    /// Rows of actual content in a block — its height minus the trailing gap.
    /// The image painter sizes its widget with this, or the picture would
    /// stretch into the spacing row.
    #[must_use]
    pub fn content_height(&self, doc: &Document, b: BlockIdx) -> u32 {
        self.height(b).saturating_sub(gap_after(doc, b))
    }

    /// The largest scroll position that still fills the viewport.
    #[must_use]
    pub fn max_scroll(&self, viewport_h: u16) -> u32 {
        self.total_rows().saturating_sub(u32::from(viewport_h))
    }
}

/// The gap after a block: one row between blocks, the bottom margin after
/// the last.
fn gap_after(doc: &Document, b: BlockIdx) -> u32 {
    if b.get() + 1 < doc.block_count() {
        BLOCK_GAP
    } else {
        BOTTOM_MARGIN
    }
}

/// §3 of the card-view spec: does this table's aligned form exceed `width`?
/// Decided fresh per layout — never stored.
#[must_use]
pub fn table_overflows(node: &Node, width: u16) -> bool {
    let NodeKind::Table { cols, .. } = &node.kind else {
        return false;
    };
    if cols.is_empty() {
        return false;
    }
    let total = cols.iter().map(|&c| u32::from(c)).sum::<u32>()
        + 3 * (cols.len() as u32 - 1)
        + u32::from(node.indent);
    total > u32::from(width)
}

/// Line-end offsets per table row: the byte before the next row's first
/// cell, and the block's (newline-trimmed) end for the last row.
fn table_line_ends(doc: &Document, node: &Node, cell_starts: &[u32], ncols: usize) -> Vec<u32> {
    let nrows = cell_starts.len() / ncols;
    let mut end = node.doc.end;
    while end > node.doc.start && doc.text.as_bytes()[end as usize - 1] == b'\n' {
        end -= 1;
    }
    (0..nrows)
        .map(|r| {
            if r + 1 < nrows {
                cell_starts[(r + 1) * ncols].saturating_sub(1)
            } else {
                end
            }
        })
        .collect()
}

/// The label gutter: widest header cell + 2, clamped to a third of the width.
fn card_gutter(
    doc: &Document,
    cell_starts: &[u32],
    ncols: usize,
    line0_end: u32,
    width: u16,
) -> u16 {
    let widest = (0..ncols)
        .map(|c| {
            let s = cell_starts[c];
            let e = if c + 1 < ncols {
                cell_starts[c + 1]
            } else {
                line0_end
            };
            display_width(doc.text[s as usize..e.max(s) as usize].trim_end())
        })
        .max()
        .unwrap_or(0);
    (widest + 2).min(width / 3).max(2)
}

/// Emit one overflowing table as cards. Returns the row count.
///
/// Legend (the header line, ordinary wrap), then per body row: a rule
/// `Decoration` anchored at the row's first cell, then one run of rows per
/// cell via `wrap_range` under the gutter. Missing cells (start at the line
/// end) are omitted; empty-but-present cells emit one blank row so the card
/// keeps its rhythm. A trailing rule reuses the last body row's own start
/// (`cell_starts[(nrows - 1) * ncols]`, the same anchor as the rule right
/// before it) rather than `node.doc.end` or the table's true last content
/// byte. Two anchors sound wrong for two different-looking rows, but both
/// values are load-bearing for `StableViewport`, restored through
/// `Layout::visual_row_of`'s `anchor < row.doc.end` search:
/// - `node.doc.end` can sit one byte into the inter-block gap (a table
///   followed by a blank line and another block) and `Document::block_at_doc`
///   resolves it to the WRONG, following block.
/// - even the table's own last content byte is exactly the maximum
///   `doc.end` among this block's rows, so `anchor < row.doc.end` never finds
///   a match for it and `visual_row_of` falls back to the row LAST in the
///   list — which, after this trailing rule, is the unrelated block-gap row
///   anchored at `node.doc.start`, not this one.
///
/// The chosen value is a real row boundary strictly inside the block and
/// strictly less than the last row's own end, so it always resolves.
fn card_rows<F: FnMut(Row)>(doc: &Document, b: BlockIdx, width: u16, mut sink: F) -> u32 {
    let node = doc.node_for_block(b);
    let NodeKind::Table { cols, cell_starts } = &node.kind else {
        return 0;
    };
    let ncols = cols.len();
    let nrows = cell_starts.len() / ncols;
    let ends = table_line_ends(doc, node, cell_starts, ncols);
    let gutter = card_gutter(doc, cell_starts, ncols, ends[0], width);
    let indent = node.indent.saturating_add(gutter);
    let mut n = 0u32;
    let rule = |sink: &mut F, at: u32, n: &mut u32| {
        sink(Row {
            block: b,
            doc: at..at,
            indent: node.indent,
            kind: RowKind::Decoration,
        });
        *n += 1;
    };

    // Legend: the header line, at the block's own indent.
    let mut first = true;
    n += wrap_range(
        doc,
        b,
        node.doc.start..ends[0],
        width,
        node.indent,
        &cluster_width,
        &mut first,
        &mut sink,
    );

    for r in 1..nrows {
        rule(&mut sink, cell_starts[r * ncols], &mut n);
        for c in 0..ncols {
            let s = cell_starts[r * ncols + c];
            if s >= ends[r] {
                continue; // missing cell — the row genuinely lacks it
            }
            let e = if c + 1 < ncols {
                cell_starts[r * ncols + c + 1]
            } else {
                ends[r]
            };
            let mut cf = false;
            let got = wrap_range(
                doc,
                b,
                s..e,
                width,
                indent,
                &cluster_width,
                &mut cf,
                &mut sink,
            );
            if got == 0 {
                // Present but empty (padding only): keep the card's rhythm.
                sink(Row {
                    block: b,
                    doc: s..s,
                    indent,
                    kind: RowKind::Text {
                        first_in_block: false,
                        continued: false,
                    },
                });
                n += 1;
            }
            n += got;
        }
    }
    if nrows > 1 {
        rule(&mut sink, cell_starts[(nrows - 1) * ncols], &mut n);
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "# Title\n\nalpha beta gamma delta epsilon\n\n- one\n- two\n";

    #[test]
    fn row_starts_are_a_prefix_sum_and_total_agrees() {
        let doc = Document::parse(SRC);
        let l = Layout::new(&doc, 80);
        let sum: u32 = (0..doc.block_count())
            .map(|i| l.height(BlockIdx(i as u32)))
            .sum();
        assert_eq!(sum, l.total_rows());
    }

    #[test]
    fn block_at_row_finds_the_block_containing_each_row() {
        let doc = Document::parse(SRC);
        let l = Layout::new(&doc, 20);
        for row in 0..l.total_rows() {
            let b = l.block_at_row(row);
            assert!(row >= l.row_start(b), "row {row} before block {b:?}");
            assert!(
                row < l.row_start(b) + l.height(b),
                "row {row} after block {b:?}"
            );
        }
    }

    #[test]
    fn a_narrow_width_produces_more_rows_than_a_wide_one() {
        let doc = Document::parse(SRC);
        assert!(Layout::new(&doc, 12).total_rows() > Layout::new(&doc, 80).total_rows());
    }

    #[test]
    fn visual_row_of_locates_an_anchor_inside_a_wrapped_block() {
        let doc = Document::parse("alpha beta gamma delta epsilon zeta");
        let l = Layout::new(&doc, 12);
        let b = BlockIdx(0);
        let mut rows = Vec::new();
        l.rows_for(&doc, b, &mut rows);
        assert!(rows.len() > 2, "must actually wrap: {rows:?}");
        let anchor = rows[2].doc.start;
        assert_eq!(l.visual_row_of(&doc, b, anchor, Affinity::Right), 2);
    }

    #[test]
    fn rows_for_reuses_the_caller_buffer() {
        let doc = Document::parse(SRC);
        let l = Layout::new(&doc, 80);
        let mut buf = Vec::new();
        l.rows_for(&doc, BlockIdx(0), &mut buf);
        let first = buf.len();
        l.rows_for(&doc, BlockIdx(0), &mut buf);
        assert_eq!(buf.len(), first, "must clear, not append");
    }

    #[test]
    fn an_image_height_override_replaces_the_alt_text_rows() {
        let doc = Document::parse("before\n\n![tall alt text that would wrap](p.png)\n\nafter\n");
        let img = BlockIdx(1);
        assert!(matches!(
            doc.node_for_block(img).kind,
            NodeKind::Image { .. }
        ));

        let plain = Layout::new(&doc, 80);
        let with = Layout::with_images(&doc, 80, HashMap::from([(img, 5u32)]), false);
        // Heights include the one-row block gap; content excludes it.
        assert_eq!(with.content_height(&doc, img), 5);
        assert_eq!(with.height(img), 5 + BLOCK_GAP);
        assert_eq!(
            with.total_rows(),
            plain.total_rows() - plain.height(img) + 5 + BLOCK_GAP,
        );

        let mut rows = Vec::new();
        with.rows_for(&doc, img, &mut rows);
        assert_eq!(
            rows.len(),
            5 + BLOCK_GAP as usize,
            "content plus the gap row"
        );
        for r in &rows {
            assert!(matches!(r.kind, RowKind::Decoration), "{r:?}");
            assert!(r.doc.is_empty(), "decoration rows carry no doc bytes");
            assert_eq!(r.doc.start, doc.node_for_block(img).doc.start);
        }
    }

    #[test]
    fn without_an_override_an_image_block_falls_back_to_alt_text() {
        let doc = Document::parse("![alt words here](p.png)\n");
        let l = Layout::new(&doc, 80);
        let mut rows = Vec::new();
        l.rows_for(&doc, BlockIdx(0), &mut rows);
        // Content rows are text (the alt), followed only by the bottom margin.
        let text_rows = rows
            .iter()
            .take_while(|r| matches!(r.kind, RowKind::Text { .. }))
            .count();
        assert!(text_rows >= 1, "{rows:?}");
        assert_eq!(rows.len() - text_rows, BOTTOM_MARGIN as usize, "{rows:?}");
    }

    #[test]
    fn blocks_are_separated_and_the_document_ends_with_a_margin() {
        let doc = Document::parse("one\n\ntwo\n\nthree\n");
        let l = Layout::new(&doc, 80);
        // Three one-row blocks, two gaps, and the bottom margin.
        assert_eq!(l.total_rows(), 3 + 2 * BLOCK_GAP + BOTTOM_MARGIN);
        let mut rows = Vec::new();
        l.rows_for(&doc, BlockIdx(0), &mut rows);
        assert_eq!(rows.len(), 2, "content row plus its gap");
        assert!(matches!(rows[1].kind, RowKind::Decoration));
        l.rows_for(&doc, BlockIdx(2), &mut rows);
        assert_eq!(
            rows.len(),
            1 + BOTTOM_MARGIN as usize,
            "the last block carries the bottom margin",
        );
        assert!(
            rows[1..]
                .iter()
                .all(|r| matches!(r.kind, RowKind::Decoration))
        );
    }

    #[test]
    fn max_scroll_never_scrolls_past_the_last_screenful() {
        let doc = Document::parse(SRC);
        let l = Layout::new(&doc, 80);
        assert_eq!(
            l.max_scroll(1000),
            0,
            "a viewport taller than the doc cannot scroll"
        );
    }

    const WIDE: &str = "\
| name | description |\n|---|---|\n\
| alpha | a value easily long enough to overflow |\n\
| beta | another long value in the second row |\n";

    fn card_rows(width: u16) -> (carrel_core::Document, Vec<Row>) {
        let doc = Document::parse(WIDE);
        let l = Layout::new(&doc, width);
        let mut rows = Vec::new();
        l.rows_for(&doc, BlockIdx(0), &mut rows);
        (doc, rows)
    }

    #[test]
    fn an_overflowing_table_becomes_cards_and_a_fitting_one_does_not() {
        let doc = Document::parse(WIDE);
        let node = doc.node_for_block(BlockIdx(0));
        let NodeKind::Table { cols, .. } = &node.kind else {
            panic!()
        };
        let fit: u16 = cols.iter().sum::<u16>() + 3 * (cols.len() as u16 - 1);
        assert!(
            !table_overflows(node, fit),
            "exactly fitting is not overflow"
        );
        assert!(table_overflows(node, fit - 1), "one short flips to cards");
        // Cards produce more rows than the two padded lines plus header.
        assert!(
            Layout::new(&doc, fit - 1).height(BlockIdx(0))
                > Layout::new(&doc, fit).height(BlockIdx(0))
        );
    }

    #[test]
    fn card_rows_cover_every_content_byte_in_order() {
        let (doc, rows) = card_rows(30);
        let node = doc.node_for_block(BlockIdx(0));
        // Text rows are ordered and non-overlapping...
        let text_rows: Vec<_> = rows
            .iter()
            .filter(|r| matches!(r.kind, RowKind::Text { .. }))
            .collect();
        for w in text_rows.windows(2) {
            assert!(w[0].doc.end <= w[1].doc.start, "{:?} then {:?}", w[0], w[1]);
        }
        // ...and every byte they skip is whitespace (elided padding / newlines).
        let mut at = node.doc.start;
        for r in &text_rows {
            assert!(
                doc.text[at as usize..r.doc.start as usize]
                    .chars()
                    .all(char::is_whitespace),
                "non-whitespace byte uncovered before {r:?}"
            );
            at = r.doc.end;
        }
        assert!(
            doc.text[at as usize..node.doc.end as usize]
                .chars()
                .all(char::is_whitespace)
        );
    }

    #[test]
    fn cards_put_rules_between_body_rows_and_values_behind_a_gutter() {
        let (doc, rows) = card_rows(30);
        let node = doc.node_for_block(BlockIdx(0));
        // Rules: one per body row plus a trailing one, anchored away from
        // node.doc.start so paint can tell them from the block gap.
        let rules: Vec<_> = rows
            .iter()
            .filter(|r| matches!(r.kind, RowKind::Decoration) && r.doc.start != node.doc.start)
            .collect();
        assert_eq!(rules.len(), 3, "2 body rows + trailing: {rows:?}");
        let NodeKind::Table { cell_starts, cols } = &node.kind else {
            panic!()
        };
        let ncols = cols.len();
        let nrows = cell_starts.len() / ncols;
        // The trailing rule reuses the last body row's own start — the same
        // anchor as the rule right before it — NOT `node.doc.end` or the
        // table's true last content byte. Either of those is either outside
        // the block (in the inter-block gap) or exactly the maximum
        // `doc.end` among this block's rows, which `visual_row_of`'s
        // `anchor < row.doc.end` search can never match. See the doc comment
        // on `card_rows`.
        assert_eq!(
            rules.last().unwrap().doc.start,
            cell_starts[(nrows - 1) * ncols]
        );
        // Value rows are inset by the gutter (widest header "description" + 2,
        // clamped to width/3 = 10 at width 30 — the same clamp exercised by
        // `the_gutter_clamps_to_a_third_of_the_viewport` below).
        let gutter = (carrel_core::display_width("description") + 2).clamp(2, 30 / 3);
        let body = cell_starts[cols.len()];
        for r in rows
            .iter()
            .filter(|r| matches!(r.kind, RowKind::Text { .. }) && r.doc.start >= body)
        {
            assert_eq!(r.indent, node.indent + gutter, "{r:?}");
        }
    }

    #[test]
    fn the_gutter_clamps_to_a_third_of_the_viewport() {
        let doc =
            Document::parse("| an unreasonably verbose header name | x |\n|---|---|\n| v | w |\n");
        let l = Layout::new(&doc, 24);
        let mut rows = Vec::new();
        l.rows_for(&doc, BlockIdx(0), &mut rows);
        let max = rows
            .iter()
            .filter(|r| matches!(r.kind, RowKind::Text { .. }))
            .map(|r| r.indent)
            .max()
            .unwrap();
        assert!(max <= 24 / 3, "gutter {max} must clamp at width/3");
    }

    #[test]
    fn wrap_tables_true_restores_the_old_wrapping_behaviour() {
        let doc = Document::parse(WIDE);
        let cards = Layout::with_images(&doc, 30, HashMap::new(), false);
        let wrapped = Layout::with_images(&doc, 30, HashMap::new(), true);
        let mut r = Vec::new();
        wrapped.rows_for(&doc, BlockIdx(0), &mut r);
        assert!(
            r.iter().all(|row| !matches!(row.kind, RowKind::Decoration)
                || row.doc.start == doc.node_for_block(BlockIdx(0)).doc.start),
            "wrapped mode has no rule rows"
        );
        assert_ne!(cards.height(BlockIdx(0)), wrapped.height(BlockIdx(0)));
    }
}
