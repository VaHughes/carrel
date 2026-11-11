//! Follow mode means the same thing for a watched file and a live pipe.
use std::fmt::Write as _;

use carrel::action::{Action, Direction, Span};
use carrel::app::{App, update};
use carrel_core::Document;

fn source(lines: usize) -> String {
    let mut text = String::new();
    for i in 0..lines {
        writeln!(text, "Paragraph {i}.\n").unwrap();
    }
    text
}

fn reader() -> App {
    App::new("live.md".into(), Document::parse(&source(60)), 40, 8)
}

#[test]
fn an_open_file_can_follow_rewrites_and_stop_when_the_reader_moves_back() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("live.md");
    std::fs::write(&file, source(60)).unwrap();
    let mut app = reader();
    app.file = Some(file.clone());
    let item = carrel::menu::global(&app)
        .into_iter()
        .find(|item| item.action == Some(Action::FollowToggle))
        .unwrap();
    assert!(
        item.enabled,
        "the file is watched, so the menu must offer following"
    );
    update(&mut app, Action::FollowToggle);
    let before = app.view.scroll_row;
    std::fs::write(&file, source(100)).unwrap();
    app.reload().unwrap();
    assert!(app.following);
    assert!(app.view.scroll_row > before);
    assert_eq!(app.view.scroll_row, app.layout.max_scroll(app.text_h()));
    update(&mut app, Action::Scroll(Span::Line, -1));
    assert!(!app.following);
    let anchor = app.view.anchor;
    std::fs::write(&file, source(140)).unwrap();
    app.reload().unwrap();
    assert_eq!(
        app.view.anchor, anchor,
        "scrolling back means stay with this text"
    );
}

#[test]
fn direct_navigation_detaches_from_a_live_stream() {
    for action in [
        Action::GoToRow(1),
        Action::ScrollTo(1),
        Action::BlockStep(-1),
        Action::SearchOpen(Direction::Forward),
    ] {
        let mut app = reader();
        app.streaming = true;
        app.path = "(stdin)".into();
        app.piped = Some(source(60));
        update(&mut app, Action::FollowToggle);
        update(&mut app, action);
        assert!(!app.following, "{action:?}");
        let anchor = app.view.anchor;
        app.reload_from(&source(120));
        assert_eq!(app.view.anchor, anchor, "{action:?}");
    }
}

#[test]
fn following_and_auto_read_do_not_compete() {
    let mut app = reader();
    app.streaming = true;
    app.path = "(stdin)".into();
    app.piped = Some(source(60));
    update(&mut app, Action::AutoToggle);
    update(&mut app, Action::FollowToggle);
    assert!(app.following);
    assert!(!app.auto_read);
    update(&mut app, Action::AutoToggle);
    assert!(app.auto_read);
    assert!(!app.following);
}

#[test]
fn a_finished_pathless_document_cannot_claim_to_follow() {
    let mut app = reader();
    update(&mut app, Action::FollowToggle);
    assert!(!app.following);
    assert!(app.note.as_deref().unwrap().contains("nothing"));
}

#[test]
fn opening_another_document_stops_automatic_motion() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("other.md");
    std::fs::write(&file, source(80)).unwrap();
    for follow in [false, true] {
        let mut app = reader();
        app.streaming = true;
        app.path = "(stdin)".into();
        app.piped = Some(source(60));
        update(
            &mut app,
            if follow {
                Action::FollowToggle
            } else {
                Action::AutoToggle
            },
        );
        app.open_path(&file).unwrap();
        assert!(!app.following);
        assert!(!app.auto_read);
    }
}

#[test]
fn jumping_to_an_annotation_stops_automatic_motion() {
    for automatic in [Action::FollowToggle, Action::AutoToggle] {
        for through_list in [false, true] {
            let mut app = reader();
            app.streaming = true;
            app.path = "(stdin)".into();
            app.piped = Some(source(60));
            update(&mut app, Action::HighlightAdd);
            assert_eq!(app.notes.entries.len(), 1);
            update(&mut app, automatic);
            if through_list {
                update(&mut app, Action::NotesToggle);
                update(&mut app, Action::NoteJump);
            } else {
                update(&mut app, Action::NoteNext);
            }
            assert!(!app.following, "{automatic:?}, list={through_list}");
            assert!(!app.auto_read, "{automatic:?}, list={through_list}");
        }
    }
}

#[test]
fn a_background_pipe_does_not_make_the_current_document_a_live_pipe() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("other.md");
    std::fs::write(&file, source(80)).unwrap();
    for generated in [false, true] {
        let mut app = reader();
        app.path = "(stdin)".into();
        app.streaming = true;
        app.piped = Some(source(60));
        update(&mut app, Action::FollowToggle);
        if generated {
            app.open_results(dir.path(), "missing", &[]);
        } else {
            app.open_welcome();
        }
        assert!(!app.following, "changing documents stops following");
        assert!(!app.can_follow(), "the background pipe is not this page");
        assert_ne!(carrel::footer::of(&app).word, "arriving");
        update(&mut app, Action::GoToEnd);
        assert!(!app.following);
        app.open_path(&file).unwrap();
        assert!(app.can_follow(), "the current file can be watched");
        assert_ne!(carrel::footer::of(&app).word, "arriving");
        update(&mut app, Action::GoToEnd);
        assert!(!app.following, "G follows automatically only in a pipe");
    }
}
