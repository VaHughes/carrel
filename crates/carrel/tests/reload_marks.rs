//! Reloads under folds and bookmarks — found by a random-sequence fuzzer
//! (2026-11-14), each a minimal sequence from a class of failures.
use carrel::action::Action;
use carrel::app::{App, update};
use carrel_core::Document;

fn at(app: &mut App, needle: &str) {
    app.view.anchor = app.doc.text.find(needle).unwrap() as u32;
    app.relayout();
}

fn shown(app: &App) -> bool {
    let b = carrel::view::anchor_block(&app.doc, app.view.anchor);
    app.layout.height(b) > 0
}

fn block_of(app: &App) -> String {
    let b = carrel::view::anchor_block(&app.doc, app.view.anchor);
    let r = app.doc.node_for_block(b).doc.clone();
    app.doc.text[r.start as usize..r.end as usize].to_string()
}

#[test]
fn a_bookmark_survives_the_file_being_emptied_twice_and_rewritten() {
    let src = "# Plan\n\nstep one\n\nstep two\n";
    let mut app = App::new("t.md".into(), Document::parse(src), 40, 12);
    update(&mut app, Action::MarkToggle);
    assert_eq!(app.marks, vec![0]);
    app.reload_from(""); // `cmd > PLAN.md` truncates...
    app.reload_from(""); // ...a second poll still sees it empty...
    app.reload_from(src); // ...and the same text comes back.
    assert_eq!(
        app.marks,
        vec![0],
        "the bookmark was dropped although the reader never moved"
    );
}

#[test]
fn control_the_reading_place_survives_two_empty_writes() {
    let src = "# Plan\n\nstep one\n\nstep two\n";
    let mut app = App::new("t.md".into(), Document::parse(src), 40, 12);
    at(&mut app, "step two");
    app.reload_from("");
    app.reload_from("");
    app.reload_from(src);
    assert_eq!(block_of(&app), "step two");
}

#[test]
fn a_bookmark_survives_one_empty_write() {
    let src = "# Plan\n\nstep one\n\nstep two\n";
    let mut app = App::new("t.md".into(), Document::parse(src), 40, 12);
    update(&mut app, Action::MarkToggle);
    app.reload_from("");
    app.reload_from(src);
    assert_eq!(app.marks, vec![0]);
}

#[test]
fn clearing_the_last_bookmark_with_the_list_open_leaves_no_cursor_past_the_end() {
    let mut app = App::new("t.md".into(), Document::parse("# A\n\npara\n"), 40, 12);
    update(&mut app, Action::MarkToggle);
    update(&mut app, Action::MarkListToggle);
    assert_eq!(app.mark_list, Some(0));
    update(&mut app, Action::MarkToggle); // `m` again: clears the only mark
    assert!(app.marks.is_empty());
    assert!(
        app.mark_list.is_none_or(|i| i < app.marks.len()),
        "bookmark list still open on row {:?} of {} marks",
        app.mark_list,
        app.marks.len()
    );
}

#[test]
fn a_reload_that_removes_the_heading_you_are_on_does_not_drop_you_into_a_fold() {
    let src = "# A\n\na body\n\n# B\n\nb body\n";
    let mut app = App::new("t.md".into(), Document::parse(src), 40, 12);
    at(&mut app, "B");
    update(&mut app, Action::FoldAll);
    assert!(
        shown(&app),
        "precondition: collapse-all keeps the anchor shown"
    );
    // The agent deletes the heading the reader is sitting on.
    app.reload_from("# A\n\na body\n\nb body\n");
    let b = carrel::view::anchor_block(&app.doc, app.view.anchor);
    assert!(
        shown(&app),
        "anchor {} is in block {b:?} ({:?}), hidden under the fold of A",
        app.view.anchor,
        &app.doc.text[app.doc.node_for_block(b).doc.start as usize
            ..app.doc.node_for_block(b).doc.end as usize]
    );
}

#[test]
fn undoing_a_delete_returns_a_reader_who_never_moved_to_the_paragraph_they_were_on() {
    let v0 = "# Title\n\nalpha para\n\nbeta para\n";
    let v1 = "# Title\n\nalpha edited para\n\nbeta para\n"; // their paragraph edited
    let v2 = "# Title\n\nbeta para\n"; // then deleted
    let mut app = App::new("t.md".into(), Document::parse(v0), 40, 12);
    at(&mut app, "alpha");
    app.reload_from(v1);
    let then = block_of(&app);
    assert_eq!(
        then, "alpha edited para",
        "precondition: the edit keeps the reader on the edited paragraph"
    );
    app.reload_from(v2);
    app.reload_from(v1); // the delete is undone; the reader has not moved since
    assert_eq!(block_of(&app), then);
}

/// The same, undone all the way: the original paragraph comes back.
#[test]
fn undoing_every_edit_returns_a_reader_who_never_moved_to_the_original() {
    let v0 = "# Title\n\nalpha para\n\nbeta para\n";
    let v1 = "# Title\n\nalpha edited para\n\nbeta para\n";
    let v2 = "# Title\n\nbeta para\n";
    let mut app = App::new("t.md".into(), Document::parse(v0), 40, 12);
    at(&mut app, "alpha");
    for v in [v1, v2, v1, v0] {
        app.reload_from(v);
    }
    assert_eq!(block_of(&app), "alpha para");
}

/// The paragraph being read is edited where it stands, and its old text
/// later reappears elsewhere: the reader who never moved stays on the edit.
#[test]
fn an_edited_paragraph_stays_the_readers_when_its_old_text_reappears_elsewhere() {
    let mut v0 = String::from("# Title\n\nalpha para\n\n");
    for i in 0..30 {
        let _ = std::fmt::Write::write_fmt(&mut v0, format_args!("filler {i}\n\n"));
    }
    let v1 = v0.replace("alpha para", "alpha edited para");
    let v2 = format!("{v1}alpha para\n");
    let mut app = App::new("t.md".into(), Document::parse(&v0), 40, 12);
    at(&mut app, "alpha");
    app.reload_from(&v1);
    assert_eq!(block_of(&app), "alpha edited para");
    app.reload_from(&v2);
    assert_eq!(block_of(&app), "alpha edited para");
}
