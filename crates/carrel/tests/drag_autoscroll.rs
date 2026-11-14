//! A selection drag held above or below the text scrolls the document.
//!
//! Crossterm reports a drag only when the pointer moves, so a hand held
//! still on the status row used to leave the selection at the last visible
//! row: nothing longer than a screen could be selected at all. The event
//! loop now sends `Action::DragScroll` on a clock while the pointer is out
//! there; these tests drive that action directly, the way the auto-read
//! tests drive `AutoTick`.

use std::fmt::Write as _;

use carrel::action::Action;
use carrel::app::{App, Outcome, update};
use carrel_core::Document;

/// Forty one-row paragraphs, each with a gap row: far taller than 12 rows.
fn src() -> String {
    let mut s = String::new();
    for i in 0..40 {
        let _ = write!(s, "para{i:02} words\n\n");
    }
    s
}

fn app_from(src: &str) -> App {
    let mut a = App::new("t.md".into(), Document::parse(src), 40, 12);
    a.on_resize(40, 12);
    a
}

fn at(a: &App, word: &str) -> u32 {
    u32::try_from(a.doc.text.find(word).unwrap()).unwrap()
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

/// Press on `word`, then drag to `to` — the two calls the frontend makes.
fn press_and_drag(a: &mut App, word: &str, to: (u16, u16)) {
    let (c, r) = cell_of(a, at(a, word));
    let press = a.doc_span_at(c, r).unwrap();
    update(a, Action::SelectAnchor(press));
    let span = a.doc_drag_at(to.0, to.1).unwrap();
    update(a, Action::SelectDrag(span));
}

fn status_row(a: &App) -> u16 {
    a.text_y() + a.text_h()
}

#[test]
fn a_drag_held_below_scrolls_a_row_a_tick_and_selects_to_the_last_row() {
    let mut a = app_from(&src());
    let below = (5, status_row(&a));
    assert_eq!(a.drag_edge(below.1), 1, "the status row is one row out");
    press_and_drag(&mut a, "para00", below);
    let before = a.view.scroll_row;
    assert!(
        (0..a.rows).all(|r| (0..a.cols).all(|c| a
            .doc_span_at(c, r)
            .is_none_or(|(s, _)| s != at(&a, "para08")))),
        "fixture: para08 starts off screen"
    );
    for _ in 0..11 {
        assert_eq!(update(&mut a, Action::DragScroll(below)), Outcome::Redraw);
    }
    assert_eq!(a.view.scroll_row, before + 11, "one row per tick");
    let end = a.doc_drag_at(below.0, below.1).unwrap().1;
    assert_eq!(
        a.selection,
        Some(at(&a, "para00")..end),
        "extended to the end of the last visible row"
    );
    update(&mut a, Action::SelectRelease);
    let copied = a.clipboard.take().unwrap().to_string();
    assert!(copied.starts_with("para00"), "{copied:?}");
    assert!(
        copied.contains("para08"),
        "text that was off screen at the press: {copied:?}"
    );
}

#[test]
fn a_drag_held_above_scrolls_up_and_selects_from_the_first_row() {
    let mut a = app_from(&src());
    update(&mut a, Action::ScrollTo(30));
    let above = (5, 0);
    assert_eq!(a.drag_edge(0), -1, "row 0 is the top margin, one row out");
    let (_, r) = cell_of(&a, at(&a, "para18"));
    assert!(
        r > a.text_y(),
        "fixture: para18 is on screen, below the top"
    );
    press_and_drag(&mut a, "para18", above);
    for _ in 0..5 {
        update(&mut a, Action::DragScroll(above));
    }
    assert_eq!(a.view.scroll_row, 25, "five rows up");
    let start = a.doc_drag_at(above.0, above.1).unwrap().0;
    assert_eq!(
        a.selection.as_ref().map(|s| s.start),
        Some(start),
        "extended to the start of the first visible row"
    );
    assert!(start < at(&a, "para15"), "reached text above the press");
}

#[test]
fn further_out_scrolls_faster() {
    let mut a = app_from(&src());
    let far = (5, status_row(&a) + 1);
    assert!(
        far.1 < a.rows,
        "fixture: the footer row is below the status row"
    );
    assert_eq!(a.drag_edge(far.1), 2);
    press_and_drag(&mut a, "para00", far);
    update(&mut a, Action::DragScroll(far));
    assert_eq!(a.view.scroll_row, 2, "two rows out, two rows a tick");
}

#[test]
fn a_tick_inside_the_text_or_after_release_does_nothing() {
    let mut a = app_from(&src());
    let below = (5, status_row(&a));
    press_and_drag(&mut a, "para00", below);
    let inside = (5, a.text_y() + 2);
    assert_eq!(a.drag_edge(inside.1), 0);
    assert_eq!(update(&mut a, Action::DragScroll(inside)), Outcome::Idle);
    assert_eq!(a.view.scroll_row, 0);
    update(&mut a, Action::SelectRelease);
    let copied = a.selection.clone();
    assert_eq!(update(&mut a, Action::DragScroll(below)), Outcome::Idle);
    assert_eq!(a.view.scroll_row, 0, "no drag, no scroll");
    assert_eq!(a.selection, copied, "and the copied selection stands");
}

#[test]
fn esc_mid_drag_stops_it() {
    let mut a = app_from(&src());
    let below = (5, status_row(&a));
    press_and_drag(&mut a, "para00", below);
    update(&mut a, Action::Dismiss);
    assert_eq!(update(&mut a, Action::DragScroll(below)), Outcome::Idle);
    assert_eq!(a.view.scroll_row, 0);
}

#[test]
fn a_pane_opened_mid_drag_stops_it() {
    let mut a = app_from(&src());
    let below = (5, status_row(&a));
    press_and_drag(&mut a, "para00", below);
    update(&mut a, Action::HelpToggle);
    assert!(a.help.is_some());
    assert_eq!(update(&mut a, Action::DragScroll(below)), Outcome::Idle);
    assert_eq!(a.view.scroll_row, 0);
}

#[test]
fn it_stops_at_the_end_of_the_document() {
    let mut a = app_from(&src());
    let max = a.layout.max_scroll(a.text_h());
    update(&mut a, Action::ScrollTo(max));
    assert_eq!(a.view.scroll_row, max);
    let below = (5, status_row(&a));
    press_and_drag(&mut a, "para38", below);
    let sel = a.selection.clone();
    assert_eq!(update(&mut a, Action::DragScroll(below)), Outcome::Idle);
    assert_eq!(a.view.scroll_row, max, "never past the end");
    assert_eq!(a.selection, sel);
}

#[test]
fn it_stops_at_the_top_of_the_document() {
    let mut a = app_from(&src());
    press_and_drag(&mut a, "para03", (5, 0));
    assert_eq!(update(&mut a, Action::DragScroll((5, 0))), Outcome::Idle);
    assert_eq!(a.view.scroll_row, 0);
}

#[test]
fn autoscroll_never_changes_folds() {
    // A folded section below the screen stays folded as the drag scrolls
    // past it; a scroll is not a jump, so nothing unfolds its way there.
    let mut s = String::from("# Top\n\n");
    s.push_str(&src());
    s.push_str("# Folded\n\nhidden words\n\n# After\n\n");
    s.push_str(&src().replace("para", "late"));
    let mut a = app_from(&s);
    let heading = at(&a, "Folded");
    update(&mut a, Action::FoldAt(heading));
    assert_eq!(a.folded.len(), 1);
    let folded = a.folded.clone();
    let below = (5, status_row(&a));
    press_and_drag(&mut a, "para00", below);
    for _ in 0..120 {
        update(&mut a, Action::DragScroll(below));
    }
    assert_eq!(a.folded, folded);
    assert!(a.view.scroll_row > 80, "it did scroll past the fold");
}
