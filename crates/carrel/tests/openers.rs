//! Every way a document arrives lays it out the same way.
//!
//! `App::new` runs the full layout. The later openers — a file from the home
//! screen or a link, a reload, a growing pipe, search results — once built a
//! plain one of their own instead: no math heights, the default table mode
//! whatever the reader had chosen, no divided words. It was replaced only if
//! something happened to force a relayout (a resize, an image arriving), so
//! the same file looked different depending on how it was reached.

use carrel::app::App;
use carrel_core::{BlockIdx, Document, NodeKind};

const SRC: &str = "Some prose before the math.\n\n\
$$\n\\frac{a}{b} + \\sqrt{x^2 + y^2}\n$$\n\n\
Some prose after it.\n\n\
| name | description | note |\n|---|---|---|\n\
| alpha | a value long enough to overflow a narrow window easily | the first one |\n\
| beta | another lengthy entry that keeps going and going | short |\n";

fn heights(app: &App) -> Vec<u32> {
    (0..app.doc.block_count())
        .map(|b| app.layout.height(BlockIdx(b as u32)))
        .collect()
}

fn blank(cols: u16, rows: u16) -> App {
    App::new("t.md".into(), Document::parse("x"), cols, rows)
}

#[test]
fn a_file_opened_later_has_the_layout_of_one_opened_first() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("m.md");
    std::fs::write(&path, SRC).unwrap();

    let first = App::new("m.md".into(), Document::parse(SRC), 60, 30);
    let math = (0..first.doc.block_count())
        .position(|b| first.doc.node_for_block(BlockIdx(b as u32)).kind == NodeKind::Math)
        .unwrap();
    assert!(
        first.layout.height(BlockIdx(math as u32)) > 2,
        "the fixture's math must be taller as art than as source"
    );

    let mut opened = blank(60, 30);
    opened.open_path(&path).unwrap();
    assert_eq!(heights(&opened), heights(&first), "open_path");

    let mut reloaded = blank(60, 30);
    reloaded.reload_from(SRC);
    assert_eq!(heights(&reloaded), heights(&first), "reload_from");
}

#[test]
fn the_table_mode_the_reader_chose_survives_opening_another_document() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("m.md");
    std::fs::write(&path, SRC).unwrap();

    let mut app = blank(40, 30);
    app.wrap_tables = true; // `t`: columns, not cards
    app.relayout();
    app.open_path(&path).unwrap();
    assert!(
        app.layout.wrap_tables(),
        "the layout in hand must be the one the reader's setting asks for"
    );
    app.reload_from(SRC);
    assert!(app.layout.wrap_tables(), "and after a reload");
}

// --- link destinations are URLs on the way in ---

use carrel::action::Action;
use carrel::app::update;

fn reader_in(dir: &std::path::Path, src: &str) -> App {
    let mut app = App::new("doc.md".into(), Document::parse(src), 60, 20);
    app.file = Some(dir.join("doc.md"));
    app.library_root = Some(dir.to_path_buf());
    app
}

/// `[notes](my%20notes.md)` is how every other tool writes a link to a file
/// with a space in its name. It used to look for a file literally called
/// `my%20notes.md`.
#[test]
fn a_percent_encoded_link_opens_the_file_it_names() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("my notes.md"), "the spaced one\n").unwrap();
    let mut app = reader_in(d.path(), "[notes](my%20notes.md)\n");
    update(&mut app, Action::LinkOpen(0));
    assert!(app.doc.text.contains("the spaced one"), "{:?}", app.note);
}

#[test]
fn a_name_that_really_contains_a_percent_escape_still_wins() {
    // Both exist. The link names the literal one, so that is what opens.
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("50%20off.md"), "the literal one\n").unwrap();
    std::fs::write(d.path().join("50 off.md"), "the decoded one\n").unwrap();
    let mut app = reader_in(d.path(), "[sale](50%20off.md)\n");
    update(&mut app, Action::LinkOpen(0));
    assert!(app.doc.text.contains("the literal one"), "{:?}", app.note);
}

/// Decoding must not become a way around the library's edge: an encoded
/// `..` is judged as the path it decodes to.
#[test]
fn an_encoded_path_out_of_the_folder_still_asks_first() {
    let outer = tempfile::tempdir().unwrap();
    let inside = outer.path().join("library");
    std::fs::create_dir(&inside).unwrap();
    std::fs::write(outer.path().join("secret.md"), "not yours\n").unwrap();
    let mut app = reader_in(&inside, "[out](%2E%2E/secret.md)\n");
    update(&mut app, Action::LinkOpen(0));
    assert!(
        !app.doc.text.contains("not yours"),
        "an encoded `..` opened a file outside the folder without asking"
    );
    assert!(
        app.note
            .as_deref()
            .unwrap_or_default()
            .contains("outside this folder"),
        "{:?}",
        app.note
    );
}

/// Search results are generated the same way the tags page is, from the
/// same untrusted file names.
#[test]
fn a_search_hit_in_an_awkwardly_named_file_opens_at_its_line() {
    let d = tempfile::tempdir().unwrap();
    for name in ["C# notes.md", "re: plan.md", "what?.md"] {
        let path = d.path().join(name);
        std::fs::write(&path, "first\n\nthe needle is here\n").unwrap();
        let hit = carrel::grep::Hit {
            path: path.clone(),
            count: 1,
            first_line: "the needle is here".into(),
            matches: vec![carrel::grep::HitLine {
                lineno: 3,
                line: "the needle is here".into(),
            }],
        };
        let mut app = blank(60, 20);
        app.library_root = Some(d.path().to_path_buf());
        app.open_results(d.path(), "needle", &[hit]);
        update(&mut app, Action::LinkOpen(0));
        assert_eq!(
            app.file.as_deref(),
            Some(path.as_path()),
            "{name}: {:?}",
            app.note
        );
    }
}
