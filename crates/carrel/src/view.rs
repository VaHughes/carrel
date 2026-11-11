//! Where the reader is looking.
//!
//! **`anchor` is the authority; `scroll_row` is a cache of it at the current
//! width.** Every scroll re-anchors, which is what makes resize stability a
//! consequence of the ordinary path rather than a special case bolted on.
//! NO RATATUI — `scripts/check-discipline.sh` rule 6.

use carrel_core::{Affinity, BlockIdx, DocByte, Document};

use crate::action::Where;
use crate::layout::Layout;

#[derive(Debug)]
pub struct ViewState {
    /// DOC byte of the first content character on the top visible row.
    /// **This is the scroll position.**
    pub anchor: u32,
    pub affinity: Affinity,
    /// DERIVED. Absolute visual row of the viewport top at the current width.
    /// Recomputed from `anchor` after every relayout; never persisted across
    /// a width change.
    pub scroll_row: u32,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            anchor: 0,
            affinity: Affinity::Right,
            scroll_row: 0,
        }
    }
}

impl ViewState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Move to an absolute row, clamp, then re-anchor from the resulting top row.
    pub fn scroll_to(&mut self, doc: &Document, l: &Layout, row: u32, h: u16) {
        self.scroll_row = row.min(l.max_scroll(h));
        self.reanchor(doc, l);
    }

    pub fn scroll_by(&mut self, doc: &Document, l: &Layout, delta: i32, h: u16) {
        let row = if delta < 0 {
            self.scroll_row.saturating_sub(delta.unsigned_abs())
        } else {
            self.scroll_row.saturating_add(delta.unsigned_abs())
        };
        self.scroll_to(doc, l, row, h);
    }

    /// Put `byte` on screen at the requested position.
    pub fn reveal(&mut self, doc: &Document, l: &Layout, byte: u32, h: u16, at: Where) {
        let row = row_of(doc, l, byte, Affinity::Right);
        let top = match at {
            Where::Top => row,
            Where::Middle => row.saturating_sub(u32::from(h) / 2),
            Where::Bottom => row.saturating_sub(u32::from(h).saturating_sub(1)),
        };
        self.scroll_to(doc, l, top, h);
    }

    /// Re-derive `scroll_row` from `anchor` after a width change. §3.5 step 3.
    ///
    /// `anchor` is deliberately NOT recomputed: clamping can move `scroll_row`
    /// at the end of the document, and the reader's position should survive
    /// that rather than be overwritten by it.
    pub fn restore(&mut self, doc: &Document, l: &Layout, h: u16) {
        self.scroll_row = row_of(doc, l, self.anchor, self.affinity).min(l.max_scroll(h));
    }

    fn reanchor(&mut self, doc: &Document, l: &Layout) {
        if let Some(at) = row_anchor(doc, l, self.scroll_row) {
            self.anchor = at;
            self.affinity = Affinity::Right;
        }
    }
}

/// The anchor that names absolute row `row`: the doc byte [`row_of`] turns
/// back into that row. `None` past the last row.
#[must_use]
pub fn row_anchor(doc: &Document, l: &Layout, row: u32) -> Option<u32> {
    let b = l.block_at_row(row);
    let mut rows = Vec::new();
    l.rows_for(doc, b, &mut rows);
    let sub = (row.checked_sub(l.row_start(b))?) as usize;
    let r = rows.get(sub)?;
    // The blank row under a block is named by the separator byte after
    // it — in no block's text, and handed by `block_at_doc` to the next
    // block, whose row it is just above ([`row_of`]). It used to take the
    // block's START, like a rule row, so a reader who stopped with the
    // blank under a long paragraph on top came back — on a resume, a
    // reload, a resize — at that paragraph's first row, a screen too
    // early. An empty block's start is its separator already, and the
    // bottom margin has no block after it: those keep the start.
    if let Some(gap) = gap_byte(doc, l, b, sub) {
        return Some(gap);
    }
    // §3.5 edge case: a decoration row has an empty doc range, but its
    // `doc.start` is already a meaningful anchor — image rows pin it to the
    // enclosing node's start, and a card-mode rule row pins it to a table
    // row's first cell (the trailing rule to the LAST row's, never
    // `node.doc.end` — that offset can resolve past the block). Falling
    // back to `node.doc.start` unconditionally (the pre-card-view
    // behaviour) collapsed every rule row inside a table to the table's own
    // top, losing the reader's actual row on a resize that lands exactly on
    // a rule.
    Some(r.doc.start)
}

/// The separator byte that names row `sub` of block `b`, when that row is
/// the gap under `b` and a separator byte stands between `b` and the next
/// block to name it by.
fn gap_byte(doc: &Document, l: &Layout, b: BlockIdx, sub: usize) -> Option<u32> {
    if (sub as u32) < l.content_height(doc, b) || b.get() + 1 >= doc.block_count() {
        return None;
    }
    let end = doc.node_for_block(b).doc.end;
    let next = doc.node_for_block(BlockIdx(b.get() as u32 + 1)).doc.start;
    (end < next && anchor_block(doc, end).get() == b.get() + 1).then_some(end)
}

/// The absolute row an anchor names: the row of its block it lands on —
/// or, for a byte of the separator before a block (`gap_byte`), the row
/// just above that block, which is the gap it names. Just above, not
/// "the previous block's last row": a folded block before has no rows,
/// and the row above is still the one that was on top.
#[must_use]
pub fn row_of(doc: &Document, l: &Layout, byte: u32, aff: Affinity) -> u32 {
    let b = anchor_block(doc, byte);
    if b.get() > 0 && byte < doc.node_for_block(b).doc.start {
        return l.row_start(b).saturating_sub(1);
    }
    l.row_start(b) + l.visual_row_of(doc, b, byte, aff)
}

/// The block an anchor names — `block_at_doc`, except for an empty block.
///
/// An empty block (an item holding only a nested list, an empty heading)
/// holds no byte, so `reanchor` pins its row to its `doc.start`: the
/// separator after it, which `block_at_doc` hands to the NEXT block. Without
/// this, scrolling an empty block to the top and then resizing, reloading or
/// resuming moved the page — one anchor, two rows. Separators make starts
/// unique, so at most one empty block starts at any byte, and it is the one
/// just before the block `block_at_doc` found.
#[must_use]
pub fn anchor_block(doc: &Document, anchor: u32) -> BlockIdx {
    let b = doc.block_at_doc(DocByte(anchor));
    if let Some(prev) = b.get().checked_sub(1).map(|i| BlockIdx(i as u32)) {
        let node = doc.node_for_block(prev);
        if node.doc.is_empty() && node.doc.start == anchor {
            return prev;
        }
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu";

    #[test]
    fn scrolling_re_anchors_so_the_anchor_always_matches_the_top_row() {
        let doc = Document::parse(SRC);
        let l = Layout::new(&doc, 12);
        let mut v = ViewState::new();
        v.scroll_to(&doc, &l, 2, 4);
        let b = l.block_at_row(v.scroll_row);
        let mut rows = Vec::new();
        l.rows_for(&doc, b, &mut rows);
        let sub = (v.scroll_row - l.row_start(b)) as usize;
        assert_eq!(v.anchor, rows[sub].doc.start);
    }

    #[test]
    fn scroll_is_clamped_at_both_ends() {
        let doc = Document::parse(SRC);
        let l = Layout::new(&doc, 12);
        let mut v = ViewState::new();
        v.scroll_by(&doc, &l, -100, 4);
        assert_eq!(v.scroll_row, 0);
        v.scroll_by(&doc, &l, 10_000, 4);
        assert_eq!(v.scroll_row, l.max_scroll(4));
    }

    /// `StableViewport`: the whole point. A resize must not move the reader.
    #[test]
    fn the_top_doc_byte_survives_a_width_change() {
        let doc = Document::parse(SRC);
        let wide = Layout::new(&doc, 40);
        let mut v = ViewState::new();
        v.scroll_to(&doc, &wide, 1, 3);
        let before = v.anchor;

        let narrow = Layout::new(&doc, 11);
        v.restore(&doc, &narrow, 3);
        assert_eq!(
            v.anchor, before,
            "the anchor is the authority; it must not move"
        );

        let b = narrow.block_at_row(v.scroll_row);
        let sub = narrow.visual_row_of(&doc, b, v.anchor, v.affinity);
        assert_eq!(v.scroll_row, narrow.row_start(b) + sub);
    }

    /// An empty block's row anchors at its `doc.start` — the byte of the
    /// separator after it, which `block_at_doc` gives to the NEXT block. So
    /// `gg` showed the empty block on top while opening (and every resize,
    /// reload or resume) showed the block below it: one anchor, two rows.
    #[test]
    fn an_empty_blocks_row_restores_to_itself() {
        // An empty item (its bullet paints) and an empty heading.
        let doc = Document::parse("-\n\nx\n\n#\n\ny\n");
        let l = Layout::new(&doc, 20);
        let mut v = ViewState::new();
        v.restore(&doc, &l, 3);
        assert_eq!(v.scroll_row, 0, "opening shows the first row");
        for b in [0u32, 2] {
            let row = l.row_start(carrel_core::BlockIdx(b));
            v.scroll_to(&doc, &l, row, 3);
            assert_eq!(v.scroll_row, row);
            v.restore(&doc, &l, 3);
            assert_eq!(v.scroll_row, row, "block {b}: anchor {}", v.anchor);
            v.reveal(&doc, &l, v.anchor, 3, Where::Top);
            assert_eq!(v.scroll_row, row, "block {b}: reveal");
        }
    }

    /// **Every row comes back as itself.** Whatever row is on top, the
    /// anchor it leaves must restore to that same row at the same width —
    /// a resize that changes nothing, a reload, a resume — and reveal it.
    ///
    /// The gap row under a block anchored at the block's START, so a
    /// reader who stopped with the blank under a long paragraph on top came
    /// back at that paragraph's first row, a screen too early (black-box
    /// review, 2026-10-31).
    #[test]
    fn every_top_row_restores_to_itself() {
        let src = include_str!("../tests/corpus/conformance.md");
        let mut failures = Vec::new();
        for width in [20u16, 40, 72, 120] {
            let doc = Document::parse(src);
            let l = Layout::new(&doc, width);
            let h = 5;
            for row in 0..=l.max_scroll(h) {
                let mut v = ViewState::new();
                v.scroll_to(&doc, &l, row, h);
                v.restore(&doc, &l, h);
                let restored = v.scroll_row;
                v.reveal(&doc, &l, v.anchor, h, Where::Top);
                // The one exception, bounded: the gap under an EMPTY block
                // (a `---` rule) shares the block's only byte with the
                // block's own row, so it comes back as that row — one up,
                // the rule instead of the blank under it.
                let b = l.block_at_row(row);
                let empty_gap = doc.node_for_block(b).doc.is_empty() && row > l.row_start(b);
                let want = if empty_gap { row - 1 } else { row };
                if restored != want || v.scroll_row != want {
                    failures.push((width, row, restored, v.scroll_row, v.anchor));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "{} rows came back elsewhere (width, row, restored, revealed, anchor): {:?}",
            failures.len(),
            &failures[..failures.len().min(12)]
        );
    }

    #[test]
    fn reveal_centres_a_byte_in_the_viewport() {
        let doc = Document::parse(SRC);
        let l = Layout::new(&doc, 12);
        let mut v = ViewState::new();
        let target = (doc.text.len() / 2) as u32;
        v.reveal(&doc, &l, target, 5, Where::Middle);
        let b = l.block_at_row(v.scroll_row);
        let row_of_target = l.row_start(b) + l.visual_row_of(&doc, b, target, Affinity::Right);
        assert!(
            row_of_target >= v.scroll_row && row_of_target < v.scroll_row + 5,
            "target row {row_of_target} outside viewport at {}",
            v.scroll_row,
        );
    }
}
