//! Break units → rows. **Knows nothing about text.**
//!
//! Every invariant the reflow layer promises lives in this file, and every one
//! of them is checkable against units built by hand — which is the whole reason
//! the seam in [`super::units`] exists.

use super::units::{Cut, MARK_COLS, Unit};
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
    fn row(&mut self, start: u32, end: u32, cut: Cut) {
        (self.sink)(Row {
            block: self.block,
            doc: self.doc_base + start..self.doc_base + end.max(start),
            indent: self.indent,
            kind: RowKind::Text {
                first_in_block: *self.first,
                continued: self.continued,
                hyphen: cut == Cut::Hyphen,
                split: cut == Cut::Marked,
            },
        });
        *self.first = false;
        self.rows = self.rows.saturating_add(1);
    }
}

/// Cells a row gives up to the hyphen it ends with.
pub(super) const HYPHEN_COLS: u16 = MARK_COLS;

/// Rows in a row that may end with a hyphen. A third is a "ladder" — the
/// eye loses its place on the way back to the left margin — so the third
/// takes the ragged edge instead.
///
/// Every hyphenated row counts, the author's own soft hyphens and the
/// emergency cut of a word longer than the row included, so the layout
/// never adds a division of its own to a ladder the text already has. Only
/// the layout's optional divisions are ever refused for it: an authored
/// hyphen and an emergency cut are not choices, and refusing them would
/// overflow the row instead. That is why the count saturates rather than
/// stopping at the limit — a run of three hundred authored divisions is
/// three hundred hyphenated rows, and the count must survive them.
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

/// Is `(head, tail, cut)` an honest answer for cutting `u` on an empty row
/// of `avail` cells? The packer owns the fit, so it checks: the halves meet,
/// each holds something — which is the progress guarantee: every believed
/// cut moves the start forward, so the loop asking for them ends — and a
/// marked head fits its row with its mark.
///
/// An unmarked head wider than the row is believed: it is the splitter's
/// answer for a first cluster wider than the whole row (`日` at width 1),
/// which goes on a row ALONE and overhangs. Refusing it placed the whole
/// unit instead, so the cluster after it overhung too. A marked head never
/// overhangs: a mark the row has no cell for is not a mark.
fn believable(u: &Unit, head: &Unit, tail: &Unit, cut: Cut, avail: u16) -> bool {
    let fits = cut == Cut::Plain
        || head.content_width().saturating_add(u32::from(MARK_COLS)) <= u32::from(avail);
    head.range.start == u.range.start
        && head.range.end == tail.range.start
        && u.range.start < head.range.end
        && head.range.end < u.content_end()
        && fits
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
///
/// # A unit wider than the row
///
/// `split(unit, avail)` is asked when a unit does not fit even an EMPTY row
/// — the emergency break — for a head that fills the row, the tail that
/// follows, and how the row is marked ([`Cut`]). It is asked again for the
/// tail until what is left fits. `None` places the unit whole: a single
/// cluster wider than the row overhangs rather than vanishing. An answer
/// that does not fit with its mark, or does not make progress, is not
/// believed, and the unit is placed whole the same way.
#[allow(clippy::too_many_arguments)]
pub(super) fn pack<I, F, D, S>(
    units: I,
    doc_base: u32,
    block: BlockIdx,
    fit: &LineFit,
    first: &mut bool,
    sink: &mut F,
    divide: &mut D,
    split: &mut S,
) -> u32
where
    I: Iterator<Item = Unit>,
    F: FnMut(Row),
    D: FnMut(&Unit, u16) -> Option<(Unit, Unit)>,
    S: FnMut(&Unit, u16) -> Option<(Unit, Unit, Cut)>,
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

    // Cells, in the units' own `u32`: a run with no break in it can be
    // wider than any row type counts.
    let mut col = 0u32;
    let mut row_start: Option<u32> = None;
    let mut content_end = 0u32;
    // Consecutive rows ended with a hyphen so far.
    let mut ladder = 0u8;
    // The last unit placed ends in the author's soft hyphen, so a row that
    // ends after it shows a hyphen.
    let mut soft = false;

    for mut u in units {
        // The fit test uses CONTENT width. Whitespace that a break elides is
        // allowed to overhang, which is what UAX #14 and every browser do;
        // counting it wraps a column early.
        if col > 0 && col.saturating_add(u.end_width()) > u32::from(avail) {
            let s = row_start.take().unwrap_or(u.range.start);
            // `col` counts the space after the previous word, so `room` is
            // exactly what a head and its hyphen may occupy. The line's last
            // word is never divided: its tail would be a row of its own.
            let room = u16::try_from(u32::from(avail).saturating_sub(col)).unwrap_or(0);
            let divided = if ladder < MAX_LADDER && !u.mandatory && room >= min_room(avail) {
                divide(&u, room).filter(|(head, _)| {
                    head.content_width().saturating_add(u32::from(HYPHEN_COLS)) <= u32::from(room)
                })
            } else {
                None
            };
            if let Some((head, tail)) = divided {
                e.row(s, head.range.end, Cut::Hyphen);
                ladder = ladder.saturating_add(1);
                u = tail;
            } else if soft && col < u32::from(avail) {
                // The author's division, and the cell its fit test kept.
                // Never refused for the ladder; see `MAX_LADDER`.
                e.row(s, content_end, Cut::Hyphen);
                ladder = ladder.saturating_add(1);
            } else {
                e.row(s, content_end, Cut::Plain);
                ladder = 0;
            }
            col = 0;
            // Every row after the first belongs to the same logical line.
            e.continued = true;
            e.indent = fit.cont_indent;
            avail = fit.cont_avail;
        }
        // The emergency break: a unit that does not fit an empty row is cut
        // until what is left does. Only on an empty row — a wide unit after
        // other words took the ordinary break above first.
        while col == 0 && u.end_width() > u32::from(avail) {
            let Some((head, tail, cut)) = split(&u, avail)
                .filter(|(head, tail, cut)| believable(&u, head, tail, *cut, avail))
            else {
                break;
            };
            let s = row_start.take().unwrap_or(head.range.start);
            e.row(s, head.range.end, cut);
            ladder = if cut == Cut::Hyphen {
                ladder.saturating_add(1)
            } else {
                0
            };
            e.continued = true;
            e.indent = fit.cont_indent;
            avail = fit.cont_avail;
            u = tail;
        }
        if row_start.is_none() {
            row_start = Some(u.range.start);
        }
        // Mid-row whitespace is real and painted, so `col` counts the full width.
        col = col.saturating_add(u.width);
        content_end = u.content_end();
        soft = u.soft_hyphen;

        if u.mandatory {
            let s = row_start.take().unwrap_or(u.range.start);
            e.row(s, content_end, Cut::Plain);
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
        e.row(s, content_end, Cut::Plain);
    }
    // An empty block is one empty row, not zero rows. Explicit, rather than a
    // `.max(1)` clamp that would also paper over a genuine loss of rows.
    if e.rows == 0 {
        e.row(0, 0, Cut::Plain);
    }
    e.rows
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit with no text behind it. The point of the seam.
    fn unit(start: u32, end: u32, width: u32, trailing_ws: u32, mandatory: bool) -> Unit {
        Unit {
            range: start..end,
            width,
            trailing_ws,
            trailing_ws_bytes: trailing_ws,
            mandatory,
            soft_hyphen: false,
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
                    split: false,
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
                    split: false,
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
                unit(u.range.start, at, u32::from(letters), 0, false),
                Unit {
                    range: at..u.range.end,
                    width: u.width - u32::from(letters),
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
            &mut |_, _| None,
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
            &mut |_, _| None,
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

    /// `unit`, ending in the author's soft hyphen.
    fn soft(start: u32, end: u32, width: u32) -> Unit {
        Unit {
            soft_hyphen: true,
            ..unit(start, end, width, 0, false)
        }
    }

    #[test]
    fn authored_hyphens_count_toward_the_ladder() {
        // Two rows end at the author's soft hyphen; the third would end in a
        // division of the layout's own — `cccc ` leaves 4 cells, and `ddd`
        // and a hyphen fill them — making a ladder of three. Refused.
        let units = vec![
            soft(0, 7, 6),
            soft(7, 14, 6),
            unit(14, 20, 6, 1, false),
            unit(20, 30, 10, 1, false),
            unit(30, 32, 2, 0, true),
        ];
        let rows = rows_divided(units, 10, &[(20, 3)]);
        let hyphens: Vec<bool> = rows.iter().map(|r| r.2).collect();
        assert_eq!(&hyphens[..3], [true, true, false], "{rows:?}");
    }

    #[test]
    fn an_authored_ladder_of_any_length_is_kept_and_counted_without_overflow() {
        // Three hundred soft-hyphen units, each filling a row: every row
        // shows the author's hyphen, and the count of them must not wrap.
        let units: Vec<Unit> = (0..300u32).map(|i| soft(i * 9, i * 9 + 9, 8)).collect();
        let rows = rows_divided(units, 10, &[]);
        assert_eq!(rows.len(), 300);
        assert!(rows[..299].iter().all(|r| r.2));
    }

    #[test]
    fn a_ladder_of_emergency_hyphens_of_any_length_does_not_overflow() {
        // A 3,000-cell word cut at every row, each cut a hyphen.
        let units = vec![unit(0, 3000, 3000, 0, true)];
        let rows = rows_cut(units, 10, |u, avail| {
            Some(cut_at(u, avail - 1, Cut::Hyphen))
        });
        assert_eq!(rows.len(), 334);
    }

    #[test]
    fn an_unmarked_head_wider_than_the_row_is_believed_and_goes_alone() {
        // The splitter's answer for a first cluster wider than the row: a
        // 2-cell head at avail 1. It goes on a row alone; refusing it put the
        // whole unit on one row, the clusters after it overhanging too.
        let units = vec![unit(0, 6, 6, 0, true)];
        let rows = rows_cut(units, 1, |u, _| {
            (u.width > 2).then(|| cut_at(u, 2, Cut::Plain))
        });
        assert_eq!(
            rows,
            [
                (0, 2, false, false),
                (2, 4, false, false),
                (4, 6, false, false)
            ]
        );
        // Marked, the same head is refused: there is no cell for its mark.
        let units = vec![unit(0, 6, 6, 0, true)];
        let rows = rows_cut(units, 1, |u, _| Some(cut_at(u, 2, Cut::Marked)));
        assert_eq!(rows, [(0, 6, false, false)]);
    }

    #[test]
    fn a_run_wider_than_a_u16_is_cut_until_every_row_fits() {
        // 100,000 cells with no break: every row is 20 cells.
        let units = vec![unit(0, 100_000, 100_000, 0, true)];
        let rows = rows_cut(units, 20, |u, avail| Some(cut_at(u, avail, Cut::Plain)));
        assert_eq!(rows.len(), 5000);
        assert!(rows.iter().all(|r| r.1 - r.0 == 20), "{:?}", rows.last());
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

    /// Pack at a uniform `avail` with `split` answering for units wider than
    /// a row. Returns `(start, end, hyphen, split)` per row.
    fn rows_cut(
        units: Vec<Unit>,
        avail: u16,
        mut split: impl FnMut(&Unit, u16) -> Option<(Unit, Unit, Cut)>,
    ) -> Vec<(u32, u32, bool, bool)> {
        let mut out = Vec::new();
        let mut first = true;
        let mut sink = |r: Row| {
            let RowKind::Text { hyphen, split, .. } = r.kind else {
                unreachable!()
            };
            out.push((r.doc.start, r.doc.end, hyphen, split));
        };
        pack(
            units.into_iter(),
            0,
            BlockIdx(0),
            &uniform(avail),
            &mut first,
            &mut sink,
            &mut |_, _| None,
            &mut split,
        );
        out
    }

    /// `u` cut `cells` in: a byte is a cell in these hand-built units.
    fn cut_at(u: &Unit, cells: u16, cut: Cut) -> (Unit, Unit, Cut) {
        let at = u.range.start + u32::from(cells);
        (
            unit(u.range.start, at, u32::from(cells), 0, false),
            Unit {
                range: at..u.range.end,
                width: u.width - u32::from(cells),
                ..u.clone()
            },
            cut,
        )
    }

    /// `aaa ` and then a 25-cell run with no break in it.
    fn long_run() -> Vec<Unit> {
        vec![unit(0, 4, 4, 1, false), unit(4, 29, 25, 0, true)]
    }

    #[test]
    fn a_unit_wider_than_the_row_is_cut_until_what_is_left_fits() {
        // At avail 10 each cut leaves a cell for its mark: 9 + 9 + 7.
        let rows = rows_cut(long_run(), 10, |u, avail| {
            Some(cut_at(u, avail - 1, Cut::Marked))
        });
        assert_eq!(
            rows,
            [
                (0, 3, false, false),
                (4, 13, false, true),
                (13, 22, false, true),
                (22, 29, false, false),
            ]
        );
    }

    #[test]
    fn a_cut_is_asked_only_of_a_unit_that_misses_an_empty_row() {
        let mut asked = Vec::new();
        let rows = rows_cut(long_run(), 30, |u, avail| {
            asked.push((u.range.start, avail));
            None
        });
        assert_eq!(rows, [(0, 29, false, false)]);
        assert!(asked.is_empty(), "{asked:?}");
    }

    #[test]
    fn a_cut_that_leaves_no_room_for_its_mark_is_not_believed() {
        // The packer owns the fit: a full-row head cannot also carry a mark.
        // Refused, the unit is placed whole and overhangs — nothing dropped.
        let rows = rows_cut(long_run(), 10, |u, avail| {
            Some(cut_at(u, avail, Cut::Marked))
        });
        assert_eq!(rows, [(0, 3, false, false), (4, 29, false, false)]);
        // The same head unmarked fits exactly, and is taken.
        let rows = rows_cut(long_run(), 10, |u, avail| {
            Some(cut_at(u, avail, Cut::Plain))
        });
        assert_eq!(rows[1], (4, 14, false, false));
    }

    #[test]
    fn a_cut_that_makes_no_progress_is_not_believed() {
        // An empty head would ask again forever; a head that is the whole
        // unit leaves nothing to continue with.
        for cells in [0, 25] {
            let rows = rows_cut(long_run(), 10, |u, _| Some(cut_at(u, cells, Cut::Plain)));
            assert_eq!(
                rows,
                [(0, 3, false, false), (4, 29, false, false)],
                "{cells}"
            );
        }
    }

    #[test]
    fn a_hyphenated_cut_is_marked_as_a_hyphen() {
        let rows = rows_cut(long_run(), 10, |u, avail| {
            Some(cut_at(u, avail - 1, Cut::Hyphen))
        });
        assert_eq!(rows[1], (4, 13, true, false));
        assert_eq!(rows[2], (13, 22, true, false));
    }
}
