//! Notes survive document changes without ever modifying the source document.
use carrel::{
    action::{Action, Direction, NoteKey, SearchKey, Span},
    app::{App, Outcome, update},
    marginalia,
};
use carrel_core::Document;
use tempfile::TempDir;

const SOURCE: &str = "# Reading\n\nA unique café quote to remember.\n\nAnother paragraph.\n";

fn reader() -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("reading.md");
    std::fs::write(&file, SOURCE).unwrap();
    let mut app = App::new("reading.md".into(), Document::parse(SOURCE), 70, 20);
    app.state_dir = Some(dir.path().join("state"));
    app.config_dir = Some(dir.path().join("config"));
    app.open_path(&file).unwrap();
    (dir, app)
}

fn select(app: &mut App, quote: &str) {
    let start = u32::try_from(app.doc.text.find(quote).unwrap()).unwrap();
    app.selection = Some(start..start + u32::try_from(quote.len()).unwrap());
}

fn input(app: &mut App, text: &str) {
    for c in text.chars() {
        update(
            app,
            Action::NoteInput(if c == '\n' {
                NoteKey::Newline
            } else {
                NoteKey::Char(c)
            }),
        );
    }
}

#[test]
fn selection_highlight_persists_separately_and_deduplicates() {
    let (_dir, mut app) = reader();
    select(&mut app, "café quote");
    let selection = app.selection.clone();
    update(&mut app, Action::HighlightAdd);
    update(&mut app, Action::HighlightAdd);
    assert_eq!(app.notes.entries.len(), 1);
    assert_eq!(app.notes.entries[0].range, selection);
    let loaded = marginalia::load_in(
        app.state_dir.as_deref().unwrap(),
        app.file.as_deref().unwrap(),
        &app.doc.text,
    )
    .unwrap();
    assert_eq!(loaded, app.notes.entries);
    assert_eq!(
        std::fs::read_to_string(app.file.as_ref().unwrap()).unwrap(),
        SOURCE
    );
}

#[test]
fn multiline_unicode_note_edits_by_grapheme_and_cancel_preserves_saved_note() {
    let (_dir, mut app) = reader();
    select(&mut app, "café quote");
    update(&mut app, Action::NoteEdit);
    input(&mut app, "first\n👩‍💻é");
    update(&mut app, Action::NoteInput(NoteKey::Backspace));
    update(&mut app, Action::NoteInput(NoteKey::Left));
    update(&mut app, Action::NoteInput(NoteKey::Delete));
    input(&mut app, "界");
    update(&mut app, Action::NoteInput(NoteKey::Save));
    assert!(app.notes.draft.is_none());
    assert_eq!(app.notes.entries[0].note, "first\n界");
    update(&mut app, Action::NoteEdit);
    input(&mut app, " discarded");
    update(&mut app, Action::NoteInput(NoteKey::Cancel));
    assert_eq!(app.notes.entries[0].note, "first\n界");
    assert_eq!(
        std::fs::read_to_string(app.file.as_ref().unwrap()).unwrap(),
        SOURCE
    );
}

#[test]
fn inserting_before_a_combining_mark_keeps_the_cursor_on_a_grapheme_boundary() {
    let (_dir, mut app) = reader();
    select(&mut app, "café quote");
    update(&mut app, Action::NoteEdit);
    input(&mut app, "\u{301}");
    update(&mut app, Action::NoteInput(NoteKey::Home));
    input(&mut app, "a");
    let draft = app.notes.draft.as_ref().unwrap();
    assert_eq!(draft.text, "a\u{301}");
    assert_eq!(draft.cursor, draft.text.len());
    update(&mut app, Action::NoteInput(NoteKey::Backspace));
    assert!(app.notes.draft.as_ref().unwrap().text.is_empty());
}

#[test]
fn reload_reanchors_unique_quotes_and_preserves_missing_notes() {
    let (_dir, mut app) = reader();
    select(&mut app, "café quote");
    update(&mut app, Action::NoteEdit);
    input(&mut app, "Remember this");
    update(&mut app, Action::NoteInput(NoteKey::Save));
    let old = app.notes.entries[0].range.clone();
    app.reload_from(&format!("Inserted introduction.\n\n{SOURCE}"));
    let entry = &app.notes.entries[0];
    assert_ne!(entry.range, old);
    let range = entry.range.clone().unwrap();
    assert_eq!(
        &app.doc.text[range.start as usize..range.end as usize],
        "café quote"
    );
    app.reload_from("The quote was removed.");
    assert_eq!(app.notes.entries[0].range, None);
    assert_eq!(app.notes.entries[0].note, "Remember this");
    update(&mut app, Action::NotesToggle);
    update(&mut app, Action::NoteJump);
    assert!(app.notes.pane.is_some());
    assert!(
        app.note
            .as_deref()
            .unwrap()
            .contains("missing or ambiguous")
    );
    app.reload_from(SOURCE);
    assert!(app.notes.entries[0].range.is_some());
}

#[test]
fn reflow_preserves_search_and_annotation_byte_coordinates() {
    let (_dir, mut app) = reader();
    select(&mut app, "café quote");
    update(&mut app, Action::HighlightAdd);
    let entries = app.notes.entries.clone();
    update(&mut app, Action::SearchOpen(Direction::Forward));
    for c in "café".chars() {
        update(&mut app, Action::SearchKey(SearchKey::Char(c)));
    }
    update(&mut app, Action::SearchKey(SearchKey::Accept));
    let ranges = app.matches.as_ref().unwrap().ranges.clone();
    for width in [12, 120, 24, 70] {
        app.on_resize(width, 20);
        assert_eq!(app.notes.entries, entries);
        assert_eq!(app.matches.as_ref().unwrap().ranges, ranges);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, 20)).unwrap();
        terminal
            .draw(|frame| carrel::render::draw(frame, &app))
            .unwrap();
    }
}

#[test]
fn switching_documents_loads_only_their_own_notes() {
    let (dir, mut app) = reader();
    let first = app.file.clone().unwrap();
    select(&mut app, "café quote");
    update(&mut app, Action::HighlightAdd);
    let second = dir.path().join("second.md");
    std::fs::write(&second, "Second document.").unwrap();
    app.open_path(&second).unwrap();
    assert!(app.notes.entries.is_empty());
    select(&mut app, "Second");
    update(&mut app, Action::HighlightAdd);
    app.open_path(&first).unwrap();
    assert_eq!(app.notes.entries.len(), 1);
    assert_eq!(app.notes.entries[0].quote, "café quote");
    app.open_path(&second).unwrap();
    assert_eq!(app.notes.entries[0].quote, "Second");
}

#[test]
fn pane_and_editor_prevent_background_navigation() {
    let (_dir, mut app) = reader();
    select(&mut app, "café quote");
    update(&mut app, Action::HighlightAdd);
    let anchor = app.view.anchor;
    let file = app.file.clone();
    update(&mut app, Action::NotesToggle);
    for action in [Action::Scroll(Span::Page, 1), Action::GoHome, Action::Back] {
        assert_eq!(update(&mut app, action), Outcome::Idle);
    }
    update(&mut app, Action::NoteEdit);
    input(&mut app, "draft");
    update(&mut app, Action::Scroll(Span::Page, 1));
    update(&mut app, Action::GoHome);
    assert_eq!(app.notes.draft.as_ref().unwrap().text, "draft");
    assert_eq!(app.view.anchor, anchor);
    assert_eq!(app.file, file);
    assert!(!app.is_home());
}

#[test]
fn failed_save_preserves_draft_and_existing_entries() {
    let (dir, mut app) = reader();
    select(&mut app, "café quote");
    update(&mut app, Action::HighlightAdd);
    let entries = app.notes.entries.clone();
    update(&mut app, Action::NoteEdit);
    input(&mut app, "unsaved note");
    let blocked = dir.path().join("blocked");
    std::fs::write(&blocked, "not a directory").unwrap();
    app.state_dir = Some(blocked);
    update(&mut app, Action::NoteInput(NoteKey::Save));
    assert_eq!(app.notes.entries, entries);
    assert_eq!(app.notes.draft.as_ref().unwrap().text, "unsaved note");
    assert!(
        app.note
            .as_deref()
            .unwrap()
            .contains("Could not save notes")
    );
    app.state_dir = Some(dir.path().join("state"));
    update(&mut app, Action::NoteInput(NoteKey::Save));
    assert!(app.notes.draft.is_none());
    assert_eq!(app.notes.entries[0].note, "unsaved note");
}

#[test]
fn export_reports_success_and_failure_without_losing_entries() {
    let (dir, mut app) = reader();
    select(&mut app, "café quote");
    update(&mut app, Action::NoteEdit);
    input(&mut app, "My thought\nSecond line");
    update(&mut app, Action::NoteInput(NoteKey::Save));
    let entries = app.notes.entries.clone();
    update(&mut app, Action::NotesExport);
    let export = std::fs::read_dir(dir.path().join("state/marginalia"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|ext| ext == "md"))
        .unwrap();
    let markdown = std::fs::read_to_string(export).unwrap();
    assert!(markdown.contains("> café quote"));
    assert!(markdown.contains("My thought\nSecond line"));
    assert!(app.note.as_deref().unwrap().contains("Notes exported"));
    let blocked = dir.path().join("blocked");
    std::fs::write(&blocked, "not a directory").unwrap();
    app.state_dir = Some(blocked);
    update(&mut app, Action::NotesExport);
    assert!(
        app.note
            .as_deref()
            .unwrap()
            .contains("Could not export notes")
    );
    assert_eq!(app.notes.entries, entries);
    assert_eq!(
        std::fs::read_to_string(app.file.as_ref().unwrap()).unwrap(),
        SOURCE
    );
}

#[test]
fn unreadable_sidecar_is_never_replaced_by_new_annotations() {
    let (dir, mut app) = reader();
    select(&mut app, "café quote");
    update(&mut app, Action::HighlightAdd);
    let sidecar = std::fs::read_dir(dir.path().join("state/marginalia"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    std::fs::write(&sidecar, "valuable but malformed data").unwrap();
    let file = app.file.clone().unwrap();
    app.open_path(&file).unwrap();
    assert!(app.notes.load_error.is_some());
    select(&mut app, "Another");
    update(&mut app, Action::HighlightAdd);
    update(&mut app, Action::NoteEdit);
    assert!(app.notes.draft.is_none());
    assert!(app.notes.entries.is_empty());
    assert_eq!(
        std::fs::read_to_string(sidecar).unwrap(),
        "valuable but malformed data"
    );
}

#[test]
fn deleting_a_selected_note_persists_and_clamps_the_pane_cursor() {
    let (_dir, mut app) = reader();
    for quote in ["café quote", "Another"] {
        select(&mut app, quote);
        update(&mut app, Action::HighlightAdd);
    }
    update(&mut app, Action::NotesToggle);
    update(&mut app, Action::NoteMove(i32::MAX));
    assert_eq!(app.notes.pane, Some(1));
    update(&mut app, Action::NoteDelete);
    assert_eq!(app.notes.pane, Some(0));
    assert_eq!(app.notes.entries[0].quote, "café quote");
    let file = app.file.clone().unwrap();
    app.open_path(&file).unwrap();
    assert_eq!(app.notes.entries.len(), 1);
    update(&mut app, Action::NotesToggle);
    update(&mut app, Action::NoteDelete);
    update(&mut app, Action::NoteDelete);
    assert_eq!(app.notes.pane, Some(0));
    app.open_path(&file).unwrap();
    assert!(app.notes.entries.is_empty());
}

#[test]
fn notes_overlay_has_clickable_controls_and_owns_empty_space_at_all_sizes() {
    let (_dir, mut app) = reader();
    select(&mut app, "café quote");
    update(&mut app, Action::HighlightAdd);
    update(&mut app, Action::NotesToggle);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(70, 20)).unwrap();
    let mut painted = carrel::render::Painted::default();
    terminal
        .draw(|frame| {
            carrel::render::draw_full(
                frame,
                &app,
                &mut painted,
                &mut std::collections::HashMap::new(),
            );
        })
        .unwrap();
    assert_eq!(painted.targets.hit(0, 0).unwrap().action, Action::Absorb);
    let edit = painted
        .targets
        .as_slice()
        .iter()
        .find(|target| target.action == Action::NoteEdit)
        .unwrap()
        .action;
    update(&mut app, edit);
    input(&mut app, "A visible draft");
    for (width, height) in [(70, 20), (18, 6), (1, 1)] {
        app.on_resize(width, height);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                carrel::render::draw_full(
                    frame,
                    &app,
                    &mut painted,
                    &mut std::collections::HashMap::new(),
                );
            })
            .unwrap();
        assert_eq!(app.notes.draft.as_ref().unwrap().text, "A visible draft");
        assert!(painted.targets.hit(0, 0).is_some(), "{width} × {height}");
    }
}

#[test]
fn empty_documents_decline_annotations_without_panicking() {
    let (_dir, mut app) = reader();
    app.reload_from("");
    for action in [
        Action::HighlightAdd,
        Action::NoteEdit,
        Action::HighlightAt(0),
        Action::NoteAt(0),
    ] {
        update(&mut app, action);
        assert!(app.notes.entries.is_empty());
        assert!(app.notes.draft.is_none());
    }
}

#[test]
fn piped_notes_survive_a_link_round_trip_without_inheriting_file_notes() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target.md");
    std::fs::write(&target, "Target document.").unwrap();
    let source = format!("Piped passage.\n\n[go]({})", target.display());
    let mut app = App::new("(stdin)".into(), Document::parse(&source), 70, 20);
    app.state_dir = Some(dir.path().join("state"));
    app.config_dir = Some(dir.path().join("config"));
    app.piped = Some(source);
    select(&mut app, "Piped passage");
    update(&mut app, Action::NoteEdit);
    input(&mut app, "Session note");
    update(&mut app, Action::NoteInput(NoteKey::Save));
    let piped_entries = app.notes.entries.clone();
    update(&mut app, Action::LinkOpen(0));
    assert_eq!(app.file.as_deref(), Some(target.as_path()));
    assert!(app.notes.entries.is_empty());
    select(&mut app, "Target");
    update(&mut app, Action::HighlightAdd);
    update(&mut app, Action::Back);
    assert_eq!(app.file, None);
    assert_eq!(app.notes.entries, piped_entries);
    update(&mut app, Action::LinkOpen(0));
    assert_eq!(app.notes.entries.len(), 1);
    assert_eq!(app.notes.entries[0].quote, "Target");
    update(&mut app, Action::Back);
    assert_eq!(app.notes.entries, piped_entries);
}

#[test]
fn next_note_advances_past_midrow_quotes_and_wraps() {
    let (_dir, mut app) = reader();
    let filler = "A filler paragraph.\n\n".repeat(12);
    app.reload_from(&format!(
        "Start.\n\n{filler}Prefix first quote.\n\n{filler}Prefix second quote.\n\n{filler}"
    ));
    for quote in ["first quote", "second quote"] {
        select(&mut app, quote);
        update(&mut app, Action::HighlightAdd);
    }
    app.selection = None;
    update(&mut app, Action::NoteNext);
    let first = app.view.anchor;
    assert!(first > 0);
    assert!(first < app.notes.entries[0].range.as_ref().unwrap().start);
    update(&mut app, Action::NoteNext);
    let second = app.view.anchor;
    assert!(
        second > first,
        "the next note must advance beyond the same wrapped row"
    );
    update(&mut app, Action::NoteNext);
    assert_eq!(app.view.anchor, first);
}

#[test]
fn bracketed_paste_preserves_newlines_without_saving_or_executing_keys() {
    let (_dir, mut app) = reader();
    select(&mut app, "café quote");
    update(&mut app, Action::NoteEdit);
    carrel::annotation_state::paste(&mut app, "first\r\nq\nV\t界\u{1b}");
    assert!(app.notes.entries.is_empty());
    assert_eq!(app.notes.draft.as_ref().unwrap().text, "first\nq\nV    界");
    carrel::annotation_state::paste(&mut app, &"x".repeat(70_000));
    assert_eq!(app.notes.draft.as_ref().unwrap().text, "first\nq\nV    界");
    update(&mut app, Action::NoteInput(NoteKey::Save));
    assert_eq!(app.notes.entries[0].note, "first\nq\nV    界");
}
