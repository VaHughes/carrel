//! The outline picker and the bookmark list name a heading or a paragraph by
//! how it BEGINS: a row too long for the pane keeps its start and ends in
//! `…`. (Paths, in the link panes, keep their end instead.)
use std::fmt::Write as _;

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

/// The rows inside the box titled `title`: between its `┌` and `└` rows,
/// and between its `┌` and the last column of its `×` — never the page
/// around it, which holds the same words.
fn pane(screen: &[String], title: &str) -> Vec<String> {
    let chars: Vec<Vec<char>> = screen.iter().map(|r| r.chars().collect()).collect();
    let top = screen
        .iter()
        .position(|r| r.contains(&format!("┌ {title}")))
        .expect("the box is up");
    let left = chars[top].iter().position(|&c| c == '┌').unwrap();
    let right = chars[top].iter().rposition(|&c| c == '×').unwrap() + 1;
    let bottom = (top + 1..chars.len())
        .find(|&y| chars[y].get(left) == Some(&'└'))
        .expect("the box closes");
    (top + 1..bottom)
        .map(|y| chars[y][left..=right].iter().collect())
        .collect()
}

/// A paragraph long enough to wrap over several rows of any pane.
fn para(n: u32) -> String {
    format!(
        "Paragraph {n} is a long stretch of prose that a reader bookmarked \
         because it said something worth coming back to later on, at length, \
         twice over and more besides."
    )
}

/// **The defect** (black-box review, 2026-10-31): with long labels the
/// selected mark wrapped over the rows the others needed, so three marks
/// showed as the selected one three times — its first two rows, then its
/// start again — and the other two were never on screen. Every mark must
/// be visible whichever is selected, each once.
#[test]
fn every_long_bookmark_is_listed_once_whichever_is_selected() {
    let src = format!("# Top\n\n{}\n\n{}\n\n{}\n", para(1), para(2), para(3));
    for (cols, rows_) in [(80, 24), (60, 20), (120, 40)] {
        let mut app = App::new("o.md".into(), Document::parse(&src), cols, rows_);
        for n in 1..=3 {
            let at = u32::try_from(app.doc.text.find(&format!("Paragraph {n}")).unwrap()).unwrap();
            app.marks.push(at);
        }
        update(&mut app, Action::MarkListToggle);
        for selected in 0..3 {
            app.mark_list = Some(selected);
            let screen = rows(&app);
            let pane = pane(&screen, "bookmarks");
            let all = screen.join("\n");
            for n in 1..=3 {
                let label = format!("Paragraph {n} is");
                let listed = pane.iter().filter(|r| r.contains(&label)).count();
                assert_eq!(
                    listed, 1,
                    "{cols}x{rows_}, mark {n}, selected {selected}:\n{all}"
                );
            }
            assert!(
                pane.iter()
                    .any(|r| r.contains(&format!("▸ Paragraph {} is", selected + 1))),
                "the selected one is marked:\n{all}"
            );
        }
    }
}

/// The outline shares the list: three long headings, each listed once,
/// whichever is selected.
#[test]
fn every_long_heading_is_listed_once_whichever_is_selected() {
    let h = |n: u32| format!("Heading {n} {LONG} and further still beyond the edge of any pane");
    let src = format!("## {}\n\na\n\n## {}\n\nb\n\n## {}\n\nc\n", h(1), h(2), h(3));
    let mut app = App::new("o.md".into(), Document::parse(&src), 80, 24);
    update(&mut app, Action::OutlineToggle);
    for selected in 0..3 {
        app.outline.as_mut().unwrap().selected = selected;
        let screen = rows(&app);
        let all = screen.join("\n");
        for n in 1..=3 {
            let listed = pane(&screen, "outline")
                .iter()
                .filter(|r| r.contains(&format!("Heading {n} Section")))
                .count();
            assert_eq!(listed, 1, "heading {n}, selected {selected}:\n{all}");
        }
    }
}

/// A selected label with room to spare still opens out over several rows,
/// and its last row CONTINUES it — it does not start it again.
#[test]
fn a_selected_label_cut_short_continues_rather_than_starting_again() {
    let long = format!("{} {}", para(1), para(1).replace("Paragraph 1", "and then"));
    let src = format!("# Top\n\n{long}\n");
    let mut app = App::new("o.md".into(), Document::parse(&src), 60, 20);
    let at = u32::try_from(app.doc.text.find("Paragraph 1").unwrap()).unwrap();
    app.marks.push(at);
    update(&mut app, Action::MarkListToggle);
    let screen = rows(&app);
    let all = screen.join("\n");
    let rows_ = pane(&screen, "bookmarks");
    assert_eq!(
        rows_
            .iter()
            .filter(|r| r.contains("Paragraph 1 is"))
            .count(),
        1,
        "its start appears once:\n{all}"
    );
    assert!(rows_.len() >= 2, "room for more than one row:\n{all}");
    assert!(rows_[0].contains("▸ Paragraph 1 is"), "{all}");
    assert!(
        rows_[1].contains("bookmarked") || rows_[1].contains("because"),
        "the second row goes on from the first:\n{all}"
    );
    assert!(
        rows_.last().unwrap().trim_end().ends_with('…'),
        "and the cut is marked where it is cut:\n{all}"
    );
}

/// A list longer than its pane scrolls anyway, so the selected entry still
/// opens out over more than one row — up to half the pane, leaving the rest
/// to its neighbors. The fix that keeps every entry of a short list visible
/// counted the WHOLE list, so a long outline cut its selection to one row
/// (cold review, 2026-11-02).
#[test]
fn a_long_list_still_opens_out_its_selected_entry() {
    let h = |n: u32| format!("Heading {n} {LONG} and further still beyond the edge of any pane");
    let mut src = String::new();
    for n in 1..=40 {
        writeln!(src, "## {}\n\nx\n", h(n)).unwrap();
    }
    let mut app = App::new("o.md".into(), Document::parse(&src), 80, 24);
    update(&mut app, Action::OutlineToggle);
    app.outline.as_mut().unwrap().selected = 5;
    let screen = rows(&app);
    let all = screen.join("\n");
    let rows_ = pane(&screen, "outline");
    let at = rows_
        .iter()
        .position(|r| r.contains("▸ Heading 6 "))
        .expect(&all);
    assert!(
        rows_[at + 1].contains("further") || rows_[at + 1].contains("goes on"),
        "the selection wraps onto a second row:\n{all}"
    );
    let others = rows_.iter().filter(|r| r.contains("  Heading ")).count();
    assert!(
        others * 2 >= rows_.len() - 1,
        "and half the pane is its neighbors:\n{all}"
    );
}
