//! The footnote peek: a footnote's text where its mark is.

use carrel::action::{Action, Span, Zone};
use carrel::app::{App, Outcome, update};
use carrel_core::Document;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use std::fmt::Write as _;

fn src() -> String {
    let mut s =
        String::from("# Notes\n\nA claim that needs support[^why], and another[^missing].\n\n");
    for i in 0..40 {
        let _ = write!(s, "filler paragraph {i}\n\n");
    }
    s.push_str("[^why]: Because the measurement said so, twice, on two machines.\n");
    s
}

fn reader(cols: u16, rows: u16) -> App {
    let mut app = App::new("n.md".into(), Document::parse(&src()), cols, rows);
    app.file = Some("/w/n.md".into());
    app.on_resize(cols, rows);
    app
}

fn painted(app: &App, cols: u16, rows: u16) -> (ratatui::buffer::Buffer, Vec<(Action, Zone, u8)>) {
    let mut term = Terminal::new(TestBackend::new(cols, rows)).unwrap();
    let mut p = carrel::render::Painted::default();
    term.draw(|f| {
        carrel::render::draw_full(f, app, &mut p, &mut std::collections::HashMap::new());
    })
    .unwrap();
    let targets = p
        .targets
        .as_slice()
        .iter()
        .map(|t| (t.action, t.zone, t.z))
        .collect();
    (term.backend().buffer().clone(), targets)
}

fn under(buf: &ratatui::buffer::Buffer, z: Zone) -> String {
    (z.x..z.x + z.w).map(|x| buf[(x, z.y)].symbol()).collect()
}

fn marks(targets: &[(Action, Zone, u8)]) -> Vec<(Action, Zone)> {
    targets
        .iter()
        .filter(|(a, _, _)| matches!(a, Action::FootnotePeek { .. }))
        .map(|(a, z, _)| (*a, *z))
        .collect()
}

#[test]
fn every_footnote_mark_on_screen_is_a_button_on_its_own_text() {
    let app = reader(80, 24);
    let (buf, targets) = painted(&app, 80, 24);
    let found: Vec<String> = marks(&targets)
        .iter()
        .map(|(_, z)| under(&buf, *z))
        .collect();
    // `[^missing]` has no footnote. It reads like a mark — and so does the
    // `[^a-z]` in a sentence about regular expressions — so only a mark with
    // something to show is a button.
    assert_eq!(found, ["[^why]"]);
}

#[test]
fn clicking_a_mark_shows_the_footnote_where_the_mark_is() {
    let mut app = reader(80, 24);
    let (_, targets) = painted(&app, 80, 24);
    let (click, zone) = marks(&targets)[0];
    assert_eq!(update(&mut app, click), Outcome::Redraw);
    let peek = app.peek.clone().expect("the peek is open");
    assert_eq!(peek.label, "[^why]");
    assert!(
        peek.text.starts_with("Because the measurement"),
        "{:?}",
        peek.text
    );
    assert_eq!(app.view.scroll_row, 0, "and the reader has not moved");

    let (buf, targets) = painted(&app, 80, 24);
    let boxed = peek.zone(80, 24);
    assert_eq!(boxed.y, zone.y + 1, "the box hangs under the mark");
    let text: String = (boxed.y..boxed.y + boxed.h)
        .map(|y| {
            (boxed.x..boxed.x + boxed.w)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("Because the measurement said so"), "{text}");
    // Its button covers its own label, above the document's own targets.
    let (_, go, z) = targets
        .iter()
        .find(|(a, _, _)| *a == Action::PeekGo)
        .copied()
        .expect("the way to the footnote itself");
    assert_eq!(under(&buf, go), "[ go to it ]");
    // And the box takes every click inside it: nothing under it is hit.
    let mut t = carrel::action::Targets::new();
    for (a, zn, zz) in &targets {
        t.push(*a, *zn, *zz);
    }
    assert_eq!(
        t.hit(boxed.x + 1, boxed.y + 1).map(|h| h.action),
        Some(Action::Absorb)
    );
    assert!(z > 0);
}

#[test]
fn whatever_comes_next_closes_the_peek_and_still_happens() {
    let mut app = reader(80, 24);
    let (_, targets) = painted(&app, 80, 24);
    update(&mut app, marks(&targets)[0].0);
    update(&mut app, Action::Scroll(Span::Line, 3));
    assert!(app.peek.is_none(), "closed");
    assert_eq!(app.view.scroll_row, 3, "and the scroll went through");

    // Esc closes it and does nothing else; the pointer moving does not
    // close it at all.
    update(&mut app, Action::Scroll(Span::Line, -3));
    let (_, targets) = painted(&app, 80, 24);
    let click = marks(&targets)[0].0;
    update(&mut app, click);
    update(&mut app, Action::Hover((1, 1)));
    assert!(
        app.peek.is_some(),
        "moving the pointer is not doing something"
    );
    app.selection = Some(0..3);
    update(&mut app, Action::Dismiss);
    assert!(app.peek.is_none());
    assert!(
        app.selection.is_some(),
        "Esc closed the peek, not the selection"
    );
}

#[test]
fn going_to_the_footnote_is_the_long_way_and_back_returns() {
    let mut app = reader(80, 24);
    let (_, targets) = painted(&app, 80, 24);
    update(&mut app, marks(&targets)[0].0);
    let def = app.peek.as_ref().unwrap().def;
    update(&mut app, Action::PeekGo);
    assert!(app.peek.is_none());
    assert!(app.view.scroll_row > 0, "the view went to the footnote");
    // The footnote is the document's last block, so it cannot reach the
    // top row — but it is on screen.
    let block = app.doc.block_at_doc(carrel_core::DocByte(def));
    let row = app.layout.row_start(block);
    assert!(
        (app.view.scroll_row..app.view.scroll_row + u32::from(app.text_h())).contains(&row),
        "the footnote's own row {row} is in view"
    );
    update(&mut app, Action::Back);
    assert_eq!(app.view.scroll_row, 0, "Back is where the mark was read");
}

#[test]
fn a_mark_with_no_footnote_says_so_instead_of_opening_an_empty_box() {
    // Not reachable by a click any more — the mark is not a button — but a
    // target from an older frame can still name one.
    let mut app = reader(80, 24);
    let byte = u32::try_from(app.doc.text.find("[^missing]").unwrap()).unwrap();
    update(&mut app, Action::FootnotePeek { at: (4, 4), byte });
    assert!(app.peek.is_none());
    assert!(app.note.as_deref().unwrap().contains("[^missing]"));
    // A target from a frame the document has outlived is inert.
    assert_eq!(
        update(
            &mut app,
            Action::FootnotePeek {
                at: (0, 0),
                byte: 9_999_999
            }
        ),
        Outcome::Idle
    );
}

/// A click is a press and a release, and the release arrives as an action of
/// its own. The press opens the peek; the release of that same click must
/// not be "the next thing the reader did".
#[test]
fn the_release_of_the_click_that_opened_it_does_not_close_it() {
    let mut app = reader(80, 24);
    let (_, targets) = painted(&app, 80, 24);
    update(&mut app, marks(&targets)[0].0);
    update(&mut app, Action::SelectRelease);
    assert!(
        app.peek.is_some(),
        "the mouse button coming up closed the peek"
    );
    // A second, separate click anywhere on the text does close it.
    update(&mut app, Action::SelectAnchor((0, 1)));
    assert!(app.peek.is_none());
}
