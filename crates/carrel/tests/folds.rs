//! Collapsed sections that stay collapsed: across closing the document, and
//! across the file being rewritten under the reader.

use std::path::Path;

use carrel::action::Action;
use carrel::app::{App, update};
use carrel_core::{Document, NodeKind};

const SRC: &str = "# Plan\n\nintro\n\n## Setup\n\nsetup body\n\n## Rollout\n\nrollout body\n\n\
<details>\n<summary>The fine print</summary>\n\nhidden words\n\n</details>\n\n## Setup\n\nthe second setup\n";

fn reader(dir: &Path, state: &Path) -> App {
    let path = dir.join("PLAN.md");
    std::fs::write(&path, SRC).unwrap();
    let mut app = App::new("x".into(), Document::parse("x"), 80, 30);
    app.state_dir = Some(state.to_path_buf());
    app.open_path(&path).unwrap();
    app
}

fn folded_titles(app: &App) -> Vec<String> {
    app.doc
        .nodes
        .iter()
        .filter(|n| app.folded.contains(&n.id))
        .map(|n| app.doc.text[n.doc.start as usize..n.doc.end as usize].to_string())
        .collect()
}

fn heading(app: &App, nth: usize) -> u32 {
    app.doc
        .nodes
        .iter()
        .filter(|n| matches!(n.kind, NodeKind::Heading { level: 2 }))
        .nth(nth)
        .map(|n| n.doc.start)
        .unwrap()
}

#[test]
fn a_document_reopens_with_the_sections_it_was_left_with() {
    let d = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut app = reader(d.path(), state.path());
    // The SECOND `## Setup`, to prove two headings of one name are told apart.
    let second_setup = heading(&app, 2);
    update(&mut app, Action::FoldAt(second_setup));
    let at = heading(&app, 1);
    update(&mut app, Action::FoldAt(at));
    assert_eq!(app.fold_keys(), ["h:rollout", "h:setup-1"]);
    let rows = app.layout.total_rows();
    update(&mut app, Action::CloseFile); // saves, as leaving any document does

    let again = reader(d.path(), state.path());
    assert_eq!(folded_titles(&again), ["Rollout", "Setup"]);
    assert!(
        !again.folded.contains(
            &again
                .doc
                .nodes
                .iter()
                .find(|n| n.doc.start == heading(&again, 0))
                .unwrap()
                .id
        ),
        "the first Setup was never collapsed"
    );
    assert_eq!(
        again.layout.total_rows(),
        rows,
        "and it is laid out that way"
    );
}

#[test]
fn expanding_everything_forgets_the_document() {
    let d = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut app = reader(d.path(), state.path());
    update(&mut app, Action::FoldAll);
    update(&mut app, Action::CloseFile);
    assert!(state.path().join("folds").exists());

    let mut app = reader(d.path(), state.path());
    assert!(!app.folded.is_empty());
    assert_eq!(app.folded_details.len(), 1, "the <details> too");
    update(&mut app, Action::UnfoldAll);
    update(&mut app, Action::CloseFile);
    let again = reader(d.path(), state.path());
    assert!(again.folded.is_empty() && again.folded_details.is_empty());
}

#[test]
fn a_section_that_was_renamed_comes_back_expanded_and_the_rest_stay() {
    let d = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut app = reader(d.path(), state.path());
    update(&mut app, Action::FoldAll);
    update(&mut app, Action::CloseFile);

    std::fs::write(
        d.path().join("PLAN.md"),
        SRC.replace("## Rollout", "## Shipping"),
    )
    .unwrap();
    let mut app = App::new("x".into(), Document::parse("x"), 80, 30);
    app.state_dir = Some(state.path().to_path_buf());
    app.open_path(&d.path().join("PLAN.md")).unwrap();
    let titles = folded_titles(&app);
    assert!(!titles.contains(&"Shipping".to_string()), "{titles:?}");
    assert!(titles.contains(&"Plan".to_string()) && titles.contains(&"Setup".to_string()));
}

/// The agent appends a line to the plan. Every section the reader had
/// closed used to spring open.
#[test]
fn a_reload_keeps_what_was_collapsed() {
    let d = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut app = reader(d.path(), state.path());
    let at = heading(&app, 0);
    update(&mut app, Action::FoldAt(at));
    let at = heading(&app, 1);
    update(&mut app, Action::FoldAt(at));
    let before = folded_titles(&app);
    assert_eq!(before, ["Setup", "Rollout"]);

    std::fs::write(
        d.path().join("PLAN.md"),
        format!("A new opening line.\n\n{SRC}\nAnd a new last one.\n"),
    )
    .unwrap();
    app.reload().unwrap();
    assert_eq!(folded_titles(&app), before, "still collapsed, by name");
    let setup = app.doc.block_at_doc(carrel_core::DocByte(
        u32::try_from(app.doc.text.find("setup body").unwrap()).unwrap(),
    ));
    assert_eq!(app.layout.height(setup), 0, "and laid out collapsed");
}

#[test]
fn a_pipe_and_a_generated_page_remember_nothing() {
    let state = tempfile::tempdir().unwrap();
    let mut app = App::new("(stdin)".into(), Document::parse(SRC), 80, 30);
    app.state_dir = Some(state.path().to_path_buf());
    update(&mut app, Action::FoldAll);
    update(&mut app, Action::CloseFile);
    assert!(
        !state.path().join("folds").exists(),
        "no file, no key to save under"
    );
}
