//! The outline picker and the bookmark list name a heading or a paragraph by
//! how it BEGINS: a row too long for the pane keeps its start and ends in
//! `…`. (Paths, in the link panes, keep their end instead.)
use carrel::action::Action;
use carrel::app::{App, update};
use carrel::render::{Painted, draw_full};
use carrel_core::Document;
use ratatui::{Terminal, backend::TestBackend};

fn rows(app: &App) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(app.cols, app.rows)).unwrap();
    let mut painted = Painted::default();
    terminal
        .draw(|f| draw_full(f, app, &mut painted, &mut std::collections::HashMap::new()))
        .unwrap();
    let buf = terminal.backend().buffer();
    (0..app.rows)
        .map(|y| (0..app.cols).map(|x| buf[(x, y)].symbol()).collect())
        .collect()
}

const LONG: &str = "Section with a considerably longer heading title that goes on and on";

#[test]
fn a_long_heading_keeps_its_start_in_the_outline() {
    let src = format!("# Top\n\nBody.\n\n## {LONG}\n\nMore.\n\n## Short\n\nEnd.\n");
    let mut app = App::new("o.md".into(), Document::parse(&src), 100, 30);
    update(&mut app, Action::OutlineToggle);
    let screen = rows(&app);
    assert!(
        screen
            .iter()
            .any(|r| r.contains("Section with a considerably") && r.contains('…')),
        "the heading's start, with its cut marked:\n{}",
        screen.join("\n")
    );
    assert!(
        !screen.iter().any(|r| r.contains("…erably")),
        "cut from the left:\n{}",
        screen.join("\n")
    );
}

#[test]
fn a_long_bookmarked_paragraph_keeps_its_start_in_the_list() {
    let src = format!("# Top\n\n{LONG} and then a great deal more after it.\n");
    let mut app = App::new("o.md".into(), Document::parse(&src), 60, 20);
    let at = u32::try_from(app.doc.text.find("Section").unwrap()).unwrap();
    app.marks.push(at);
    app.marks.push(0);
    update(&mut app, Action::MarkListToggle);
    // The paragraph is the row that is NOT selected, so it is one line.
    app.mark_list = Some(1);
    let screen = rows(&app);
    assert!(
        screen
            .iter()
            .any(|r| r.contains("Section with a considerably") && r.contains('…')),
        "{}",
        screen.join("\n")
    );
}
