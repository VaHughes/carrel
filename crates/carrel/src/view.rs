//! Where the reader is looking.
//!
//! **`anchor` is the authority; `scroll_row` is a cache of it at the current
//! width.** Every scroll re-anchors, which is what makes resize stability a
//! consequence of the ordinary path rather than a special case bolted on.
//! NO RATATUI — `scripts/check-discipline.sh` rule 6.

use carrel_core::{Affinity, DocByte, Document};

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
        let b = doc.block_at_doc(DocByte(byte));
        let row = l.row_start(b) + l.visual_row_of(doc, b, byte, Affinity::Right);
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
        let b = doc.block_at_doc(DocByte(self.anchor));
        let sub = l.visual_row_of(doc, b, self.anchor, self.affinity);
        self.scroll_row = (l.row_start(b) + sub).min(l.max_scroll(h));
    }

    fn reanchor(&mut self, doc: &Document, l: &Layout) {
        let b = l.block_at_row(self.scroll_row);
        let mut rows = Vec::new();
        l.rows_for(doc, b, &mut rows);
        let sub = (self.scroll_row - l.row_start(b)) as usize;
        let Some(row) = rows.get(sub) else { return };
        // §3.5 edge case: a decoration row has an empty doc range, but its
        // `doc.start` is already a meaningful anchor — image and gap rows pin
        // it to the enclosing node's start, and a card-mode rule row pins it
        // to a table row's first cell (the trailing rule to the LAST row's,
        // never `node.doc.end` — that offset can resolve past the block).
        // Falling back to `node.doc.start` unconditionally (the
        // pre-card-view behaviour) collapsed every rule row inside a table to
        // the table's own top, losing the reader's actual row on a resize
        // that lands exactly on a rule.
        self.anchor = row.doc.start;
        self.affinity = Affinity::Right;
    }
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
