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
        app.doc.text[peek.span.start as usize..].starts_with("Because the measurement"),
        "{:?}",
        peek.span
    );
    assert_eq!(app.view.scroll_row, 0, "and the reader has not moved");

    let (buf, targets) = painted(&app, 80, 24);
    let boxed = peek.zone(&app.doc, 80, 24);
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

// --- the whole footnote, not its first paragraph ---

/// A footnote with one of everything a footnote can hold, under a mark at
/// the top of the page.
fn rich() -> String {
    let mut s = String::from("# Notes\n\nA claim[^why] to support.\n\n");
    for i in 0..30 {
        let _ = write!(s, "filler paragraph {i}\n\n");
    }
    s.push_str(
        "[^why]: Because the measurement said so.\n\n    \
         It was measured again[^again], see [the top](#notes).\n\n    \
         - first run\n    - second run\n\n    \
         > quoted remark\n\n    \
         ```\n    let x = 1;\n    ```\n\n    \
         | col a | col b |\n    |---|---|\n    | one | two |\n\n    \
         ![a missing figure](missing.png)\n\n    \
         $$x^2$$\n\n    \
         ```mermaid\n    graph TD; A-->B\n    ```\n\n    \
         The last word.\n\n\
         [^again]: Again text.\n",
    );
    s
}

fn rich_reader(cols: u16, rows: u16) -> App {
    let mut app = App::new("r.md".into(), Document::parse(&rich()), cols, rows);
    app.file = Some("/w/r.md".into());
    app.on_resize(cols, rows);
    app
}

/// Open the peek on the first mark on screen.
fn open_first(app: &mut App, cols: u16, rows: u16) {
    let (_, targets) = painted(app, cols, rows);
    update(app, marks(&targets)[0].0);
    assert!(app.peek.is_some(), "the peek is open");
}

/// Each footnote row the box shows is painted from its own slice of
/// `Document::text`, at the column the slice says, and nothing that is not
/// in the text is painted to its right. Returns the slices, top to bottom.
fn check_rows(app: &App, cols: u16, rows: u16) -> Vec<String> {
    let (buf, _) = painted(app, cols, rows);
    let peek = app.peek.as_ref().unwrap();
    let z = peek.zone(&app.doc, cols, rows);
    let view = peek.view(&app.doc, cols, rows);
    let inset = peek.inset(&app.doc);
    let (left, right) = (z.x + 2, z.x + z.w - 2);
    let mut out = Vec::new();
    for (dy, line) in view.lines.iter().enumerate() {
        let y = z.y + 1 + u16::try_from(dy).unwrap();
        let Some(row) = &line.row else {
            let blank: String = (left..right).map(|x| buf[(x, y)].symbol()).collect();
            assert!(blank.trim().is_empty(), "a gap row holds {blank:?}");
            out.push(String::new());
            continue;
        };
        let slice = app.doc.text[row.doc.start as usize..row.doc.end as usize].replace('\t', " ");
        let x0 = left + row.indent - inset;
        let mut x = x0;
        let mut got = String::new();
        while x < right && carrel_core::display_width(&got) < carrel_core::display_width(&slice) {
            got.push_str(buf[(x, y)].symbol());
            x += carrel_core::cluster_width(buf[(x, y)].symbol()).max(1);
        }
        assert_eq!(got, slice, "row {dy} painted something else");
        let rest: String = (x..right).map(|x| buf[(x, y)].symbol()).collect();
        assert!(
            rest.trim().is_empty(),
            "row {dy} has {rest:?} after its text"
        );
        out.push(slice);
    }
    out
}

#[test]
fn the_box_shows_every_block_of_the_footnote_and_scrolls_to_the_rest() {
    let (cols, rows) = (80, 24);
    let mut app = rich_reader(cols, rows);
    open_first(&mut app, cols, rows);
    let mut seen = Vec::new();
    for _ in 0..60 {
        seen.extend(check_rows(&app, cols, rows));
        let view = app.peek.as_ref().unwrap().view(&app.doc, cols, rows);
        if view.below == 0 {
            break;
        }
        assert_eq!(update(&mut app, Action::PeekScroll(1)), Outcome::Redraw);
        assert!(app.peek.is_some(), "the wheel does not close it");
    }
    let all = seen.join("\n");
    for part in [
        "Because the measurement said so.",
        "It was measured again[^again], see the top.",
        "first run",
        "second run",
        "quoted remark",
        "let x = 1;",
        "col a",
        "a missing figure",
        "graph TD; A-->B",
        "The last word.",
    ] {
        assert!(all.contains(part), "{part:?} never shown in\n{all}");
    }
    assert!(
        !all.contains("Again text"),
        "the next footnote is not this one"
    );
    assert!(!all.contains("filler"), "the page is not the footnote");
}

/// The box says what it is not showing, and does not say it when it shows
/// everything.
#[test]
fn a_footnote_taller_than_its_box_says_how_much_more_there_is() {
    let (cols, rows) = (80, 24);
    let mut app = rich_reader(cols, rows);
    open_first(&mut app, cols, rows);
    let (buf, _) = painted(&app, cols, rows);
    let z = app.peek.as_ref().unwrap().zone(&app.doc, cols, rows);
    let button_row: String = (z.x..z.x + z.w)
        .map(|x| buf[(x, z.y + z.h - 2)].symbol())
        .collect();
    assert!(button_row.contains("more"), "{button_row:?}");
    assert!(button_row.contains("[ go to it ]"), "{button_row:?}");

    let mut short = reader(80, 24);
    open_first(&mut short, 80, 24);
    let (buf, _) = painted(&short, 80, 24);
    let z = short.peek.as_ref().unwrap().zone(&short.doc, 80, 24);
    let button_row: String = (z.x..z.x + z.w)
        .map(|x| buf[(x, z.y + z.h - 2)].symbol())
        .collect();
    assert!(!button_row.contains("more"), "{button_row:?}");
}

/// The link and the footnote mark inside the box are buttons on the box's
/// own layer — and acting on one closes the box and does it.
#[test]
fn a_link_and_a_mark_inside_the_box_are_clickable() {
    let (cols, rows) = (80, 24);
    let mut app = rich_reader(cols, rows);
    open_first(&mut app, cols, rows);
    let (buf, targets) = painted(&app, cols, rows);
    let mut t = carrel::action::Targets::new();
    for (a, z, zz) in &targets {
        t.push(*a, *z, *zz);
    }
    let boxed = app.peek.as_ref().unwrap().zone(&app.doc, cols, rows);
    let inside = |z: &Zone| boxed.contains(z.x, z.y);
    let link = targets
        .iter()
        .find(|(a, z, _)| matches!(a, Action::LinkOpen(_)) && inside(z))
        .copied()
        .expect("the link in the footnote is a button");
    assert_eq!(under(&buf, link.1), "the top");
    assert_eq!(t.hit(link.1.x, link.1.y).map(|h| h.action), Some(link.0));
    let mark = targets
        .iter()
        .find(|(a, z, _)| matches!(a, Action::FootnotePeek { .. }) && inside(z))
        .copied()
        .expect("the mark in the footnote is a button");
    assert_eq!(under(&buf, mark.1), "[^again]");
    assert_eq!(t.hit(mark.1.x, mark.1.y).map(|h| h.action), Some(mark.0));

    // The mark opens ITS footnote in place of this one.
    update(&mut app, mark.0);
    assert_eq!(
        app.peek.as_ref().map(|p| p.label.as_str()),
        Some("[^again]")
    );
    // And the link closes the box and goes where it says.
    let mut app = rich_reader(cols, rows);
    update(&mut app, Action::Scroll(Span::Line, 2));
    open_first(&mut app, cols, rows);
    let (_, targets) = painted(&app, cols, rows);
    let link = targets
        .iter()
        .find(|(a, _, _)| matches!(a, Action::LinkOpen(_)))
        .copied()
        .unwrap();
    assert_eq!(
        app.view.scroll_row, 2,
        "the fixture starts away from the top"
    );
    update(&mut app, link.0);
    assert!(app.peek.is_none());
    assert_eq!(app.view.scroll_row, 0, "the link to #notes went to the top");
}

/// A search hit inside the footnote is lit inside the box: the box shows
/// the document's own bytes, so the document's hits are its hits.
#[test]
fn a_search_hit_in_the_footnote_is_lit_in_the_box() {
    let (cols, rows) = (80, 24);
    let mut app = rich_reader(cols, rows);
    open_first(&mut app, cols, rows);
    app.matches = Some(carrel_core::search(&app.doc, "measurement", false));
    let (buf, _) = painted(&app, cols, rows);
    let z = app.peek.as_ref().unwrap().zone(&app.doc, cols, rows);
    let y = z.y + 1;
    let row: String = (z.x..z.x + z.w).map(|x| buf[(x, y)].symbol()).collect();
    let col = row.find("measurement").expect("the first row has the word");
    let x = z.x + u16::try_from(row[..col].chars().count()).unwrap();
    let lit = [
        carrel::theme::match_normal().bg,
        carrel::theme::match_current().bg,
    ];
    assert!(
        lit.contains(&Some(buf[(x, y)].bg)),
        "{:?} is not a match colour",
        buf[(x, y)].bg
    );
    assert!(
        !lit.contains(&Some(buf[(z.x + 2, y)].bg)),
        "only the hit is lit"
    );
}

/// At thirty columns the box is narrower and the footnote wraps to fit
/// it; nothing is clipped off the right.
#[test]
fn at_thirty_columns_the_footnote_wraps_inside_the_box() {
    let (cols, rows) = (30, 20);
    let mut app = rich_reader(cols, rows);
    open_first(&mut app, cols, rows);
    let mut seen = Vec::new();
    for _ in 0..200 {
        seen.extend(check_rows(&app, cols, rows));
        if app.peek.as_ref().unwrap().view(&app.doc, cols, rows).below == 0 {
            break;
        }
        update(&mut app, Action::PeekScroll(1));
    }
    let words: String = seen.join(" ");
    assert!(words.contains("The last word."), "{words}");
}
