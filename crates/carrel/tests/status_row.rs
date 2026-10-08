//! The status row as the window narrows.
//!
//! The row degrades one way only: as the window loses a column, something
//! may give way, and nothing that gave way comes back. What gives way does
//! so at a boundary that means something — a whole chip, a whole name on the
//! trail, a whole clause of a note — and a note cut short ends on a word, or
//! says it was cut. These tests sweep every width rather than spot-checking
//! a few, because the defects they guard were both visible only as a
//! sequence: a `…` that blinked in and out as the window shrank, and a note
//! that ended `c goes to the n`.

use std::path::Path;

use carrel::action::{Action, Zone};
use carrel::app::{App, update};
use carrel_core::Document;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;

const NAMES: [&str; 6] = [
    "the-first-document.md",
    "b.md",
    "a-somewhat-longer-third-one.md",
    "four.md",
    "the-fifth.md",
    "here.md",
];

/// A chain of documents, each linking to the next, followed to the end: a
/// trail of five names behind the one being read.
fn long_trail(dir: &Path, cols: u16, rows: u16, depth: usize) -> App {
    for (i, n) in NAMES.iter().enumerate() {
        let next = NAMES
            .get(i + 1)
            .map_or(String::new(), |m| format!("[next]({m})"));
        let body = "a paragraph of words to read. ".repeat(60);
        std::fs::write(dir.join(n), format!("# {n}\n\n{next}\n\n{body}\n")).unwrap();
    }
    let src = std::fs::read_to_string(dir.join(NAMES[0])).unwrap();
    let mut app = App::new(NAMES[0].into(), Document::parse(&src), cols, rows);
    app.file = Some(dir.join(NAMES[0]));
    app.library_root = Some(dir.to_path_buf());
    app.on_resize(cols, rows);
    for _ in 0..depth {
        update(&mut app, Action::LinkOpen(0));
    }
    app
}

fn painted(app: &App, cols: u16, rows: u16) -> (Buffer, Vec<(Action, Zone)>) {
    let mut term = Terminal::new(TestBackend::new(cols, rows)).unwrap();
    let mut p = carrel::render::Painted::default();
    term.draw(|f| {
        carrel::render::draw_full(f, app, &mut p, &mut std::collections::HashMap::new());
    })
    .unwrap();
    let targets = p
        .targets
        .as_slice()
        .iter()
        .map(|t| (t.action, t.zone))
        .collect();
    (term.backend().buffer().clone(), targets)
}

/// The status row: the one the `≡` launcher sits on.
fn status_row(buf: &Buffer, targets: &[(Action, Zone)], cols: u16) -> Option<(u16, String)> {
    let y = targets
        .iter()
        .find(|(a, z)| matches!(a, Action::MenuOpen { .. }) && z.x + 1 == cols)
        .map(|(_, z)| z.y)?;
    Some((y, (0..cols).map(|x| buf[(x, y)].symbol()).collect()))
}

/// What the row shows of `note`: the longest run from where it starts that
/// is still the note — or the note cut with an ellipsis. Whatever follows
/// it on the row (the right end, the launcher) is not the note.
fn shown_note(row: &str, note: &str) -> String {
    let first = note.split(' ').next().unwrap();
    let head: String = first.chars().take(2).collect();
    let Some(at) = row.find(&head) else {
        return String::new();
    };
    let rest = &row[at..];
    let mut best = "";
    for (i, c) in rest.char_indices() {
        let p = &rest[..i + c.len_utf8()];
        if note.starts_with(p) {
            best = p;
        } else if c == '\u{2026}' && note.starts_with(&rest[..i]) {
            best = p;
            break;
        } else {
            break;
        }
    }
    best.trim_end().to_string()
}

#[test]
#[ignore = "a viewer, not a test: DEPTH=n NOTE=... cargo test print_sweep -- --ignored"]
fn print_sweep() {
    let d = tempfile::tempdir().unwrap();
    let depth: usize = std::env::var("DEPTH").map_or(2, |v| v.parse().unwrap());
    let note = std::env::var("NOTE").ok();
    for cols in (10u16..=120).rev() {
        let rows = 20;
        let mut app = long_trail(d.path(), cols, rows, depth);
        app.note.clone_from(&note);
        let (buf, targets) = painted(&app, cols, rows);
        if let Some((_, row)) = status_row(&buf, &targets, cols) {
            println!("{cols:>3} |{row}|");
        }
    }
}

const NOTES: [&str; 6] = [
    "reloaded \u{2014} 7 changes \u{b7} c goes to the next",
    "theme: everforest",
    "resumed where you left off \u{b7} Home for the top",
    "Notes kept for this session only \u{2014} export before leaving.",
    "a document read from a pipe has no folder to link from",
    "could-not-open-an-extraordinarily-long-file-name.md",
];

/// A cut note keeps most of the room it was given: the blank run between
/// its ellipsis and whatever the row shows next is at most half of what
/// it kept (plus the cell of air before the next thing, and one for a wide
/// glyph that did not split). A word boundary bought at any price left
/// `theme\u{2026}` beside eleven empty cells at 18 columns.
fn assert_keeps_the_room(row: &str, shown: &str, ctx: &str) {
    let at = row.find(shown).expect("the note is on the row");
    let gap = row[at + shown.len()..]
        .chars()
        .take_while(|c| *c == ' ')
        .count();
    let kept = carrel_core::display_width(shown) as usize;
    assert!(
        gap <= kept / 2 + 2,
        "kept {kept} cells and left {gap} empty: {ctx}"
    );
}

/// A note cut short is cut where a reader can tell: at a clause, or with
/// an ellipsis saying it was cut — at a word while that keeps most of the
/// room, inside one past that. Never cut in silence, and never longer at a
/// narrower width.
#[test]
fn a_note_gives_way_by_clauses_and_words_and_never_in_silence() {
    let d = tempfile::tempdir().unwrap();
    for note in NOTES {
        for hints in [true, false] {
            for breadcrumb in [true, false] {
                let mut last: Option<String> = None;
                for cols in (10u16..=120).rev() {
                    let rows = 20;
                    let mut app = long_trail(d.path(), cols, rows, 2);
                    app.hints = hints;
                    app.breadcrumb = breadcrumb;
                    app.on_resize(cols, rows);
                    app.note = Some(note.to_string());
                    let (buf, targets) = painted(&app, cols, rows);
                    let Some((_, row)) = status_row(&buf, &targets, cols) else {
                        continue;
                    };
                    let shown = shown_note(&row, note);
                    let ctx = format!("cols={cols} hints={hints} note={note:?}: {row:?}");
                    if shown != note {
                        if let Some(stem) = shown.strip_suffix('\u{2026}') {
                            assert!(note.starts_with(stem), "{ctx}");
                            assert_keeps_the_room(&row, &shown, &ctx);
                        } else {
                            assert!(note.starts_with(&shown), "{ctx}");
                            let after = &note[shown.len()..];
                            assert!(
                                after.starts_with(" \u{b7} ") || after.starts_with(" \u{2014} "),
                                "cut short without saying so, not at a clause: {ctx}"
                            );
                        }
                    }
                    if let Some(prev) = &last {
                        assert!(
                            prev.starts_with(shown.trim_end_matches('\u{2026}')),
                            "a narrower row showed more of the note: {prev:?} then {ctx}"
                        );
                    }
                    last = Some(shown);
                }
            }
        }
    }
}

/// Which names of the trail, which right-end chips, and whether the `…`
/// is up, at every width from wide to narrow. Each may only ever give way:
/// a name or chip absent at one width is absent at every narrower one, and
/// the `…` is up over one unbroken run of widths.
#[test]
fn the_status_row_only_ever_gives_way_as_it_narrows() {
    let d = tempfile::tempdir().unwrap();
    for depth in [1usize, 2, 5] {
        for hints in [true, false] {
            for task_list in [false, true] {
                let mut prev: Option<(Vec<String>, u16)> = None;
                let mut ellipsis_runs = 0;
                let mut ellipsis_was = false;
                for cols in (10u16..=120).rev() {
                    let rows = 20;
                    let mut app = long_trail(d.path(), cols, rows, depth);
                    app.hints = hints;
                    if task_list {
                        app.doc = Document::parse(&format!(
                            "- [ ] a task\n\n{}",
                            "some words to read here. ".repeat(400)
                        ));
                    }
                    app.on_resize(cols, rows);
                    let (buf, targets) = painted(&app, cols, rows);
                    let Some((y, row)) = status_row(&buf, &targets, cols) else {
                        continue;
                    };
                    let ctx = format!("depth={depth} hints={hints} cols={cols}: {row:?}");
                    // Every word-ish piece on the row: names, chips.
                    let pieces: Vec<String> = targets
                        .iter()
                        .filter(|(a, z)| z.y == y && !matches!(a, Action::MenuOpen { .. }))
                        .map(|(a, _)| format!("{a:?}"))
                        .collect();
                    let has_ellipsis = row.contains("\u{2026} \u{203a}");
                    if has_ellipsis && !ellipsis_was {
                        ellipsis_runs += 1;
                    }
                    ellipsis_was = has_ellipsis;
                    assert!(ellipsis_runs <= 1, "the \u{2026} came back: {ctx}");
                    if let Some((before, before_cols)) = &prev {
                        for p in &pieces {
                            assert!(
                                before.contains(p),
                                "{p} is back at {cols} after giving way at {before_cols}: {ctx}"
                            );
                        }
                    }
                    // The right end sits against the launcher: one cell of
                    // air, then `≡`.
                    // The way out is always the last chip standing.
                    if let Some((_, z)) = targets
                        .iter()
                        .find(|(a, z)| z.y == y && *a == Action::CloseFile)
                    {
                        assert_eq!(z.x + z.w, cols - 2, "{ctx}");
                    }
                    prev = Some((pieces, cols));
                }
            }
        }
    }
}

/// The home screen's row: its note gives way at words too.
#[test]
fn the_home_screen_note_is_never_cut_in_silence() {
    let d = tempfile::tempdir().unwrap();
    let note = "scan interrupted \u{2014} showing the cached list";
    let mut last: Option<String> = None;
    for cols in (10u16..=80).rev() {
        let rows = 16;
        let mut app = App::new_home(d.path().into(), vec![], cols, rows);
        app.hints = false;
        app.home_mut().unwrap().note = Some(note.into());
        let (buf, targets) = painted(&app, cols, rows);
        let Some((_, row)) = status_row(&buf, &targets, cols) else {
            continue;
        };
        let shown = shown_note(&row, note);
        let ctx = format!("cols={cols}: {row:?}");
        if shown != note {
            if let Some(stem) = shown.strip_suffix('\u{2026}') {
                assert!(note.starts_with(stem), "{ctx}");
                assert_keeps_the_room(&row, &shown, &ctx);
            } else {
                assert!(note.starts_with(&shown), "{ctx}");
                assert!(
                    note[shown.len()..].starts_with(" \u{2014} "),
                    "cut in silence, not at a clause: {ctx}"
                );
            }
        }
        if let Some(prev) = &last {
            assert!(
                prev.starts_with(shown.trim_end_matches('\u{2026}')),
                "{prev:?} then {ctx}"
            );
        }
        last = Some(shown);
    }
}
