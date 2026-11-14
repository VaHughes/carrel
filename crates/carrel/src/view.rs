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
    let sub = row.checked_sub(l.row_start(b))?;
    let base = l.rows_window(doc, b, sub, 1, &mut rows);
    let r = rows.get((sub - base) as usize)?;
    // The blank row under a block is named by the separator byte after
    // it — in no block's text, and handed by `block_at_doc` to the next
    // block, whose row it is just above ([`row_of`]). It used to take the
    // block's START, like a rule row, so a reader who stopped with the
    // blank under a long paragraph on top came back — on a resume, a
    // reload, a resize — at that paragraph's first row, a screen too
    // early. An empty block's start is its separator already, and the
    // bottom margin has no block after it: those keep the start.
    if let Some(gap) = gap_byte(doc, l, b, sub as usize) {
        return Some(gap);
    }
    // §3.5 edge case: a decoration row has an empty doc range, but its
    // `doc.start` is already a meaningful anchor — image rows pin it to the
    // enclosing node's start, and a card-mode rule row to a byte of its own
    // that no other row holds, the `\n` ending the card above it
    // (`layout::card_rows`; never `node.doc.end`, which can resolve past
    // the block). Falling
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

    /// Wide tables in the shapes that give a card's rows trouble: an empty
    /// last cell (its row sits on the line's `\n`), a row missing cells, a
    /// single column, a quote around the table, a table that ends the
    /// document with and without a final newline, an empty header cell.
    fn wide_tables() -> Vec<String> {
        let long = "a value easily long enough to overflow any narrow terminal";
        let mut out = vec![
            format!(
                "| name | description |\n|---|---|\n| alpha | {long} |\n| beta | short |\n\nafter\n"
            ),
            format!("| a | b | c |\n|---|---|---|\n| x | {long} | |\n| y | | |\n| z |\n\nafter\n"),
            format!("| only |\n|---|\n| {long} |\n| |\n| {long} |\n\nafter\n"),
            format!("> | k | v |\n> |---|---|\n> | one | {long} |\n> | two | |\n\nafter\n"),
            format!("intro\n\n| k | v |\n|---|---|\n| one | {long} |\n| two | {long} |\n"),
            format!("intro\n\n| k | v |\n|---|---|\n| one | {long} |\n| two | |"),
            format!("|  | v |\n|---|---|\n| one | {long} |\n|  | {long} |\n\n# next\n"),
            format!("- item\n\n  | k | v |\n  |---|---|\n  | one | {long} |\n  | two | {long} |\n"),
        ];
        // Generated: rows and columns of varying width, some cells empty.
        for seed in 0..12u32 {
            let ncols = 2 + seed % 3;
            let nrows = 2 + seed % 4;
            let mut s = String::from("lead\n\n|");
            for c in 0..ncols {
                s.push_str(" h");
                s.push_str(&c.to_string());
                s.push_str(" |");
            }
            s.push_str("\n|");
            for _ in 0..ncols {
                s.push_str("---|");
            }
            for r in 0..nrows {
                s.push_str("\n|");
                for c in 0..ncols {
                    let k = (seed * 7 + r * 3 + c * 5) % 6;
                    let cell = match k {
                        0 => String::new(),
                        1 => "w".into(),
                        _ => long[..(k as usize * 9).min(long.len())].into(),
                    };
                    s.push(' ');
                    s.push_str(&cell);
                    s.push_str(" |");
                }
            }
            s.push_str(if seed % 2 == 0 { "\n\ntail\n" } else { "\n" });
            out.push(s);
        }
        out
    }

    /// The card view's rule rows round-trip like every other row. A rule
    /// was named by the next card's first byte, which that card's first
    /// row also holds, so a rule on top came back one row lower after a
    /// resize, a reload or a resume — and the trailing rule, pinned to the
    /// LAST card's first cell, came back a whole card higher (found by
    /// fuzzing, 2026-11-14).
    #[test]
    fn every_card_row_restores_to_itself() {
        let mut srcs = vec![include_str!("../tests/corpus/conformance.md").to_string()];
        srcs.extend(wide_tables());
        let mut failures = Vec::new();
        let mut rules = 0;
        for (i, src) in srcs.iter().enumerate() {
            let doc = Document::parse(src);
            // Cards, and the column view beside them, which must be
            // unaffected: its rows are whole lines and hold no rule.
            let modes = [12u16, 16, 20, 24, 30, 40, 56, 72, 120]
                .into_iter()
                .flat_map(|w| [(w, false), (w, true)]);
            for (width, columns) in modes {
                let lay =
                    Layout::with_images(&doc, width, std::collections::HashMap::new(), columns);
                let l = &lay;
                let height = 5;
                for row in 0..=l.max_scroll(height) {
                    let b = l.block_at_row(row);
                    let node = doc.node_for_block(b);
                    if !matches!(node.kind, carrel_core::NodeKind::Table { .. }) {
                        continue;
                    }
                    let mut v = ViewState::new();
                    v.scroll_to(&doc, l, row, height);
                    let mut rows = Vec::new();
                    l.rows_for(&doc, b, &mut rows);
                    let r = &rows[(row - l.row_start(b)) as usize];
                    let rule = matches!(r.kind, carrel_core::RowKind::Decoration)
                        && r.doc.start != node.doc.start;
                    rules += usize::from(rule);
                    // The one exception, bounded: a single-column table's
                    // EMPTY body cell is a blank row on its line's `\n`,
                    // the line's only byte, so the rule under it has no
                    // byte of its own and keeps the next card's first.
                    let sub = (row - l.row_start(b)) as usize;
                    let ncols = match &node.kind {
                        carrel_core::NodeKind::Table { cols, .. } => cols.len(),
                        _ => 0,
                    };
                    if rule
                        && ncols == 1
                        && sub > 0
                        && rows[sub - 1].doc.is_empty()
                        && matches!(rows[sub - 1].kind, carrel_core::RowKind::Text { .. })
                    {
                        continue;
                    }
                    if rule {
                        // The rule's byte is the table's own, and no text
                        // row holds it: search, selection and clicks never
                        // land on a rule.
                        let at = v.anchor;
                        assert!(doc.text.is_char_boundary(at as usize));
                        assert_eq!(anchor_block(&doc, at), b, "src {i} w {width} row {row}");
                        assert!(
                            rows.iter().all(|t| !t.doc.contains(&at)),
                            "src {i} w {width}: rule byte {at} is in a text row"
                        );
                    }
                    v.restore(&doc, l, height);
                    let restored = v.scroll_row;
                    v.reveal(&doc, l, v.anchor, height, Where::Top);
                    if restored != row || v.scroll_row != row {
                        failures.push((i, width, row, rule, restored, v.scroll_row, v.anchor));
                    }
                }
                // Every byte of a table — its line ends and padding, which
                // no row of the aligned views holds, and a card rule's `\n`
                // — names a row of the table, never the gap below it. The
                // last line's `\n` (the trailing rule's byte) once fell
                // through to the gap in the column view, scrolling the
                // table off the top (cold review, 2026-11-14).
                for t in (0..doc.block_count()).map(|t| BlockIdx(t as u32)) {
                    let node = doc.node_for_block(t);
                    if !matches!(node.kind, carrel_core::NodeKind::Table { .. }) {
                        continue;
                    }
                    for at in node.doc.clone() {
                        if !doc.text.is_char_boundary(at as usize) || anchor_block(&doc, at) != t {
                            continue;
                        }
                        let sub = l.visual_row_of(&doc, t, at, Affinity::Right);
                        if sub >= l.content_height(&doc, t) {
                            failures.push((i, width, u32::MAX, columns, sub, at, at));
                        }
                    }
                }
            }
        }
        assert!(rules > 100, "the fixture must reach card rules: {rules}");
        assert!(
            failures.is_empty(),
            "{} table rows came back elsewhere (src, width, row, rule, restored, revealed, anchor; row u32::MAX: the table byte `anchor` named the gap, (src, width, MAX, column view, row, byte, byte)): {:?}",
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
