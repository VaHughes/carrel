//! Back, forward, and the trail between documents.
//!
//! One model for every kind of place a reader can have come from — a file,
//! a pipe, a page carrel generated — and one question every departure asks
//! before it leaves (`App::location`). The tests here are about the model
//! holding when those kinds are mixed.

use std::fmt::Write as _;
use std::path::Path;

use carrel::action::Action;
use carrel::app::{App, Outcome, update};
use carrel_core::Document;

/// `a.md` → `b.md` → `c.md`, each linking to the next.
fn chain(dir: &Path) -> App {
    std::fs::write(dir.join("a.md"), "# A\n\nfirst [to b](b.md)\n").unwrap();
    std::fs::write(dir.join("b.md"), "# B\n\nsecond [to c](c.md)\n").unwrap();
    std::fs::write(dir.join("c.md"), "# C\n\nthird, the end\n").unwrap();
    let src = std::fs::read_to_string(dir.join("a.md")).unwrap();
    let mut app = App::new("a.md".into(), Document::parse(&src), 60, 20);
    app.file = Some(dir.join("a.md"));
    app.library_root = Some(dir.to_path_buf());
    app
}

fn name(app: &App) -> String {
    app.file
        .as_deref()
        .and_then(Path::file_name)
        .map_or_else(|| app.path.clone(), |n| n.to_string_lossy().into_owned())
}

#[test]
fn forward_returns_to_where_back_came_from() {
    let d = tempfile::tempdir().unwrap();
    let mut app = chain(d.path());
    update(&mut app, Action::LinkOpen(0));
    update(&mut app, Action::LinkOpen(0));
    assert_eq!(name(&app), "c.md");

    update(&mut app, Action::Back);
    update(&mut app, Action::Back);
    assert_eq!(name(&app), "a.md");
    assert_eq!(app.future.len(), 2);

    update(&mut app, Action::Forward);
    assert_eq!(name(&app), "b.md");
    update(&mut app, Action::Forward);
    assert_eq!(name(&app), "c.md");
    assert_eq!(
        update(&mut app, Action::Forward),
        Outcome::Idle,
        "nothing further"
    );
    assert_eq!(app.history.len(), 2, "and Back works again from here");
}

#[test]
fn a_new_departure_forgets_the_way_forward() {
    let d = tempfile::tempdir().unwrap();
    let mut app = chain(d.path());
    update(&mut app, Action::LinkOpen(0));
    update(&mut app, Action::Back);
    assert_eq!(app.future.len(), 1);
    // From `a.md` again, follow the link afresh: a new branch.
    update(&mut app, Action::LinkOpen(0));
    assert!(app.future.is_empty(), "the branch not taken is gone");
}

#[test]
fn a_name_on_the_trail_goes_back_that_far_and_forward_retraces_it() {
    let d = tempfile::tempdir().unwrap();
    let mut app = chain(d.path());
    update(&mut app, Action::LinkOpen(0));
    update(&mut app, Action::LinkOpen(0));
    // history: [a, b]; here: c. Click `a` — entry 0.
    update(&mut app, Action::BackTo(0));
    assert_eq!(name(&app), "a.md");
    assert_eq!(
        app.history,
        [] as [(std::path::PathBuf, carrel::state::Place); 0]
    );
    update(&mut app, Action::Forward);
    update(&mut app, Action::Forward);
    assert_eq!(name(&app), "c.md", "forward retraces every step it skipped");
    // A segment from a frame the trail has outlived is inert.
    assert_eq!(update(&mut app, Action::BackTo(9)), Outcome::Idle);
    assert_eq!(name(&app), "c.md");
}

#[test]
fn a_target_that_has_vanished_does_not_strand_the_reader() {
    let d = tempfile::tempdir().unwrap();
    let mut app = chain(d.path());
    update(&mut app, Action::LinkOpen(0));
    std::fs::remove_file(d.path().join("a.md")).unwrap();
    update(&mut app, Action::Back);
    assert_eq!(name(&app), "b.md", "still on the document that exists");
    assert!(app.note.as_deref().unwrap().contains("cannot return to"));
    assert!(
        app.future.is_empty(),
        "a Back that went nowhere left nothing ahead"
    );
}

fn hit(path: &Path) -> carrel::grep::Hit {
    carrel::grep::Hit {
        path: path.to_path_buf(),
        count: 1,
        first_line: "third, the end".into(),
        matches: vec![carrel::grep::HitLine {
            lineno: 3,
            line: "third, the end".into(),
        }],
    }
}

#[test]
fn search_results_are_somewhere_to_come_back_to() {
    let d = tempfile::tempdir().unwrap();
    let mut app = chain(d.path());
    app.open_results(d.path(), "third", &[hit(&d.path().join("c.md"))]);
    assert_eq!(app.path, "search: third");

    update(&mut app, Action::LinkOpen(0));
    assert_eq!(name(&app), "c.md");
    update(&mut app, Action::Back);
    assert_eq!(
        app.path, "search: third",
        "Back is the results, not nowhere"
    );
    update(&mut app, Action::Forward);
    assert_eq!(name(&app), "c.md");
}

/// Two generated pages on one trail, with files between them. Each `Back`
/// must reopen the page it left, not the most recent one.
#[test]
fn two_generated_pages_on_one_trail_stay_distinct() {
    let d = tempfile::tempdir().unwrap();
    let mut app = chain(d.path());
    let c = d.path().join("c.md");
    app.open_results(d.path(), "first", &[hit(&c)]);
    update(&mut app, Action::LinkOpen(0)); // → c.md
    // From inside a document, a second search page.
    app.open_results(d.path(), "second", &[hit(&c)]);
    assert_eq!(app.desks.len(), 2);
    update(&mut app, Action::LinkOpen(0)); // → c.md again

    update(&mut app, Action::Back);
    assert_eq!(app.path, "search: second");
    update(&mut app, Action::Back);
    assert_eq!(
        app.path, "search: first",
        "each Back reopens the page it left"
    );
    // Opening the same page again is the same desk, not a third copy.
    app.open_results(d.path(), "second", &[hit(&c)]);
    assert_eq!(app.desks.len(), 2);
}

#[test]
fn a_pipe_is_a_place_too() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("t.md"), "# Target\n").unwrap();
    let src = format!("piped text\n\n[go]({})\n", d.path().join("t.md").display());
    let mut app = App::new("(stdin)".into(), Document::parse(&src), 60, 20);
    app.piped = Some(src);
    update(&mut app, Action::LinkOpen(0));
    assert_eq!(name(&app), "t.md");
    update(&mut app, Action::Back);
    assert!(app.doc.text.contains("piped text"));
    update(&mut app, Action::Forward);
    assert_eq!(
        name(&app),
        "t.md",
        "forward out of a pipe works like any other"
    );
    update(&mut app, Action::Back);
    assert!(
        app.doc.text.contains("piped text"),
        "and the pipe is still there"
    );
}

#[test]
fn returning_to_the_file_list_ends_the_trail() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("a.md"), "[b](b.md)\n").unwrap();
    std::fs::write(d.path().join("b.md"), "b\n").unwrap();
    let mut app = App::new_home(d.path().into(), vec![], 60, 20);
    app.library_root = Some(d.path().to_path_buf());
    app.open_path(&d.path().join("a.md")).unwrap();
    update(&mut app, Action::LinkOpen(0));
    update(&mut app, Action::Back);
    assert_ne!(
        app.future,
        [] as [(std::path::PathBuf, carrel::state::Place); 0]
    );
    update(&mut app, Action::CloseFile);
    assert!(app.is_home());
    assert!(app.history.is_empty() && app.future.is_empty() && app.desks.is_empty());
}

// --- the status row ---

use carrel::action::Zone;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

fn painted(app: &App, cols: u16, rows: u16) -> (ratatui::buffer::Buffer, Vec<(Action, Zone)>) {
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

fn under(buf: &ratatui::buffer::Buffer, z: Zone) -> String {
    (z.x..z.x + z.w).map(|x| buf[(x, z.y)].symbol()).collect()
}

/// `a → b → c`, then one step back: a trail behind and a way forward.
fn mid_trail(dir: &Path, cols: u16, rows: u16) -> App {
    let mut app = chain(dir);
    app.on_resize(cols, rows);
    update(&mut app, Action::LinkOpen(0));
    update(&mut app, Action::LinkOpen(0));
    update(&mut app, Action::Back);
    // Tasks, so the right end has its chip too.
    app
}

/// Geometry that depends on the data painted is recorded, never re-derived —
/// so the check is that each recorded zone sits on exactly the text that
/// says what it does.
#[test]
fn every_part_of_the_status_row_is_a_button_on_its_own_text() {
    let d = tempfile::tempdir().unwrap();
    let (cols, rows) = (100u16, 20u16);
    let app = mid_trail(d.path(), cols, rows);
    assert_eq!(name(&app), "b.md");
    let (buf, targets) = painted(&app, cols, rows);
    let status_y = targets
        .iter()
        .find(|(a, _)| *a == Action::GoHome)
        .map(|(_, z)| z.y)
        .expect("the home icon is on the status row");

    let mut seen = Vec::new();
    for (action, zone) in targets.iter().filter(|(_, z)| z.y == status_y) {
        let text = under(&buf, *zone);
        let want = match action {
            Action::GoHome => "\u{2302}",
            Action::Back => "\u{2039}",
            Action::Forward => "\u{203a}",
            Action::BackTo(0) => "a.md",
            Action::ThemeCycle => "T theme",
            Action::CloseFile => "q quit",
            Action::MenuOpen { .. } => "\u{2261}",
            other => panic!("an unexpected button on the status row: {other:?}"),
        };
        assert_eq!(text, want, "{action:?} is registered over {text:?}");
        seen.push(*action);
    }
    for must in [Action::Back, Action::Forward, Action::BackTo(0)] {
        assert!(seen.contains(&must), "{must:?} was not offered: {seen:?}");
    }
    // The current document's name follows the trail, and is not a button.
    let row: String = (0..cols).map(|x| buf[(x, status_y)].symbol()).collect();
    assert!(row.contains("a.md \u{203a} b.md"), "{row:?}");
}

#[test]
fn a_document_opened_on_its_own_shows_no_arrows_and_no_trail() {
    let d = tempfile::tempdir().unwrap();
    let mut app = chain(d.path());
    app.on_resize(100, 20);
    let (_, targets) = painted(&app, 100, 20);
    assert!(
        !targets
            .iter()
            .any(|(a, _)| matches!(a, Action::Back | Action::Forward | Action::BackTo(_))),
        "nothing to go back to, so nothing offers to"
    );
}

/// At every width the row is one line of non-overlapping things. The right
/// end thins, the trail shortens, the icons give way — and nothing is ever
/// painted over anything else.
#[test]
fn the_status_row_never_overlaps_itself_at_any_width() {
    let d = tempfile::tempdir().unwrap();
    for cols in 12u16..=120 {
        let rows = 20;
        let mut app = mid_trail(d.path(), cols, rows);
        app.on_resize(cols, rows);
        let (buf, targets) = painted(&app, cols, rows);
        let Some(status_y) = targets
            .iter()
            .find(|(a, _)| matches!(a, Action::MenuOpen { .. }))
            .map(|(_, z)| z.y)
        else {
            continue;
        };
        let mut zones: Vec<Zone> = targets
            .iter()
            .filter(|(_, z)| z.y == status_y)
            .map(|(_, z)| *z)
            .collect();
        zones.sort_by_key(|z| z.x);
        for pair in zones.windows(2) {
            assert!(
                pair[0].x + pair[0].w <= pair[1].x,
                "cols={cols}: two buttons share cells: {pair:?}"
            );
        }
        for z in &zones {
            assert!(
                z.x + z.w <= cols,
                "cols={cols}: a button past the edge: {z:?}"
            );
            assert!(
                !under(&buf, *z).trim().is_empty(),
                "cols={cols}: a button over blank cells: {z:?}"
            );
        }
        // The name of the document being read survives every width that has
        // room for it at all.
        if cols >= 20 {
            let row: String = (0..cols).map(|x| buf[(x, status_y)].symbol()).collect();
            assert!(row.contains("b.md"), "cols={cols}: {row:?}");
        }
    }
}

/// Pointing at a link says where it goes, before it is followed.
#[test]
fn pointing_at_a_link_shows_its_destination_on_the_status_row() {
    let d = tempfile::tempdir().unwrap();
    let (cols, rows) = (80u16, 20u16);
    let mut app = chain(d.path());
    app.on_resize(cols, rows);
    let (buf, targets) = painted(&app, cols, rows);
    let link = targets
        .iter()
        .find(|(a, _)| *a == Action::LinkOpen(0))
        .map(|(_, z)| *z)
        .expect("the link is painted");
    let row_with = |buf: &ratatui::buffer::Buffer, needle: &str| {
        (0..rows).any(|y| {
            (0..cols)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
                .contains(needle)
        })
    };
    assert!(row_with(&buf, "T theme") && !row_with(&buf, "b.md "));

    update(&mut app, Action::Hover((link.x, link.y)));
    assert_eq!(app.hovered_link(), Some(carrel_core::LinkId(0)));
    let (buf, _) = painted(&app, cols, rows);
    let status: String = (0..cols).map(|x| buf[(x, rows - 2)].symbol()).collect();
    assert!(status.contains("b.md"), "the destination: {status:?}");
    assert!(
        !status.contains("T theme"),
        "in place of the usual right end"
    );

    // Off the link, it is gone; and a pane over the link means the pointer
    // is on the pane.
    update(&mut app, Action::Hover((link.x, link.y + 3)));
    assert_eq!(app.hovered_link(), None);
    update(&mut app, Action::Hover((link.x, link.y)));
    update(&mut app, Action::HelpToggle);
    assert_eq!(app.hovered_link(), None, "the help sheet is in the way");
}

// --- the scrollbar ---

fn scrollbar(app: &App, cols: u16, rows: u16) -> Vec<String> {
    let (buf, _) = painted(app, cols, rows);
    let top = app.text_y();
    (top..top + app.text_h())
        .map(|y| buf[(cols - 1, y)].symbol().to_string())
        .collect()
}

fn sections(n: usize, level: &str, body_lines: usize) -> String {
    let mut s = String::new();
    for i in 1..=n {
        let _ = write!(s, "{level} Section {i}\n\n");
        for j in 0..body_lines {
            let _ = write!(s, "paragraph {j} of section {i}\n\n");
        }
    }
    s
}

#[test]
fn the_scrollbar_is_notched_where_sections_begin() {
    let (cols, rows) = (60u16, 30u16);
    // Four sections of equal length: the notches are evenly spaced.
    let mut app = App::new(
        "d.md".into(),
        Document::parse(&sections(4, "#", 20)),
        cols,
        rows,
    );
    app.on_resize(cols, rows);
    let bar = scrollbar(&app, cols, rows);
    let notches: Vec<usize> = bar
        .iter()
        .enumerate()
        .filter(|(_, c)| c.as_str() == "┤")
        .map(|(i, _)| i)
        .collect();
    // The first section starts at the top, under the thumb, which wins.
    assert_eq!(notches.len(), 3, "{bar:?}");
    let gaps: Vec<usize> = notches.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(gaps[0].abs_diff(gaps[1]) <= 1, "evenly spaced: {notches:?}");
    // Each notch sits where dragging the thumb would put that heading.
    let total = app.layout.total_rows();
    let third = app.headings()[2];
    let want =
        u64::from(app.layout.row_start(third)) * u64::from(app.text_h() - 1) / u64::from(total - 1);
    assert_eq!(notches[1] as u64, want);
}

#[test]
fn a_document_that_fits_or_has_too_many_sections_has_no_notches() {
    let (cols, rows) = (60u16, 30u16);
    let has = |src: &str| {
        let mut app = App::new("d.md".into(), Document::parse(src), cols, rows);
        app.on_resize(cols, rows);
        scrollbar(&app, cols, rows).iter().any(|c| c == "┤")
    };
    assert!(
        !has("# One\n\nshort\n\n# Two\n\nshort\n"),
        "nowhere to scroll"
    );
    assert!(
        !has(&sections(60, "#", 2)),
        "sixty sections on a 26-row bar would be a solid line"
    );
    // Deep headings do not count: only the top two levels are marked.
    assert!(!has(&sections(6, "####", 12)));
    assert!(has(&sections(6, "##", 12)));
}

/// Once there is a trail the two arrows are drawn together; the one with
/// nowhere to go is there, dim, and is not a button. A lone `›` in front of
/// a name reads as a separator.
#[test]
fn the_arrows_come_as_a_pair_and_only_the_live_one_is_a_button() {
    let d = tempfile::tempdir().unwrap();
    let (cols, rows) = (100u16, 20u16);
    let mut app = chain(d.path());
    app.on_resize(cols, rows);
    update(&mut app, Action::LinkOpen(0));
    update(&mut app, Action::Back); // on a.md: nothing behind, b.md ahead
    let (buf, targets) = painted(&app, cols, rows);
    let y = targets
        .iter()
        .find(|(a, _)| *a == Action::GoHome)
        .map(|(_, z)| z.y)
        .unwrap();
    let row: String = (0..cols).map(|x| buf[(x, y)].symbol()).collect();
    assert!(row.contains("\u{2039} \u{203a} a.md"), "{row:?}");
    assert!(targets.iter().any(|(a, _)| *a == Action::Forward));
    assert!(
        !targets.iter().any(|(a, _)| *a == Action::Back),
        "with nothing behind, the back arrow is not a button"
    );
}

#[test]
fn the_tutorial_can_return_to_the_document_it_was_opened_from() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = chain(dir.path());
    update(&mut app, Action::WelcomeOpen);
    assert!(app.doc.text.contains("Reading in carrel"));
    update(&mut app, Action::Back);
    assert_eq!(name(&app), "a.md");
    update(&mut app, Action::Forward);
    assert!(app.doc.text.contains("Reading in carrel"));
    assert_eq!(app.path, "how carrel works");
}

#[test]
fn a_tutorial_heading_jump_returns_within_the_tutorial_even_with_a_background_pipe() {
    let mut app = App::new("(stdin)".into(), Document::parse("background pipe"), 80, 24);
    app.piped = Some("background pipe".into());
    app.streaming = true;
    update(&mut app, Action::WelcomeOpen);
    update(&mut app, Action::HighlightAdd);
    assert_eq!(app.notes.entries.len(), 1);
    let heading = (0..app.doc.block_count())
        .map(|i| carrel_core::BlockIdx(i as u32))
        .rev()
        .find(|&b| {
            matches!(
                app.doc.node_for_block(b).kind,
                carrel_core::NodeKind::Heading { .. }
            )
        })
        .unwrap();
    update(&mut app, Action::OutlineJumpTo(heading));
    update(&mut app, Action::Back);
    assert!(app.doc.text.contains("Reading in carrel"));
    assert_eq!(
        app.notes.entries.len(),
        1,
        "a same-page jump keeps its notes"
    );
    assert!(!app.showing_pipe());
    update(&mut app, Action::Back);
    assert!(app.showing_pipe());
    assert!(app.doc.text.contains("background pipe"));
}

#[test]
fn opening_the_tutorial_from_home_does_not_revive_a_closed_document() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("closed.md");
    std::fs::write(&file, "closed document").unwrap();
    let mut app = App::new_home(dir.path().into(), vec![], 80, 24);
    app.open_path(&file).unwrap();
    update(&mut app, Action::CloseFile);
    assert!(app.is_home());
    assert_eq!(app.location_path(), None);
    update(&mut app, Action::WelcomeOpen);
    assert_eq!(
        app.history,
        [] as [(std::path::PathBuf, carrel::state::Place); 0]
    );
    update(&mut app, Action::CloseFile);
    assert!(app.is_home());
}

#[test]
fn a_retained_page_keeps_its_annotations_and_bookmarks_across_back_and_forward() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = chain(dir.path());
    update(&mut app, Action::WelcomeOpen);
    update(&mut app, Action::HighlightAdd);
    update(&mut app, Action::MarkToggle);
    assert_eq!(app.notes.entries.len(), 1);
    assert_eq!(app.marks.len(), 1);
    let note = app.notes.entries[0].clone();
    update(&mut app, Action::Back);
    assert_eq!(name(&app), "a.md");
    assert!(
        app.notes.entries.is_empty(),
        "tutorial notes do not belong to a.md"
    );
    assert!(
        app.marks.is_empty(),
        "tutorial bookmarks do not belong to a.md"
    );
    update(&mut app, Action::Forward);
    assert_eq!(
        app.notes.entries.len(),
        1,
        "the retained page still owns its notes"
    );
    assert_eq!(app.notes.entries[0], note);
    assert_eq!(app.marks.len(), 1);
    update(&mut app, Action::WelcomeOpen);
    assert_eq!(
        app.notes.entries[0], note,
        "reopening this same page keeps its notes"
    );
    assert_eq!(app.marks.len(), 1);
}

#[test]
fn a_pipe_keeps_its_bookmarks_and_collapsed_sections_across_a_link_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let other = dir.path().join("other.md");
    std::fs::write(&other, "# Other\n\nother document").unwrap();
    let source = format!(
        "[other]({})\n\n# Section\n\nkept paragraph\n",
        other.display()
    );
    let mut app = App::new("(stdin)".into(), Document::parse(&source), 80, 24);
    app.piped = Some(source);
    update(&mut app, Action::MarkToggle);
    update(&mut app, Action::FoldAll);
    assert_eq!(app.marks.len(), 1);
    assert_eq!(app.folded.len(), 1);
    update(&mut app, Action::LinkOpen(0));
    assert_eq!(name(&app), "other.md");
    assert!(app.marks.is_empty(), "pipe bookmarks stay with the pipe");
    update(&mut app, Action::Back);
    assert!(app.showing_pipe());
    assert_eq!(app.marks.len(), 1);
    assert_eq!(app.folded.len(), 1);
}

#[test]
fn back_and_forward_follow_their_text_across_a_live_rewrite() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("live.md");
    let source = format!(
        "# Origin\n\nOpening paragraph.\n\n## Destination\n\n{}",
        "A later paragraph.\n\n".repeat(30)
    );
    std::fs::write(&file, &source).unwrap();
    let mut app = App::new("live.md".into(), Document::parse(""), 70, 10);
    app.open_path(&file).unwrap();
    let destination = (0..app.doc.block_count())
        .map(|i| carrel_core::BlockIdx(i as u32))
        .find(|&block| app.doc.block_text(block) == "Destination")
        .unwrap();
    update(&mut app, Action::OutlineJumpTo(destination));
    let at = |app: &App| {
        app.doc
            .block_text(app.doc.block_at_doc(carrel_core::DocByte(app.view.anchor)))
            .to_owned()
    };
    assert_eq!(at(&app), "Destination");
    app.reload_from(&format!("# Inserted before everything\n\n{source}"));
    update(&mut app, Action::Back);
    assert_eq!(at(&app), "Origin", "Back names the text previously visited");
    app.reload_from(&format!(
        "# Another insertion\n\nLonger new introduction.\n\n{source}"
    ));
    update(&mut app, Action::Forward);
    assert_eq!(at(&app), "Destination", "Forward survives a rewrite too");
}

#[test]
fn back_follows_the_old_paragraph_when_the_file_changed_while_away() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = chain(dir.path());
    let source = std::fs::read_to_string(dir.path().join("a.md")).unwrap();
    update(&mut app, Action::LinkOpen(0));
    std::fs::write(
        dir.path().join("a.md"),
        format!("# Newly inserted\n\n{source}"),
    )
    .unwrap();
    update(&mut app, Action::Back);
    assert_eq!(name(&app), "a.md");
    assert_eq!(
        app.doc
            .block_text(app.doc.block_at_doc(carrel_core::DocByte(app.view.anchor))),
        "A",
        "return to the old heading, not whatever moved into byte zero"
    );
}
