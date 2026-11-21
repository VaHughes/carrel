//! Two readers on one document share its bookmarks and notes, and a save
//! that did not reach the disk says so.
//!
//! Each reader used to write the whole list it held, so the second to save
//! erased what the first had added — silently, and for good once both had
//! quit. A save now merges with the file as it is, against what the session
//! last saw there; these tests are two `App`s over one real state directory,
//! in every order the two can act in.
use carrel::{
    action::{Action, NoteKey, Where},
    app::{App, update},
    marginalia, state,
};
use carrel_core::Document;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Four paragraphs to mark, and enough below them that each can be
/// scrolled to the top row, which is the block `m` marks.
fn source() -> String {
    let mut s = String::from(
        "# Shared\n\nFirst paragraph here.\n\nSecond paragraph here.\n\n\
         Third paragraph here.\n\nFourth paragraph here.\n",
    );
    for i in 0..40 {
        let _ = write!(s, "\nFiller {i}.\n");
    }
    s
}

fn desk() -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("shared.md");
    std::fs::write(&file, source()).unwrap();
    (dir, file)
}

/// A reader on `file`, as a second terminal would start one.
fn open(dir: &Path, file: &Path) -> App {
    let mut app = App::new("shared.md".into(), Document::parse(&source()), 70, 20);
    app.state_dir = Some(dir.join("state"));
    app.open_path(file).unwrap();
    app
}

fn start_of(app: &App, text: &str) -> u32 {
    u32::try_from(app.doc.text.find(text).unwrap()).unwrap()
}

/// Press `m` with `text`'s block at the top of the view.
fn toggle(app: &mut App, text: &str) {
    let at = start_of(app, text);
    app.reveal_byte(at, app.text_h(), Where::Top);
    update(app, Action::MarkToggle);
}

/// The bookmarks a reader opening the document now would see, as the first
/// word of each marked block.
fn on_disk(dir: &Path, file: &Path) -> Vec<String> {
    let app = open(dir, file);
    words(&app)
}

fn words(app: &App) -> Vec<String> {
    app.marks
        .iter()
        .map(|&m| {
            app.doc.text[m as usize..]
                .split_whitespace()
                .next()
                .unwrap()
                .to_string()
        })
        .collect()
}

#[test]
fn two_readers_bookmarks_both_survive_in_either_order() {
    for a_first in [true, false] {
        let (dir, file) = desk();
        let mut a = open(dir.path(), &file);
        let mut b = open(dir.path(), &file);
        if a_first {
            toggle(&mut a, "First");
            toggle(&mut b, "Third");
        } else {
            toggle(&mut b, "Third");
            toggle(&mut a, "First");
        }
        assert_eq!(on_disk(dir.path(), &file), ["First", "Third"]);
        // Quitting writes no bookmarks, in either order.
        let (mut one, mut two) = if a_first { (a, b) } else { (b, a) };
        update(&mut one, Action::Quit);
        update(&mut two, Action::Quit);
        assert_eq!(on_disk(dir.path(), &file), ["First", "Third"]);
    }
}

#[test]
fn a_save_shows_the_other_readers_bookmarks_and_counts_them() {
    let (dir, file) = desk();
    let mut a = open(dir.path(), &file);
    let mut b = open(dir.path(), &file);
    toggle(&mut a, "First");
    toggle(&mut b, "Third");
    assert_eq!(words(&b), ["First", "Third"]);
    assert_eq!(b.note.as_deref(), Some("bookmark 2 of 2"));
    // A has not saved since, so it has not looked; its next save does.
    assert_eq!(words(&a), ["First"]);
    toggle(&mut a, "Fourth");
    assert_eq!(words(&a), ["First", "Third", "Fourth"]);
    assert_eq!(a.note.as_deref(), Some("bookmark 3 of 3"));
}

#[test]
fn a_bookmark_cleared_by_one_reader_is_not_brought_back_by_the_other() {
    let (dir, file) = desk();
    let mut seed = open(dir.path(), &file);
    toggle(&mut seed, "First");
    toggle(&mut seed, "Second");
    // Both load [First, Second].
    let mut a = open(dir.path(), &file);
    let mut b = open(dir.path(), &file);
    toggle(&mut a, "First"); // A clears First
    assert_eq!(on_disk(dir.path(), &file), ["Second"]);
    toggle(&mut b, "Third"); // B, still holding First, adds Third
    assert_eq!(on_disk(dir.path(), &file), ["Second", "Third"]);
    assert_eq!(words(&b), ["Second", "Third"], "and B stops showing it");
    // The other way round: B clears Second, A — which still holds it —
    // adds Fourth.
    toggle(&mut b, "Second");
    toggle(&mut a, "Fourth");
    assert_eq!(on_disk(dir.path(), &file), ["Third", "Fourth"]);
}

#[test]
fn both_readers_clearing_and_adding_at_once_keep_every_change() {
    let (dir, file) = desk();
    let mut seed = open(dir.path(), &file);
    toggle(&mut seed, "First");
    toggle(&mut seed, "Second");
    let mut a = open(dir.path(), &file);
    let mut b = open(dir.path(), &file);
    toggle(&mut a, "First"); // A: -First
    toggle(&mut b, "Second"); // B: -Second
    toggle(&mut a, "Third"); // A: +Third
    toggle(&mut b, "Fourth"); // B: +Fourth
    assert_eq!(on_disk(dir.path(), &file), ["Third", "Fourth"]);
    // The same block set by both is one bookmark, and one press clears it.
    toggle(&mut a, "Shared");
    toggle(&mut b, "Third"); // B syncs: sees Shared; clears Third
    toggle(&mut b, "Shared");
    assert_eq!(on_disk(dir.path(), &file), ["Fourth"]);
}

fn note(app: &mut App, quote: &str, text: &str) {
    let start = start_of(app, quote);
    app.selection = Some(start..start + u32::try_from(quote.len()).unwrap());
    update(app, Action::NoteEdit);
    update(app, Action::NoteInput(NoteKey::DeleteToLineStart));
    for c in text.chars() {
        update(app, Action::NoteInput(NoteKey::Char(c)));
    }
    update(app, Action::NoteInput(NoteKey::Save));
    assert!(app.notes.draft.is_none(), "{:?}", app.note);
    app.selection = None;
}

/// `quote=note` for every note a reader opening the document now would see.
fn notes_on_disk(dir: &Path, file: &Path) -> Vec<String> {
    let text = Document::parse(&source()).text;
    let mut v: Vec<String> = marginalia::load_in(&dir.join("state"), file, &text)
        .unwrap()
        .into_iter()
        .map(|a| {
            assert!(a.range.is_some(), "{a:?}");
            format!("{}={}", a.quote, a.note)
        })
        .collect();
    v.sort();
    v
}

#[test]
fn two_readers_notes_both_survive_in_either_order() {
    for a_first in [true, false] {
        let (dir, file) = desk();
        let mut a = open(dir.path(), &file);
        let mut b = open(dir.path(), &file);
        if a_first {
            note(&mut a, "First", "A1");
            note(&mut b, "Second", "B1");
        } else {
            note(&mut b, "Second", "B1");
            note(&mut a, "First", "A1");
        }
        assert_eq!(notes_on_disk(dir.path(), &file), ["First=A1", "Second=B1"]);
        note(&mut a, "Third", "A2");
        note(&mut b, "Fourth", "B2");
        let all = ["First=A1", "Fourth=B2", "Second=B1", "Third=A2"];
        assert_eq!(notes_on_disk(dir.path(), &file), all);
        // The reader that saved last shows all four, each placed.
        assert_eq!(b.notes.entries.len(), 4);
        assert!(b.notes.entries.iter().all(|e| e.range.is_some()));
        let (mut one, mut two) = if a_first { (a, b) } else { (b, a) };
        update(&mut one, Action::Quit);
        update(&mut two, Action::Quit);
        assert_eq!(notes_on_disk(dir.path(), &file), all);
    }
}

fn delete(app: &mut App, quote: &str) {
    if app.notes.pane.is_none() {
        update(app, Action::NotesToggle);
    }
    let i = app
        .notes
        .entries
        .iter()
        .position(|e| e.quote == quote)
        .unwrap();
    update(app, Action::NoteSelect(u32::try_from(i).unwrap()));
    update(app, Action::NoteDelete);
    update(app, Action::NotesToggle);
}

#[test]
fn a_note_deleted_by_one_reader_stays_deleted_and_an_edit_outlives_a_delete() {
    let (dir, file) = desk();
    let mut seed = open(dir.path(), &file);
    note(&mut seed, "First", "one");
    note(&mut seed, "Second", "two");
    note(&mut seed, "Third", "three");
    let mut a = open(dir.path(), &file);
    let mut b = open(dir.path(), &file);
    // A deletes First; B, which still holds it, saves something else.
    delete(&mut a, "First");
    note(&mut b, "Fourth", "four");
    assert_eq!(
        notes_on_disk(dir.path(), &file),
        ["Fourth=four", "Second=two", "Third=three"]
    );
    assert!(b.notes.entries.iter().all(|e| e.quote != "First"));
    // B rewrites Second while A deletes it: what B wrote is not lost.
    note(&mut b, "Second", "two, revised");
    delete(&mut a, "Second");
    assert!(
        notes_on_disk(dir.path(), &file).contains(&"Second=two, revised".to_string()),
        "{:?}",
        notes_on_disk(dir.path(), &file)
    );
    // A rewrites Third after B deleted it: what A wrote is not lost either.
    delete(&mut b, "Third");
    note(&mut a, "Third", "three, revised");
    assert!(notes_on_disk(dir.path(), &file).contains(&"Third=three, revised".to_string()));
}

#[test]
fn two_readers_writing_the_same_note_keep_both_texts() {
    let (dir, file) = desk();
    let mut seed = open(dir.path(), &file);
    note(&mut seed, "First", "seed");
    let mut a = open(dir.path(), &file);
    let mut b = open(dir.path(), &file);
    note(&mut a, "First", "from A");
    note(&mut b, "First", "from B");
    assert_eq!(notes_on_disk(dir.path(), &file), ["First=from B\n\nfrom A"]);
    // Said once: the next save does not join them again.
    note(&mut b, "Second", "more");
    assert_eq!(
        notes_on_disk(dir.path(), &file),
        ["First=from B\n\nfrom A", "Second=more"]
    );
    // And A, saving an unrelated note, takes B's text for the first.
    note(&mut a, "Third", "third");
    assert_eq!(a.notes.entries.len(), 3);
    assert_eq!(
        notes_on_disk(dir.path(), &file),
        ["First=from B\n\nfrom A", "Second=more", "Third=third"]
    );
}

/// A directory nothing can be created in — or `None` where that cannot be
/// arranged (running as root, a filesystem without modes).
#[cfg(unix)]
fn read_only(dir: &Path) -> Option<impl Drop + use<>> {
    use std::os::unix::fs::PermissionsExt as _;
    struct Restore(PathBuf);
    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o700));
        }
    }
    std::fs::create_dir_all(dir).unwrap();
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o500)).unwrap();
    let guard = Restore(dir.to_path_buf());
    if std::fs::write(dir.join("probe"), "x").is_ok() {
        eprintln!("skipping: this user can write to a read-only directory");
        return None;
    }
    Some(guard)
}

#[cfg(unix)]
#[test]
fn a_bookmark_that_could_not_be_saved_says_so_and_stays_for_the_session() {
    let (dir, file) = desk();
    let mut app = open(dir.path(), &file);
    toggle(&mut app, "First");
    assert_eq!(app.note.as_deref(), Some("bookmark 1 of 1"));
    {
        let Some(_guard) = read_only(&dir.path().join("state")) else {
            return;
        };
        // Setting one.
        toggle(&mut app, "Third");
        let said = app.note.clone().unwrap();
        assert!(said.starts_with("bookmark 2 of 2"), "{said}");
        assert!(said.contains("not saved"), "{said}");
        assert_eq!(words(&app), ["First", "Third"], "still set in this reader");
        // Clearing one.
        toggle(&mut app, "First");
        let said = app.note.clone().unwrap();
        assert!(said.starts_with("bookmark cleared"), "{said}");
        assert!(said.contains("not saved"), "{said}");
        assert_eq!(words(&app), ["Third"]);
        // Nothing reached the disk, and nothing on it was harmed.
        assert_eq!(on_disk(dir.path(), &file), ["First"]);
    }
    // Writable again: the next save carries everything that was held back.
    toggle(&mut app, "Fourth");
    assert_eq!(app.note.as_deref(), Some("bookmark 2 of 2"));
    assert_eq!(on_disk(dir.path(), &file), ["Third", "Fourth"]);
}

#[cfg(unix)]
#[test]
fn a_highlight_that_could_not_be_saved_says_so() {
    let (dir, file) = desk();
    let mut app = open(dir.path(), &file);
    let Some(_guard) = read_only(&dir.path().join("state")) else {
        return;
    };
    let start = start_of(&app, "Second");
    app.selection = Some(start..start + 6);
    update(&mut app, Action::HighlightAdd);
    let said = app.note.clone().unwrap();
    assert!(said.contains("Could not save notes"), "{said}");
}

/// The state layer's own half: two handles, real files, no reader.
#[test]
fn syncing_marks_merges_with_the_file_under_one_read() {
    let (dir, file) = desk();
    let state_dir = dir.path().join("state");
    let p = |n| state::Place::Legacy(n);
    state::save_marks_in(&state_dir, &file, &[p(1), p(2)]).unwrap();
    let base = state::load_marks_in(&state_dir, &file);
    // Another session adds 3 and removes 1.
    state::save_marks_in(&state_dir, &file, &[p(2), p(3)]).unwrap();
    // This one removed 2 and added 4.
    let mine = [p(1), p(4)];
    let written =
        state::sync_marks_in(&state_dir, &file, |disk| state::merge3(&base, &mine, &disk)).unwrap();
    assert_eq!(written, [p(4), p(3)]);
    assert_eq!(state::load_marks_in(&state_dir, &file), [p(4), p(3)]);
}

/// A document opened while it was empty — an agent had just truncated it —
/// has nowhere to show its bookmarks. They come back with the text, so the
/// next `m` adds to them; it used to find none in memory and save that.
#[test]
fn bookmarks_of_a_file_opened_empty_come_back_with_its_text() {
    let (dir, file) = desk();
    let mut seed = open(dir.path(), &file);
    toggle(&mut seed, "First");
    toggle(&mut seed, "Second");
    std::fs::write(&file, "").unwrap();
    let mut app = open(dir.path(), &file);
    assert!(app.marks.is_empty(), "nothing to mark in an empty document");
    app.reload().unwrap(); // a second look, still empty
    std::fs::write(&file, source()).unwrap();
    app.reload().unwrap();
    assert_eq!(words(&app), ["First", "Second"]);
    toggle(&mut app, "Third");
    assert_eq!(on_disk(dir.path(), &file), ["First", "Second", "Third"]);
}

/// The same while reading: emptied, looked at twice, rewritten, then `m`.
#[test]
fn bookmarks_survive_on_disk_when_the_open_file_is_emptied_and_rewritten() {
    let (dir, file) = desk();
    let mut app = open(dir.path(), &file);
    toggle(&mut app, "First");
    toggle(&mut app, "Second");
    std::fs::write(&file, "").unwrap();
    app.reload().unwrap();
    app.reload().unwrap();
    let saved = state::load_marks_in(&dir.path().join("state"), &file);
    assert_eq!(saved.len(), 2, "an empty read saves nothing");
    std::fs::write(&file, source()).unwrap();
    app.reload().unwrap();
    toggle(&mut app, "Third");
    assert_eq!(on_disk(dir.path(), &file), ["First", "Second", "Third"]);
}

/// A bookmark on the last block is one bookmark. Its saved place ends in
/// the zero hash (nothing follows the last block), and a bookmarks line
/// was split inside the place, so that half read back as a bookmark on the
/// document's first block.
#[test]
fn a_bookmark_on_the_last_block_does_not_also_mark_the_first() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("last.md");
    // The last block is taller than the window, so `G` puts it on the top
    // row, which is the block `m` marks.
    let src = format!(
        "# Title\n\nIntro paragraph.\n\nSecond paragraph.\n\n```\n{}```\n",
        "line\n".repeat(60)
    );
    std::fs::write(&file, &src).unwrap();
    let mut app = App::new("last.md".into(), Document::parse(&src), 70, 20);
    app.state_dir = Some(dir.path().join("state"));
    app.open_path(&file).unwrap();
    update(&mut app, Action::GoToEnd);
    update(&mut app, Action::MarkToggle);
    let last = app
        .doc
        .node_for_block(carrel_core::BlockIdx(app.doc.block_count() as u32 - 1))
        .doc
        .start;
    assert_eq!(app.marks, [last], "the fixture marks the last block");
    assert_eq!(app.note.as_deref(), Some("bookmark 1 of 1"));
    toggle(&mut app, "Intro");
    assert_eq!(app.note.as_deref(), Some("bookmark 1 of 2"));
    assert_eq!(app.marks, [start_of(&app, "Intro"), last]);
    let mut again = App::new("last.md".into(), Document::parse(&src), 70, 20);
    again.state_dir = Some(dir.path().join("state"));
    again.open_path(&file).unwrap();
    assert_eq!(again.marks, app.marks);
    // Set and cleared: the document's line is gone, not left holding half
    // a place for good.
    toggle(&mut app, "Intro");
    update(&mut app, Action::GoToEnd);
    update(&mut app, Action::MarkToggle);
    assert!(app.marks.is_empty());
    let left = std::fs::read_to_string(dir.path().join("state/bookmarks")).unwrap();
    assert_eq!(left, "");
}

/// A reader still showing the file as it was (the other has seen it
/// rewritten) saves its own bookmark and leaves the other's exactly as
/// they were written — it used to rewrite every one as a place in its own,
/// older text, which moved a bookmark on a new paragraph to another one.
#[test]
fn a_reader_showing_older_text_leaves_the_others_bookmarks_as_written() {
    let (dir, file) = desk();
    let mut a = open(dir.path(), &file); // v1
    let v2 = source().replace(
        "# Shared\n",
        "# Shared\n\nBrand new intro paragraph.\n\nAnother new one.\n",
    ) + "\nNewly appended conclusion.\n";
    std::fs::write(&file, &v2).unwrap();
    let open_v2 = || {
        let mut q = App::new("shared.md".into(), Document::parse(&v2), 70, 20);
        q.state_dir = Some(dir.path().join("state"));
        q.open_path(&file).unwrap();
        q
    };
    let mut b = open_v2();
    toggle(&mut b, "Brand new intro");
    toggle(&mut b, "Third");
    let line = || std::fs::read_to_string(dir.path().join("state/bookmarks")).unwrap();
    let before = line();
    // A has not reloaded yet.
    toggle(&mut a, "Second");
    assert!(
        line().trim_end().starts_with(before.trim_end()),
        "B's places are carried through as written:\n{before}\n{}",
        line()
    );
    assert_eq!(words(&open_v2()), ["Brand", "Second", "Third"]);
    // And B, saving again, neither loses its own nor drops A's.
    toggle(&mut b, "Fourth");
    assert_eq!(words(&open_v2()), ["Brand", "Second", "Third", "Fourth"]);
    // A catches up, and its own next save keeps all of them too.
    a.reload().unwrap();
    assert_eq!(
        words(&a),
        ["Brand", "Second", "Third"],
        "placed, not carried"
    );
    toggle(&mut a, "First");
    assert_eq!(
        words(&open_v2()),
        ["Brand", "First", "Second", "Third", "Fourth"]
    );
}

/// A file with nowhere to keep its bookmarks — no home directory to hold
/// state — is a save that failed. It said `bookmark 1 of 1` as if kept. A
/// piped document, which keeps its bookmarks in memory by design, does not
/// complain.
#[test]
fn a_bookmark_with_no_folder_to_keep_it_in_says_so() {
    let src = "# One\n\ntext\n";
    let mut a = App::new("t.md".into(), carrel_core::Document::parse(src), 60, 20);
    a.file = Some("t.md".into());
    update(&mut a, Action::MarkToggle);
    let note = a.note.clone().unwrap_or_default();
    assert!(note.contains("not saved"), "{note}");
    assert_eq!(a.marks.len(), 1, "it is still set for the session");

    let mut piped = App::new("(stdin)".into(), carrel_core::Document::parse(src), 60, 20);
    update(&mut piped, Action::MarkToggle);
    let note = piped.note.clone().unwrap_or_default();
    assert!(!note.contains("not saved"), "{note}");
}

/// A setting changed while the config cannot be written says so, and the
/// settings pane stops reading `saved in …`.
#[cfg(unix)]
#[test]
fn a_setting_that_could_not_be_written_says_so() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("cfg");
    std::fs::create_dir_all(&cfg).unwrap();
    std::fs::set_permissions(&cfg, std::fs::Permissions::from_mode(0o500)).unwrap();
    if std::fs::write(cfg.join("probe"), "x").is_ok() {
        eprintln!("SKIP: this user writes to a read-only folder");
        return;
    }
    let mut a = App::new("t.md".into(), carrel_core::Document::parse("# T\n"), 60, 20);
    a.config_dir = Some(cfg.clone());
    for act in [
        Action::HintsToggle,
        Action::BreadcrumbToggle,
        Action::MeasureStep(1),
    ] {
        a.note = None;
        update(&mut a, act);
        let note = a.note.clone().unwrap_or_default();
        assert!(note.contains("not saved"), "{act:?}: {note}");
        assert!(a.setting_unsaved, "{act:?}");
    }
    // The control: a folder that takes the write says nothing of the kind.
    std::fs::set_permissions(&cfg, std::fs::Permissions::from_mode(0o700)).unwrap();
    a.note = None;
    update(&mut a, Action::HintsToggle);
    assert!(!a.note.clone().unwrap_or_default().contains("not saved"));
    assert!(!a.setting_unsaved);
}
