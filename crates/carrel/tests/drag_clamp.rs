//! A drag that leaves the text still selects up to where it left.
//!
//! The pointer used to map a cell to a byte only when the cell held a
//! character, so a drag released on the blank row under a paragraph, right
//! of a short line, in the margin, or on the status row produced no drag at
//! all and copied nothing — while every terminal and every GUI a reader has
//! used clamps to the nearest position. These tests drive the same two calls
//! the frontend makes, at cells found by scanning what `doc_span_at` maps,
//! so they follow the geometry instead of hard-coding it.

use carrel::action::Action;
use carrel::app::{App, update};
use carrel_core::Document;

const SRC: &str = "# Head\n\nalpha beta gamma\n\nsecond para\n";

fn app() -> App {
    let mut a = App::new("t.md".into(), Document::parse(SRC), 40, 12);
    a.on_resize(40, 12);
    a
}

/// The screen cell painted with the cluster starting at `byte`.
fn cell_of(a: &App, byte: u32) -> (u16, u16) {
    for row in 0..a.rows {
        for col in 0..a.cols {
            if a.doc_span_at(col, row).is_some_and(|(s, _)| s == byte) {
                return (col, row);
            }
        }
    }
    panic!("byte {byte} is not on screen");
}

fn at(a: &App, word: &str) -> u32 {
    u32::try_from(a.doc.text.find(word).unwrap()).unwrap()
}

/// Press `off` bytes into `word`, drag to the cell, release; what reached
/// the clipboard.
fn drag(a: &mut App, word: &str, off: u32, to: (u16, u16)) -> Option<String> {
    let (c, r) = cell_of(a, at(a, word) + off);
    let press = a.doc_span_at(c, r).unwrap();
    update(a, Action::SelectAnchor(press));
    if let Some(span) = a.doc_drag_at(to.0, to.1) {
        update(a, Action::SelectDrag(span));
    }
    update(a, Action::SelectRelease);
    a.clipboard.take().map(|c| c.to_string())
}

#[test]
fn a_release_right_of_a_row_selects_to_its_end() {
    let mut a = app();
    let (_, row) = cell_of(&a, at(&a, "alpha"));
    let col = a.cols - 2;
    assert!(a.doc_span_at(col, row).is_none(), "past the text column");
    let got = drag(&mut a, "alpha", 0, (col, row));
    assert_eq!(got.as_deref().map(str::trim_end), Some("alpha beta gamma"));
}

#[test]
fn a_release_on_the_gap_row_below_selects_to_the_end_of_the_block_above() {
    let mut a = app();
    let (col, row) = cell_of(&a, at(&a, "beta"));
    assert!(
        a.doc_span_at(col, row + 1).is_none(),
        "the gap row is blank"
    );
    let got = drag(&mut a, "alpha", 0, (col, row + 1));
    assert_eq!(got.as_deref().map(str::trim_end), Some("alpha beta gamma"));
}

#[test]
fn a_release_in_the_left_margin_selects_from_that_rows_start() {
    let mut a = app();
    let (_, row) = cell_of(&a, at(&a, "alpha"));
    // Backwards, from the end of the line into the margin beside its start.
    let got = drag(&mut a, "gamma", 4, (0, row));
    assert_eq!(got.as_deref(), Some("alpha beta gamma"));
}

#[test]
fn a_release_on_the_status_row_selects_to_the_last_visible_text() {
    let mut a = app();
    let rows = a.rows;
    let got = drag(&mut a, "alpha", 0, (5, rows - 1)).unwrap();
    assert!(got.starts_with("alpha beta gamma"), "{got:?}");
    assert!(got.trim_end().ends_with("second para"), "{got:?}");
}

#[test]
fn a_release_above_the_text_selects_from_the_first_visible_row() {
    let mut a = app();
    let top = a.text_y();
    assert!(top > 0, "the fixture has a row above the text");
    let got = drag(&mut a, "second", 5, (20, top - 1)).unwrap();
    assert!(got.starts_with("Head"), "{got:?}");
    assert!(got.ends_with("second"), "{got:?}");
}

#[test]
fn a_press_in_the_margin_starts_a_selection_but_one_on_a_blank_row_does_not() {
    let a = app();
    let alpha = at(&a, "alpha");
    let (col, row) = cell_of(&a, alpha);
    assert!(a.doc_span_at(0, row).is_none(), "the margin holds no text");
    assert_eq!(a.doc_row_start_left_of(0, row), Some((alpha, alpha)));
    assert_eq!(a.doc_row_start_left_of(col, row + 1), None, "a gap row");
    assert_eq!(a.doc_row_start_left_of(col + 2, row), None, "on the text");
}
