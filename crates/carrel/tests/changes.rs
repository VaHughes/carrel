//! What a reload changed.
//!
//! The reader this is for has an agent rewriting the file they are looking
//! at. "reloaded" tells them something happened; these are about telling
//! them what, and taking them to it.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use carrel::action::Action;
use carrel::app::{App, update};
use carrel_core::{BlockIdx, Document};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

const BASE: &str =
    "# Plan\n\nFirst paragraph.\n\nSecond paragraph.\n\n## Later\n\nThird paragraph.\n";

fn opened(dir: &Path, cols: u16, rows: u16) -> (App, PathBuf) {
    let path = dir.join("PLAN.md");
    std::fs::write(&path, BASE).unwrap();
    let mut app = App::new("PLAN.md".into(), Document::parse(BASE), cols, rows);
    app.file = Some(path.clone());
    app.on_resize(cols, rows);
    (app, path)
}

fn rewrite(app: &mut App, path: &Path, src: &str) {
    std::fs::write(path, src).unwrap();
    app.reload().unwrap();
}

fn changed(app: &App) -> Vec<&str> {
    app.changed.iter().map(|b| app.doc.block_text(*b)).collect()
}

#[test]
fn an_appended_paragraph_is_the_one_thing_marked() {
    let d = tempfile::tempdir().unwrap();
    let (mut app, path) = opened(d.path(), 80, 24);
    rewrite(&mut app, &path, &format!("{BASE}\nA fourth, new.\n"));
    assert_eq!(changed(&app), ["A fourth, new."]);
    assert_eq!(
        app.note.as_deref(),
        Some("reloaded — 1 change · c goes to it")
    );
}

#[test]
fn inserting_at_the_top_does_not_mark_everything_below_it() {
    // Every block after the insertion has a new index and the same text.
    let d = tempfile::tempdir().unwrap();
    let (mut app, path) = opened(d.path(), 80, 24);
    let src = BASE.replace("# Plan\n\n", "# Plan\n\nA new opening.\n\n");
    rewrite(&mut app, &path, &src);
    assert_eq!(changed(&app), ["A new opening."]);
}

#[test]
fn an_edited_paragraph_is_marked_and_its_twin_is_not() {
    let d = tempfile::tempdir().unwrap();
    let (mut app, path) = opened(d.path(), 80, 24);
    // A repeated line. One copy before; afterward that copy, an edited one,
    // and a second identical copy — which is an addition, and is marked,
    // while the copy that was always there is not.
    rewrite(&mut app, &path, "# Plan\n\nsame\n\nother\n");
    update(&mut app, Action::Dismiss); // acknowledge: this is the base now
    rewrite(&mut app, &path, "# Plan\n\nsame\n\nother, edited\n\nsame\n");
    assert_eq!(changed(&app), ["other, edited", "same"]);
    assert_eq!(
        app.changed.last().copied(),
        Some(BlockIdx(3)),
        "the SECOND `same` is the new one; the first was there all along"
    );
}

#[test]
fn a_reload_that_changes_nothing_you_can_see_marks_nothing() {
    let d = tempfile::tempdir().unwrap();
    let (mut app, path) = opened(d.path(), 80, 24);
    // Source formatting only: the display text is identical.
    rewrite(&mut app, &path, &BASE.replace("\n\n", "\n\n\n"));
    assert!(app.changed.is_empty());
    assert_eq!(app.note.as_deref(), Some("reloaded"));
}

#[test]
fn a_complete_rewrite_is_not_a_mark_on_every_block() {
    let d = tempfile::tempdir().unwrap();
    let (mut app, path) = opened(d.path(), 80, 24);
    rewrite(&mut app, &path, "# Other\n\nNothing in common.\n");
    assert!(app.changed.is_empty(), "{:?}", changed(&app));
    assert_eq!(app.note.as_deref(), Some("reloaded"));
}

#[test]
fn saves_in_a_row_add_up_until_the_reader_puts_them_away() {
    let d = tempfile::tempdir().unwrap();
    let (mut app, path) = opened(d.path(), 80, 24);
    rewrite(&mut app, &path, &format!("{BASE}\nOne.\n"));
    rewrite(&mut app, &path, &format!("{BASE}\nOne.\n\nTwo.\n"));
    assert_eq!(
        changed(&app),
        ["One.", "Two."],
        "the second save must not wipe what the first one marked"
    );

    update(&mut app, Action::Dismiss);
    assert!(app.changed.is_empty(), "Esc puts the news away");
    rewrite(
        &mut app,
        &path,
        &format!("{BASE}\nOne.\n\nTwo.\n\nThree.\n"),
    );
    assert_eq!(
        changed(&app),
        ["Three."],
        "and the next save starts from here"
    );
}

#[test]
fn opening_another_document_forgets_the_marks() {
    let d = tempfile::tempdir().unwrap();
    let (mut app, path) = opened(d.path(), 80, 24);
    rewrite(&mut app, &path, &format!("{BASE}\nOne.\n"));
    let other = d.path().join("other.md");
    std::fs::write(&other, "# Other\n").unwrap();
    app.open_path(&other).unwrap();
    assert!(
        app.changed.is_empty(),
        "block indices of a document that is gone"
    );
}

#[test]
fn the_key_walks_the_changes_and_opens_a_collapsed_section_to_reach_one() {
    let d = tempfile::tempdir().unwrap();
    let (mut app, path) = opened(d.path(), 80, 8);
    let mut src = String::from(BASE);
    for i in 0..30 {
        let _ = write!(src, "\nfiller {i}\n");
    }
    rewrite(&mut app, &path, &src);
    app.changed.clear();
    update(&mut app, Action::Dismiss);
    // Two edits, far apart; the second inside `## Later`.
    let src = src
        .replace("First paragraph.", "First paragraph, edited.")
        .replace("filler 25\n", "filler 25, edited\n");
    rewrite(&mut app, &path, &src);
    assert_eq!(changed(&app).len(), 2);
    update(&mut app, Action::FoldAll);

    update(&mut app, Action::ChangeStep(1));
    let top = |app: &App| {
        app.doc
            .block_text(app.layout.block_at_row(app.view.scroll_row))
            .to_string()
    };
    assert_eq!(top(&app), "First paragraph, edited.");
    assert_eq!(app.note.as_deref(), Some("change 1 of 2"));
    update(&mut app, Action::ChangeStep(1));
    assert_eq!(app.note.as_deref(), Some("change 2 of 2"));
    let block = app.changed[1];
    assert!(app.layout.height(block) > 0, "the section opened for it");
    update(&mut app, Action::ChangeStep(1));
    assert_eq!(
        app.note.as_deref(),
        Some("change 1 of 2"),
        "and round again"
    );
    update(&mut app, Action::ChangeStep(-1));
    assert_eq!(app.note.as_deref(), Some("change 2 of 2"));
}

#[test]
fn with_nothing_changed_the_key_says_so() {
    let d = tempfile::tempdir().unwrap();
    let (mut app, _) = opened(d.path(), 80, 24);
    update(&mut app, Action::ChangeStep(1));
    assert!(app.note.as_deref().unwrap().contains("nothing has changed"));
    assert_eq!(app.view.scroll_row, 0);
}

#[test]
fn a_changed_block_is_barred_in_the_margin_and_counted_on_the_status_row() {
    let d = tempfile::tempdir().unwrap();
    let (cols, rows) = (80u16, 24u16);
    let (mut app, path) = opened(d.path(), cols, rows);
    rewrite(
        &mut app,
        &path,
        &BASE.replace("Second paragraph.", "Second, rewritten."),
    );
    assert_eq!(changed(&app), ["Second, rewritten."]);

    let mut term = Terminal::new(TestBackend::new(cols, rows)).unwrap();
    let mut painted = carrel::render::Painted::default();
    term.draw(|f| {
        carrel::render::draw_full(f, &app, &mut painted, &mut std::collections::HashMap::new());
    })
    .unwrap();
    let buf = term.backend().buffer().clone();
    let row = |y: u16| -> String { (0..cols).map(|x| buf[(x, y)].symbol()).collect() };
    let barred: Vec<String> = (0..rows).map(row).filter(|r| r.contains('▎')).collect();
    assert_eq!(barred.len(), 1, "{barred:?}");
    assert!(barred[0].contains("Second, rewritten."), "{barred:?}");
    // The bar is in the margin, left of the text, never on it.
    let bar_x = barred[0].chars().position(|c| c == '▎').unwrap();
    let text_x = barred[0].chars().position(|c| c == 'S').unwrap();
    assert!(bar_x < text_x);

    let chip = painted
        .targets
        .as_slice()
        .iter()
        .find(|t| t.action == Action::ChangeStep(1))
        .expect("the status row offers the change");
    let label: String = (chip.zone.x..chip.zone.x + chip.zone.w)
        .map(|x| buf[(x, chip.zone.y)].symbol())
        .collect();
    assert_eq!(label, "1 changed");
    let _ = BlockIdx(0);
}

// --- another document was written ---

use carrel::app::{Sibling, newest_sibling};
use carrel::scan::Entry;
use std::time::{Duration, SystemTime};

fn entry(dir: &Path, name: &str, secs: u64) -> Entry {
    let path = dir.join(name);
    std::fs::write(&path, "x\n").unwrap();
    Entry {
        path,
        mtime: SystemTime::UNIX_EPOCH + Duration::from_secs(secs),
    }
}

#[test]
fn the_newest_other_document_is_the_one_offered() {
    let d = tempfile::tempdir().unwrap();
    let since = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
    let entries = vec![
        entry(d.path(), "old.md", 50),
        entry(d.path(), "PLAN.md", 300), // the one being read
        entry(d.path(), "notes.md", 200),
        entry(d.path(), "CHANGELOG.md", 250),
    ];
    let reading = d.path().join("PLAN.md").canonicalize().unwrap();
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
    assert_eq!(
        newest_sibling(&entries, &reading, since, now),
        Some(Sibling {
            path: d.path().join("CHANGELOG.md"),
            more: 1,
        }),
        "the newest that is not this document, and a count of the rest"
    );
    let later = SystemTime::UNIX_EPOCH + Duration::from_secs(400);
    assert_eq!(
        newest_sibling(&entries, &reading, later, now),
        None,
        "nothing new"
    );
    assert_eq!(newest_sibling(&[], &reading, since, now), None);
}

/// A file dated tomorrow is newer than every moment it could be dismissed
/// at: it was announced as just changed, again, after every `Esc`.
#[test]
fn a_document_dated_in_the_future_is_not_news() {
    let d = tempfile::tempdir().unwrap();
    let since = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(200);
    let entries = vec![
        entry(d.path(), "PLAN.md", 50),
        entry(d.path(), "tomorrow.md", 90_000),
        entry(d.path(), "fresh.md", 150),
    ];
    let reading = d.path().join("PLAN.md").canonicalize().unwrap();
    assert_eq!(
        newest_sibling(&entries, &reading, since, now),
        Some(Sibling {
            path: d.path().join("fresh.md"),
            more: 0,
        })
    );
}

#[test]
fn the_chip_opens_the_document_and_back_returns() {
    let d = tempfile::tempdir().unwrap();
    let (mut app, path) = opened(d.path(), 100, 24);
    let other = d.path().join("CHANGELOG.md");
    std::fs::write(&other, "# Changes\n\nthe other document\n").unwrap();
    app.sibling = Some(Sibling {
        path: other.clone(),
        more: 2,
    });

    let mut term = Terminal::new(TestBackend::new(100, 24)).unwrap();
    let mut painted = carrel::render::Painted::default();
    term.draw(|f| {
        carrel::render::draw_full(f, &app, &mut painted, &mut std::collections::HashMap::new());
    })
    .unwrap();
    let buf = term.backend().buffer().clone();
    let chip = painted
        .targets
        .as_slice()
        .iter()
        .find(|t| t.action == Action::SiblingOpen)
        .expect("the status row offers it");
    let label: String = (chip.zone.x..chip.zone.x + chip.zone.w)
        .map(|x| buf[(x, chip.zone.y)].symbol())
        .collect();
    assert_eq!(label, "● CHANGELOG.md changed (+2)");

    update(&mut app, Action::SiblingOpen);
    assert_eq!(app.file.as_deref(), Some(other.as_path()));
    assert_eq!(app.sibling, None, "acted on, so it is put away");
    update(&mut app, Action::Back);
    assert_eq!(app.file.as_deref(), Some(path.as_path()));
}

#[test]
fn esc_puts_the_news_away_without_acting_on_it() {
    let d = tempfile::tempdir().unwrap();
    let (mut app, path) = opened(d.path(), 100, 24);
    app.sibling = Some(Sibling {
        path: d.path().join("x.md"),
        more: 0,
    });
    update(&mut app, Action::Dismiss);
    assert_eq!(app.sibling, None);
    assert_eq!(app.file.as_deref(), Some(path.as_path()), "still here");
    // With nothing offered, the action is inert.
    assert_eq!(
        update(&mut app, Action::SiblingOpen),
        carrel::app::Outcome::Idle
    );
}
