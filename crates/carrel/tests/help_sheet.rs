//! The help sheet as a list of buttons.
//!
//! It is a list of what carrel does beside the key that does each. For a
//! reader who does not know the keys that is already the list of things
//! they wanted, so clicking a row does it.

use carrel::action::{Action, SearchKey, Zone};
use carrel::app::{App, Outcome, update};
use carrel::keys::{HOME_HELP, READER_HELP};
use carrel_core::Document;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;

const SRC: &str = "# One\n\nsome text\n\n## Two\n\nmore text\n";

fn sheet(cols: u16, rows: u16) -> App {
    let mut app = App::new("d.md".into(), Document::parse(SRC), cols, rows);
    app.on_resize(cols, rows);
    update(&mut app, Action::HelpToggle);
    app
}

fn frame(app: &App, cols: u16, rows: u16) -> (Buffer, Vec<(u32, Zone)>) {
    let mut term = Terminal::new(TestBackend::new(cols, rows)).unwrap();
    let mut painted = carrel::render::Painted::default();
    term.draw(|f| {
        carrel::render::draw_full(f, app, &mut painted, &mut std::collections::HashMap::new());
    })
    .unwrap();
    let rows = painted
        .targets
        .as_slice()
        .iter()
        .filter_map(|t| match t.action {
            Action::HelpRun(i) => Some((i, t.zone)),
            _ => None,
        })
        .collect();
    (term.backend().buffer().clone(), rows)
}

fn under(buf: &Buffer, z: Zone) -> String {
    (z.x..z.x + z.w).map(|x| buf[(x, z.y)].symbol()).collect()
}

fn row_of(key: &str) -> u32 {
    u32::try_from(READER_HELP.iter().position(|(k, _)| *k == key).unwrap()).unwrap()
}

#[test]
fn each_button_sits_on_the_row_it_runs() {
    let app = sheet(80, 60);
    let (buf, rows) = frame(&app, 80, 60);
    assert!(rows.len() > 20, "most of the sheet is keys: {}", rows.len());
    for (i, zone) in &rows {
        let (key, desc) = READER_HELP[*i as usize];
        let text = under(&buf, *zone);
        assert!(
            text.contains(key) && text.contains(desc),
            "row {i} ({key:?}) is registered over {text:?}"
        );
    }
}

#[test]
fn a_sentence_is_not_a_button() {
    let app = sheet(80, 80);
    let (_, rows) = frame(&app, 80, 80);
    let buttons: Vec<&str> = rows
        .iter()
        .map(|(i, _)| READER_HELP[*i as usize].0)
        .collect();
    for sentence in ["drag", "wheel", "how it works", "click a tag", "42G"] {
        assert!(
            !buttons.contains(&sentence),
            "{sentence:?} offers to be run"
        );
    }
    assert!(!buttons.contains(&"§"));
}

#[test]
fn clicking_a_row_closes_the_sheet_and_does_what_the_row_says() {
    let mut app = sheet(80, 60);
    assert_eq!(
        update(&mut app, Action::HelpRun(row_of("o"))),
        Outcome::Redraw
    );
    assert!(app.help.is_none(), "the sheet closed");
    assert!(app.outline.is_some(), "and the outline opened");

    // A row that acts on the document acts on the document.
    let mut app = sheet(80, 60);
    update(&mut app, Action::HelpRun(row_of("zM zR")));
    assert!(app.help.is_none());
    assert!(!app.folded.is_empty(), "collapse all, collapsed");

    // The sheet's own row closes it rather than reopening it.
    let mut app = sheet(80, 60);
    update(&mut app, Action::HelpRun(row_of("h F1")));
    assert!(app.help.is_none());
}

#[test]
fn a_row_that_is_a_sentence_or_does_not_exist_leaves_the_sheet_alone() {
    let mut app = sheet(80, 60);
    for i in [row_of("drag"), row_of("42G"), 9_999] {
        assert_eq!(update(&mut app, Action::HelpRun(i)), Outcome::Idle);
        assert!(app.help.is_some());
    }
}

#[test]
fn a_filtered_sheet_still_runs_the_row_that_was_clicked() {
    // Filtering reorders and drops rows; the button carries the row's place
    // in the TABLE, not its place on screen.
    let mut app = sheet(80, 40);
    for c in "theme".chars() {
        update(&mut app, Action::HelpKey(SearchKey::Char(c)));
    }
    let (buf, rows) = frame(&app, 80, 40);
    let (i, zone) = rows
        .iter()
        .find(|(_, z)| under(&buf, *z).contains("next theme"))
        .copied()
        .expect("the theme row survived the filter");
    assert_eq!(READER_HELP[i as usize].0, "T");
    update(&mut app, Action::HelpRun(i));
    assert!(app.help.is_none());
    assert!(app.theme_cycle, "the theme was asked to change");
    let _ = zone;
}

#[test]
fn a_narrow_sheet_wraps_its_rows_and_every_line_of_one_is_the_button() {
    let app = sheet(30, 40);
    let (buf, rows) = frame(&app, 30, 40);
    assert!(!rows.is_empty());
    // A wrapped row is more than one line, all of them the same button.
    let mut per_row = std::collections::BTreeMap::<u32, usize>::new();
    for (i, _) in &rows {
        *per_row.entry(*i).or_default() += 1;
    }
    assert!(per_row.values().any(|n| *n > 1), "{per_row:?}");
    for (_, zone) in &rows {
        assert!(zone.x + zone.w <= 30);
        assert!(
            !under(&buf, *zone).trim().is_empty(),
            "a button over nothing"
        );
    }
}

#[test]
fn the_file_lists_sheet_runs_the_file_lists_keys() {
    let d = tempfile::tempdir().unwrap();
    let mut app = App::new_home(d.path().into(), Vec::new(), 80, 40);
    update(&mut app, Action::HelpToggle);
    let i = u32::try_from(HOME_HELP.iter().position(|(k, _)| *k == "d").unwrap()).unwrap();
    update(&mut app, Action::HelpRun(i));
    assert!(app.help.is_none());
    assert_eq!(
        app.home().unwrap().mode,
        carrel::home::HomeMode::Picker,
        "`d` on the file list chooses a folder"
    );
}
