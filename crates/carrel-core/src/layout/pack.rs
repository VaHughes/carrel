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
    fn row(&mut self, start: u32, end: u32, hyphen: bool) {
        (self.sink)(Row {
            block: self.block,
            doc: self.doc_base + start..self.doc_base + end.max(start),
            indent: self.indent,
            kind: RowKind::Text {
                first_in_block: *self.first,
                continued: self.continued,
                hyphen,
            },
        });
        *self.first = false;
        self.rows = self.rows.saturating_add(1);
    }
}

/// Cells a row gives up to the hyphen it ends with.
pub(super) const HYPHEN_COLS: u16 = 1;

/// Rows in a row that may end with a hyphen. A third is a "ladder" — the
/// eye loses its place on the way back to the left margin — so the third
/// takes the ragged edge instead.
const MAX_LADDER: u8 = 2;

/// The smallest hole worth filling with half a word.
///
/// A division needs three cells at least (two letters and the hyphen), and
/// four is where it starts to pay. Past that it scales with the row: a hole of six
/// cells is a third of an 18-cell column and a rounding error in a 60-cell
/// one, and a hyphen is a cost the reader pays to have it filled.
pub(super) fn min_room(avail: u16) -> u16 {
    (avail / 8).max(4)
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
///
/// # Dividing a word
///
/// `divide(unit, room)` is asked, when a unit misses the end of a row, for a
/// head of that unit no wider than `room - HYPHEN_COLS` and the tail that
/// follows it. `None` — always, for a caller that does not hyphenate — takes
/// the break before the unit, as ever. **Whether the hole is worth filling is
/// decided here**, because that is geometry; *where a word may be divided* is
/// text, and stays on the other side of the seam. So the packer still never
/// sees a string, and every rule below is testable with a closure that
/// answers from a table.
pub(super) fn pack<I, F, D>(
    units: I,
    doc_base: u32,
    block: BlockIdx,
    fit: &LineFit,
    first: &mut bool,
    sink: &mut F,
    divide: &mut D,
) -> u32
where
    I: Iterator<Item = Unit>,
    F: FnMut(Row),
    D: FnMut(&Unit, u16) -> Option<(Unit, Unit)>,
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
    // Consecutive rows ended with a hyphen so far.
    let mut ladder = 0u8;

    for mut u in units {
        // The fit test uses CONTENT width. Whitespace that a break elides is
        // allowed to overhang, which is what UAX #14 and every browser do;
        // counting it wraps a column early.
        if col > 0 && col.saturating_add(u.content_width()) > avail {
            let s = row_start.take().unwrap_or(u.range.start);
            // `col` counts the space after the previous word, so `room` is
            // exactly what a head and its hyphen may occupy. The line's last
            // word is never divided: its tail would be a row of its own.
            let room = avail.saturating_sub(col);
            let divided = if ladder < MAX_LADDER && !u.mandatory && room >= min_room(avail) {
                divide(&u, room)
                    .filter(|(head, _)| head.content_width().saturating_add(HYPHEN_COLS) <= room)
            } else {
                None
            };
            if let Some((head, tail)) = divided {
                e.row(s, head.range.end, true);
                ladder += 1;
                u = tail;
            } else {
                e.row(s, content_end, false);
                ladder = 0;
            }
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
            e.row(s, content_end, false);
            col = 0;
            ladder = 0;
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
        e.row(s, content_end, false);
    }
    // An empty block is one empty row, not zero rows. Explicit, rather than a
    // `.max(1)` clamp that would also paper over a genuine loss of rows.
    if e.rows == 0 {
        e.row(0, 0, false);
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
            &mut |_, _| None,
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
            &mut |_, _| None,
        );
        assert_eq!(out.len(), 2, "{out:?}");
        assert_eq!(
            out[0],
            (
                0,
                RowKind::Text {
                    first_in_block: true,
                    continued: false,
                    hyphen: false,
                }
            ),
        );
        assert_eq!(
            out[1],
            (
                6,
                RowKind::Text {
                    first_in_block: false,
                    continued: true,
                    hyphen: false,
                }
            ),
            "the continuation row carries the continuation indent",
        );
    }

    /// Pack at a uniform `avail`, dividing through `table`: a unit starting
    /// at byte `start` may be cut `letters` cells in. Returns `(start, end,
    /// hyphen)` per row.
    fn rows_divided(units: Vec<Unit>, avail: u16, table: &[(u32, u16)]) -> Vec<(u32, u32, bool)> {
        let mut out = Vec::new();
        let mut first = true;
        let mut sink = |r: Row| {
            let RowKind::Text { hyphen, .. } = r.kind else {
                unreachable!()
            };
            out.push((r.doc.start, r.doc.end, hyphen));
        };
        let mut divide = |u: &Unit, room: u16| {
            let letters = table.iter().find(|(s, _)| *s == u.range.start)?.1;
            // A real divider answers for the room it is given; this one has a
            // single answer and declines when it does not fit.
            if letters + HYPHEN_COLS > room {
                return None;
            }
            let at = u.range.start + u32::from(letters);
            Some((
                unit(u.range.start, at, letters, 0, false),
                Unit {
                    range: at..u.range.end,
                    width: u.width - letters,
                    ..u.clone()
                },
            ))
        };
        pack(
            units.into_iter(),
            0,
            BlockIdx(0),
            &uniform(avail),
            &mut first,
            &mut sink,
            &mut divide,
        );
        out
    }

    /// `aaaa bbbbbbbb cc`: a 4-cell word, an 8-cell word, and a last word.
    fn three_words() -> Vec<Unit> {
        vec![
            unit(0, 5, 5, 1, false),
            unit(5, 14, 9, 1, false),
            unit(14, 16, 2, 0, true),
        ]
    }

    #[test]
    fn a_word_that_misses_the_row_end_is_divided_to_fill_it() {
        // avail 10: `aaaa ` leaves 5 cells, `bbbbbbbb` needs 8. Cut 4 in:
        // `aaaa bbbb` + hyphen is exactly 10.
        let rows = rows_divided(three_words(), 10, &[(5, 4)]);
        assert_eq!(rows, vec![(0, 9, true), (9, 16, false)]);
    }

    #[test]
    fn a_divided_row_leaves_no_gap_and_its_hyphen_fits() {
        // The head ends where the tail begins — no elided whitespace — and
        // head + hyphen never exceeds the row.
        let rows = rows_divided(three_words(), 10, &[(5, 4)]);
        assert_eq!(rows[0].1, rows[1].0, "a division is not a gap");
        let head_cells = 5 + 4 + HYPHEN_COLS;
        assert!(head_cells <= 10);
    }

    #[test]
    fn a_head_too_wide_for_the_hole_takes_the_ordinary_break() {
        // Cut 5 in needs 6 cells with its hyphen; the hole is 5.
        let rows = rows_divided(three_words(), 10, &[(5, 5)]);
        assert_eq!(rows, vec![(0, 4, false), (5, 13, false), (14, 16, false)]);
    }

    #[test]
    fn a_divider_that_overfills_the_hole_is_not_believed() {
        // The packer owns the fit. A head wider than the room it was offered
        // is refused rather than painted over the edge.
        let mut out = Vec::new();
        let mut first = true;
        let mut sink = |r: Row| out.push((r.doc.start, r.doc.end));
        let mut greedy = |u: &Unit, _room: u16| {
            let at = u.range.start + 7;
            Some((
                unit(u.range.start, at, 7, 0, false),
                unit(at, u.range.end, 2, 1, false),
            ))
        };
        pack(
            three_words().into_iter(),
            0,
            BlockIdx(0),
            &uniform(10),
            &mut first,
            &mut sink,
            &mut greedy,
        );
        assert_eq!(out, vec![(0, 4), (5, 13), (14, 16)]);
    }

    #[test]
    fn a_small_hole_is_left_ragged() {
        // `aaaaaa ` at avail 10 leaves 3 cells: under the 4-cell floor, so
        // the divider is never even asked.
        let units = vec![
            unit(0, 7, 7, 1, false),
            unit(7, 16, 9, 1, false),
            unit(16, 18, 2, 0, true),
        ];
        let rows = rows_divided(units, 10, &[(7, 2)]);
        assert!(rows.iter().all(|r| !r.2), "{rows:?}");
    }

    #[test]
    fn the_hole_worth_filling_grows_with_the_row() {
        assert_eq!(min_room(10), 4);
        assert_eq!(min_room(32), 4);
        assert_eq!(min_room(40), 5);
        assert_eq!(min_room(64), 8);
    }

    #[test]
    fn the_last_word_of_a_line_is_never_divided() {
        // The mandatory unit is the line's last word; dividing it would
        // strand its tail on a row of its own.
        let units = vec![unit(0, 5, 5, 1, false), unit(5, 13, 8, 0, true)];
        let rows = rows_divided(units, 10, &[(5, 4)]);
        assert_eq!(rows, vec![(0, 4, false), (5, 13, false)]);
    }

    #[test]
    fn no_more_than_two_rows_in_a_row_end_with_a_hyphen() {
        // A 4-cell word, then 10-cell words, at avail 12. Each division
        // leaves a 7-cell tail, so the next word misses by exactly enough to
        // be divided too — a ladder all the way down, left to itself.
        let mut units = vec![unit(0, 5, 5, 1, false)];
        let mut table = Vec::new();
        for i in 0..6u32 {
            let s = 5 + i * 11;
            units.push(unit(s, s + 11, 11, 1, false));
            table.push((s, 3));
        }
        units.push(unit(71, 73, 2, 0, true));
        let rows = rows_divided(units, 12, &table);
        let hyphens: Vec<bool> = rows.iter().map(|r| r.2).collect();
        assert_eq!(&hyphens[..3], [true, true, false], "{hyphens:?}");
        assert!(
            !hyphens.windows(3).any(|w| w == [true, true, true]),
            "a ladder of three: {hyphens:?}"
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
