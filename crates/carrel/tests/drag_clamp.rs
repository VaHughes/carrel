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
fn a_press_in_the_margin_starts_a_selection_at_that_rows_start() {
    let a = app();
    let alpha = at(&a, "alpha");
    let (_, row) = cell_of(&a, alpha);
    assert!(a.doc_span_at(0, row).is_none(), "the margin holds no text");
    assert_eq!(a.doc_press_at(0, row), Some((alpha, alpha)));
}

/// Press at a cell with no character under it, drag to the cluster that
/// starts at `to`, release; what reached the clipboard.
fn drag_from(a: &mut App, from: (u16, u16), to: u32) -> Option<String> {
    let press = a
        .doc_press_at(from.0, from.1)
        .expect("the press starts a selection");
    update(a, Action::SelectAnchor(press));
    let (c, r) = cell_of(a, to);
    update(a, Action::SelectDrag(a.doc_span_at(c, r).unwrap()));
    update(a, Action::SelectRelease);
    a.clipboard.take().map(|c| c.to_string())
}

/// The gap row between two paragraphs is the boundary between them: a drag
/// from it takes the text it moves into, in either direction, and none of
/// the blank between.
#[test]
fn a_drag_from_a_gap_row_selects_toward_the_pointer_from_the_boundary() {
    let mut a = app();
    let (col, row) = cell_of(&a, at(&a, "beta"));
    let gap = (col, row + 1);
    assert!(
        a.doc_span_at(gap.0, gap.1).is_none(),
        "the gap row is blank"
    );
    let last = at(&a, "para") + 3;
    assert_eq!(drag_from(&mut a, gap, last).as_deref(), Some("second para"));
    let beta = at(&a, "beta");
    assert_eq!(drag_from(&mut a, gap, beta).as_deref(), Some("beta gamma"));
}

#[test]
fn a_drag_from_past_the_end_of_a_line_starts_at_that_lines_end() {
    let mut a = app();
    let (_, row) = cell_of(&a, at(&a, "alpha"));
    let right = (a.cols - 2, row);
    assert!(
        a.doc_span_at(right.0, right.1).is_none(),
        "past the text column"
    );
    let beta = at(&a, "beta");
    assert_eq!(
        drag_from(&mut a, right, beta).as_deref(),
        Some("beta gamma")
    );
    let second = at(&a, "second") + 5;
    let got = drag_from(&mut a, right, second).unwrap();
    assert!(got.trim_start().starts_with("second"), "{got:?}");
    assert!(!got.contains("gamma"), "{got:?}");
}

/// A click on a blank gap row is not a click on the heading beside it —
/// above it or below. The gap's anchor names the NEXT block's start, so a
/// fold check that read it as a press there collapsed the heading under it.
#[test]
fn a_click_on_a_gap_row_beside_a_heading_folds_nothing() {
    let src = "# Head\n\npara one\n\n# Next\n\nbody\n";
    let mut a = App::new("t.md".into(), Document::parse(src), 40, 12);
    a.on_resize(40, 12);
    for word in ["Head", "para"] {
        let (col, row) = cell_of(&a, at(&a, word));
        let press = a.doc_press_at(col, row + 1).expect("the gap row anchors");
        update(&mut a, Action::SelectAnchor(press));
        update(&mut a, Action::SelectRelease);
        assert!(a.folded.is_empty(), "nothing folded under {word}");
    }
}

/// Chrome never starts a selection: the status row, the scrollbar column,
/// and the empty rows below the end of the document.
#[test]
fn a_press_on_chrome_or_below_the_document_starts_nothing() {
    let a = app();
    let (col, row) = cell_of(&a, at(&a, "second"));
    assert_eq!(a.doc_press_at(col, a.rows - 1), None, "the status row");
    assert_eq!(a.doc_press_at(a.cols - 1, row), None, "the scrollbar");
    assert_eq!(a.doc_press_at(col, row + 3), None, "below the document");
    assert_eq!(a.doc_press_at(col, a.text_y() - 1), None, "above the text");
}
