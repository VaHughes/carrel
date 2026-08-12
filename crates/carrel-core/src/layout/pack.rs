//! Break units → rows. **Knows nothing about text.**
//!
//! Every invariant the reflow layer promises lives in this file, and every one
//! of them is checkable against units built by hand — which is the whole reason
//! the seam in [`super::units`] exists.

use super::units::Unit;
use super::{LineFit, Row, RowKind};
use crate::position::BlockIdx;

/// Where emitted rows go, plus the state a row emission mutates.
struct Emitter<'a, F: FnMut(Row)> {
    sink: &'a mut F,
    doc_base: u32,
    block: BlockIdx,
    indent: u16,
    /// Whether the next row emitted is the block's first. Threaded across
    /// chunks by the caller, since a chunk boundary is not a block boundary.
    first: &'a mut bool,
    /// Whether the next row emitted continues the current logical line.
    continued: bool,
    rows: u32,
}

impl<F: FnMut(Row)> Emitter<'_, F> {
    fn row(&mut self, start: u32, end: u32) {
        (self.sink)(Row {
            block: self.block,
            doc: self.doc_base + start..self.doc_base + end.max(start),
            indent: self.indent,
            kind: RowKind::Text {
                first_in_block: *self.first,
                continued: self.continued,
            },
        });
        *self.first = false;
        self.rows = self.rows.saturating_add(1);
    }
}

/// Greedily pack one logical line's units into rows.
///
/// The budget is per-row, not per-call: the first row fills against
/// `fit.first_avail`, every later row against `fit.cont_avail`, because a
/// continuation hangs under the line's own indentation and the marker
/// reservation. Called once per logical line, so at most one mandatory break —
/// the line's own terminator — ever arrives, as the last unit.
///
/// Returns the row count. The count is `u32`, not `u16`: a 100 KB paragraph at
/// width 1 is 100,000 rows, and width 1 is a case the reader must survive.
///
/// Every unit is assumed to fit `fit.cont_avail` — the caller splits against
/// the narrower budget — so there is no overflow branch here.
pub(super) fn pack<I, F>(
    units: I,
    doc_base: u32,
    block: BlockIdx,
    fit: &LineFit,
    first: &mut bool,
    sink: &mut F,
) -> u32
where
    I: Iterator<Item = Unit>,
    F: FnMut(Row),
{
    let mut e = Emitter {
        sink,
        doc_base,
        block,
        indent: fit.first_indent,
        first,
        continued: false,
        rows: 0,
    };
    let mut avail = fit.first_avail;

    let mut col = 0u16;
    let mut row_start: Option<u32> = None;
    let mut content_end = 0u32;

    for u in units {
        // The fit test uses CONTENT width. Whitespace that a break elides is
        // allowed to overhang, which is what UAX #14 and every browser do;
        // counting it wraps a column early.
        if col > 0 && col.saturating_add(u.content_width()) > avail {
            let s = row_start.take().unwrap_or(u.range.start);
            e.row(s, content_end);
            col = 0;
            // Every row after the first belongs to the same logical line.
            e.continued = true;
            e.indent = fit.cont_indent;
            avail = fit.cont_avail;
        }
        if row_start.is_none() {
            row_start = Some(u.range.start);
        }
        // Mid-row whitespace is real and painted, so `col` counts the full width.
        col = col.saturating_add(u.width);
        content_end = u.content_end();

        if u.mandatory {
            let s = row_start.take().unwrap_or(u.range.start);
            e.row(s, content_end);
            col = 0;
            // A mandatory break ends the logical line. The caller normally
            // splits before packing, so this is the last unit — but if a
            // mandatory unit ever arrives mid-stream, the next text must start
            // a FRESH line, not inherit this one's continuation state.
            e.continued = false;
            e.indent = fit.first_indent;
            avail = fit.first_avail;
        }
    }

    if let Some(s) = row_start {
        e.row(s, content_end);
    }
    // An empty block is one empty row, not zero rows. Explicit, rather than a
    // `.max(1)` clamp that would also paper over a genuine loss of rows.
    if e.rows == 0 {
        e.row(0, 0);
    }
    e.rows
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit with no text behind it. The point of the seam.
    fn unit(start: u32, end: u32, width: u16, trailing_ws: u16, mandatory: bool) -> Unit {
        Unit {
            range: start..end,
            width,
            trailing_ws,
            trailing_ws_bytes: u32::from(trailing_ws),
            mandatory,
        }
    }

    /// A uniform budget: first and continuation rows get the same `avail`,
    /// which is what every caller before per-line fits effectively had. The
    /// budget-switching behaviour is tested separately below.
    fn uniform(avail: u16) -> LineFit {
        LineFit {
            first_indent: 0,
            cont_indent: 0,
            first_avail: avail,
            cont_avail: avail,
        }
    }

    fn rows_of(units: Vec<Unit>, avail: u16) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        let mut first = true;
        let mut sink = |r: Row| out.push((r.doc.start, r.doc.end));
        pack(
            units.into_iter(),
            0,
            BlockIdx(0),
            &uniform(avail),
            &mut first,
            &mut sink,
        );
        out
    }

    #[test]
    fn the_budget_narrows_after_the_first_row_of_a_logical_line() {
        // Three 4-cell words. First row fits two (avail 9); once continued the
        // budget drops to 4, so the remaining words go one per row.
        let fit = LineFit {
            first_indent: 0,
            cont_indent: 6,
            first_avail: 9,
            cont_avail: 4,
        };
        let mut out = Vec::new();
        let mut first = true;
        let mut sink = |r: Row| out.push((r.indent, r.kind));
        pack(
            vec![
                unit(0, 5, 5, 1, false),
                unit(5, 10, 5, 1, false),
                unit(10, 14, 4, 0, true),
            ]
            .into_iter(),
            0,
            BlockIdx(0),
            &fit,
            &mut first,
            &mut sink,
        );
        assert_eq!(out.len(), 2, "{out:?}");
        assert_eq!(
            out[0],
            (
                0,
                RowKind::Text {
                    first_in_block: true,
                    continued: false
                }
            ),
        );
        assert_eq!(
            out[1],
            (
                6,
                RowKind::Text {
                    first_in_block: false,
                    continued: true
                }
            ),
            "the continuation row carries the continuation indent",
        );
    }

    #[test]
    fn units_that_fit_share_one_row() {
        let rows = rows_of(vec![unit(0, 4, 4, 1, false), unit(4, 7, 3, 0, true)], 10);
        assert_eq!(rows, vec![(0, 7)]);
    }

    #[test]
    fn trailing_whitespace_is_excluded_from_the_fit_test() {
        // 4 cells + 3 content cells = 7, but the elided space means content is
        // 3 + 3 = 6. At avail 7 this must stay on one row.
        let rows = rows_of(vec![unit(0, 4, 4, 1, false), unit(4, 8, 4, 1, true)], 7);
        assert_eq!(rows, vec![(0, 7)]);
    }

    #[test]
    fn a_row_ends_at_content_excluding_the_elided_break_whitespace() {
        let rows = rows_of(vec![unit(0, 4, 4, 1, false), unit(4, 8, 4, 0, true)], 4);
        assert_eq!(rows, vec![(0, 3), (4, 8)], "row 0 stops before the space");
    }

    #[test]
    fn rows_are_ordered_and_non_overlapping() {
        let rows = rows_of(
            vec![
                unit(0, 4, 4, 1, false),
                unit(4, 9, 5, 1, false),
                unit(9, 13, 4, 0, true),
            ],
            5,
        );
        for w in rows.windows(2) {
            assert!(w[0].1 <= w[1].0, "overlap: {rows:?}");
        }
    }

    #[test]
    fn a_mandatory_break_ends_the_row_even_when_there_is_room() {
        let rows = rows_of(vec![unit(0, 2, 2, 1, true), unit(2, 4, 2, 0, true)], 80);
        assert_eq!(rows, vec![(0, 1), (2, 4)]);
    }

    #[test]
    fn no_units_still_produces_one_empty_row() {
        assert_eq!(rows_of(vec![], 80), vec![(0, 0)]);
    }
}
