//! The file list's columns, its order, the preview beside it, and paths
//! pasted onto it.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use carrel::action::{Action, SearchKey};
use carrel::app::{App, paste_on_home, update};
use carrel::home::{self, HomeMode, Reading, Sort};
use carrel::scan::Entry;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;

fn at(secs: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
}

/// Three documents, written in the order b, a, c (c newest).
fn listed(dir: &Path, cols: u16, rows: u16) -> App {
    let mut entries = Vec::new();
    for (name, secs, body) in [
        ("b.md", 100, "# Bee\n\nthe second letter\n"),
        ("a.md", 200, "# Ay\n\nthe first letter\n"),
        ("sub/c.md", 300, "# Sea\n\nthe third letter\n"),
    ] {
        let path = dir.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, body).unwrap();
        entries.push(Entry {
            path,
            mtime: at(secs),
        });
    }
    let mut app = App::new_home(dir.to_path_buf(), Vec::new(), cols, rows);
    let h = app.home_mut().unwrap();
    h.push_many(entries);
    h.finish_scan(0);
    app
}

fn names(app: &App) -> Vec<String> {
    let h = app.home().unwrap();
    h.filtered
        .iter()
        .map(|&i| {
            h.entries[i]
                .path
                .strip_prefix(&h.root)
                .unwrap()
                .display()
                .to_string()
        })
        .collect()
}

fn frame(app: &App, cols: u16, rows: u16) -> (Buffer, carrel::render::Painted) {
    let mut term = Terminal::new(TestBackend::new(cols, rows)).unwrap();
    let mut painted = carrel::render::Painted::default();
    term.draw(|f| {
        carrel::render::draw_full(f, app, &mut painted, &mut std::collections::HashMap::new());
    })
    .unwrap();
    (term.backend().buffer().clone(), painted)
}

fn rows_of(buf: &Buffer, cols: u16, rows: u16) -> Vec<String> {
    (0..rows)
        .map(|y| (0..cols).map(|x| buf[(x, y)].symbol()).collect())
        .collect()
}

// --- when, and how far ---

#[test]
fn a_time_is_said_the_way_a_person_says_it() {
    let now = at(5_000_000_000);
    let back = |secs: u64| home::ago(now, at(5_000_000_000 - secs));
    assert_eq!(back(0), "just now");
    assert_eq!(back(59), "just now");
    assert_eq!(back(60), "1 min ago");
    assert_eq!(back(3_599), "59 min ago");
    assert_eq!(back(3_600), "1 h ago");
    assert_eq!(back(86_399), "23 h ago");
    assert_eq!(back(86_400), "1 d ago");
    assert_eq!(back(6 * 86_400), "6 d ago");
    assert_eq!(back(7 * 86_400), "1 wk ago");
    assert_eq!(back(60 * 86_400), "1 mo ago");
    assert_eq!(back(800 * 86_400), "2 y ago");
    // A file from the future — a wrong clock, a copy from another machine.
    assert_eq!(home::ago(at(100), at(5_000)), "just now");
    // Every answer fits the column the list gives it.
    for secs in [
        0,
        61,
        3_700,
        90_000,
        700_000,
        3_000_000,
        40_000_000,
        4_000_000_000,
    ] {
        assert!(back(secs).len() <= 10, "{}", back(secs));
    }
}

#[test]
fn progress_is_a_percentage_between_not_started_and_read() {
    assert_eq!(home::progress_label(0), "");
    assert_eq!(home::progress_label(9), "", "opened, not read");
    assert_eq!(home::progress_label(10), "1%");
    assert_eq!(home::progress_label(424), "42%");
    assert_eq!(home::progress_label(989), "98%");
    assert_eq!(home::progress_label(990), "read");
    assert_eq!(home::progress_label(1000), "read");
    assert_eq!(home::progress_label(u16::MAX), "read");
}

#[test]
fn a_roomy_list_says_when_and_how_far_and_a_narrow_one_only_says_what() {
    let d = tempfile::tempdir().unwrap();
    let mut app = listed(d.path(), 80, 30);
    let a = d.path().join("a.md");
    app.home_mut().unwrap().reading.insert(
        a,
        Reading {
            permille: 420,
            rank: 0,
        },
    );
    let (buf, _) = frame(&app, 80, 30);
    let rows = rows_of(&buf, 80, 30);
    let row = rows.iter().find(|r| r.contains("a.md")).unwrap();
    assert!(row.contains("42%"), "{row:?}");
    assert!(row.trim_end().ends_with("ago"), "{row:?}");
    let other = rows.iter().find(|r| r.contains("b.md")).unwrap();
    assert!(
        !other.contains('%'),
        "an unread document shows no progress: {other:?}"
    );

    app.on_resize(40, 30);
    let (buf, _) = frame(&app, 40, 30);
    let rows = rows_of(&buf, 40, 30);
    let row = rows.iter().find(|r| r.contains("a.md")).unwrap();
    assert!(!row.contains("ago") && !row.contains('%'), "{row:?}");
}

// --- order ---

#[test]
fn the_list_has_three_orders_and_keeps_its_place_through_each() {
    let d = tempfile::tempdir().unwrap();
    let mut app = listed(d.path(), 80, 30);
    assert_eq!(names(&app), ["sub/c.md", "a.md", "b.md"], "newest first");
    // Read b most recently, then c; a never.
    let h = app.home_mut().unwrap();
    h.reading.insert(
        d.path().join("b.md"),
        Reading {
            permille: 500,
            rank: 0,
        },
    );
    h.reading.insert(
        d.path().join("sub/c.md"),
        Reading {
            permille: 100,
            rank: 1,
        },
    );
    h.select(1); // a.md

    update(&mut app, Action::HomeSort);
    assert_eq!(names(&app), ["a.md", "b.md", "sub/c.md"], "by name");
    let h = app.home().unwrap();
    assert_eq!(h.selected_path(), Some(d.path().join("a.md").as_path()));
    assert_eq!(h.note.as_deref(), Some("sorted by name"));

    update(&mut app, Action::HomeSort);
    assert_eq!(
        names(&app),
        ["b.md", "sub/c.md", "a.md"],
        "recently read first, then the unread"
    );
    assert_eq!(
        app.home().unwrap().selected_path(),
        Some(d.path().join("a.md").as_path()),
        "the highlight stayed on its file"
    );

    update(&mut app, Action::HomeSort);
    assert_eq!(app.home().unwrap().sort, Sort::Newest, "and round again");
}

#[test]
fn the_order_is_remembered_and_an_unknown_word_is_the_default() {
    let d = tempfile::tempdir().unwrap();
    let cfg = tempfile::tempdir().unwrap();
    let mut app = listed(d.path(), 80, 30);
    app.config_dir = Some(cfg.path().to_path_buf());
    update(&mut app, Action::HomeSort);
    assert_eq!(
        carrel::config::load_all_in(cfg.path()).sort.as_deref(),
        Some("name")
    );
    assert_eq!(Sort::from_name("name"), Some(Sort::Name));
    assert_eq!(Sort::from_name(" READ "), Some(Sort::Read));
    assert_eq!(Sort::from_name("sideways"), None);
}

#[test]
fn typing_still_ranks_by_match_whatever_the_order() {
    let d = tempfile::tempdir().unwrap();
    let mut app = listed(d.path(), 80, 30);
    update(&mut app, Action::HomeSort); // by name
    update(&mut app, Action::HomeFilterMode);
    for c in "c.md".chars() {
        update(&mut app, Action::HomeKey(SearchKey::Char(c)));
    }
    // Best match first — not first by name, which would be `a.md` if the
    // scratch folder's own random name happens to let it match at all.
    assert_eq!(names(&app)[0], "sub/c.md");
}

// --- the preview ---

#[test]
fn a_wide_window_splits_and_a_narrow_one_does_not() {
    assert_eq!(home::preview_split(99, true), None);
    assert_eq!(home::preview_split(200, false), None, "switched off");
    for cols in [100u16, 120, 160, 300] {
        let (list, px, pw) = home::preview_split(cols, true).unwrap();
        assert!(list >= home::META_MIN_COLS, "the list keeps its columns");
        assert!(
            px > list && px + pw <= cols,
            "cols={cols}: {list} {px} {pw}"
        );
    }
}

fn with_preview(app: &mut App, name: &str, text: Option<&str>, width: u16) {
    let h = app.home_mut().unwrap();
    let e = h
        .entries
        .iter()
        .find(|e| e.path.ends_with(name))
        .unwrap()
        .clone();
    h.preview = Some(home::Preview {
        path: e.path,
        mtime: e.mtime,
        text: text.map(str::to_string),
        lines: text
            .map(|t| home::preview_lines(t, width))
            .unwrap_or_default(),
        width,
    });
}

#[test]
fn the_preview_shows_the_highlighted_document_and_only_that_one() {
    let d = tempfile::tempdir().unwrap();
    let (cols, rows) = (120u16, 30u16);
    let mut app = listed(d.path(), cols, rows);
    let (_, _, pw) = home::preview_split(cols, true).unwrap();
    assert_eq!(
        app.home().unwrap().preview_wanted(cols).map(|w| w.0),
        Some(d.path().join("sub/c.md")),
        "the highlighted row is what is asked for"
    );

    with_preview(
        &mut app,
        "sub/c.md",
        Some("# Sea\n\nthe third letter\n"),
        pw - 2,
    );
    let (buf, _) = frame(&app, cols, rows);
    let text = rows_of(&buf, cols, rows).join("\n");
    assert!(text.contains("the third letter"), "{text}");

    // Move the highlight: the old preview is for another document now, and
    // is not shown under this one's name.
    update(&mut app, Action::HomeMove(1));
    let (buf, _) = frame(&app, cols, rows);
    let text = rows_of(&buf, cols, rows).join("\n");
    assert!(
        !text.contains("the third letter"),
        "a stale preview:\n{text}"
    );

    with_preview(&mut app, "a.md", None, pw - 2);
    let (buf, _) = frame(&app, cols, rows);
    assert!(
        rows_of(&buf, cols, rows)
            .join("\n")
            .contains("cannot be previewed")
    );
}

#[test]
fn nothing_is_asked_for_when_there_is_no_preview_to_show() {
    let d = tempfile::tempdir().unwrap();
    let mut app = listed(d.path(), 120, 30);
    assert!(
        app.home().unwrap().preview_wanted(80).is_none(),
        "too narrow"
    );
    app.home_mut().unwrap().show_preview = false;
    assert!(
        app.home().unwrap().preview_wanted(120).is_none(),
        "switched off"
    );
    app.home_mut().unwrap().show_preview = true;
    update(&mut app, Action::HomeSearchMode);
    assert!(
        app.home().unwrap().preview_wanted(120).is_none(),
        "searching"
    );
}

/// The list's rows are found by row alone, so the preview's rectangle has
/// to take its own clicks — or a click on the preview selects whichever
/// file shares its screen row.
#[test]
fn a_click_on_the_preview_is_not_a_click_on_the_list() {
    let d = tempfile::tempdir().unwrap();
    let (cols, rows) = (120u16, 30u16);
    let app = listed(d.path(), cols, rows);
    let (_, painted) = frame(&app, cols, rows);
    let (list_w, px, _) = home::preview_split(cols, true).unwrap();
    let h = app.home().unwrap();
    let (top, _) = home::list_geometry(cols, rows, app.hints, h.resume_shown());
    assert!(h.row_at(top, cols, rows, app.hints).is_some(), "a list row");
    assert_eq!(
        painted.targets.hit(px + 3, top).map(|t| t.action),
        Some(Action::Absorb),
        "the preview swallows it"
    );
    assert_eq!(
        painted.targets.hit(list_w - 3, top).map(|t| t.action),
        None,
        "and the list's own cells are left to the list"
    );
}

#[test]
fn a_head_is_whole_lines_and_never_a_pipe() {
    let d = tempfile::tempdir().unwrap();
    let long = d.path().join("long.md");
    let line = "a line of exactly forty characters here\n";
    std::fs::write(&long, line.repeat(1000)).unwrap();
    let head = carrel::scan::head_of(&long).unwrap();
    assert!(head.len() <= carrel::scan::HEAD_BYTES);
    assert!(head.ends_with('\n'), "cut at a line, not mid-word");
    assert_eq!(head.len() % line.len(), 0);

    let short = d.path().join("short.md");
    std::fs::write(&short, "no newline at the end").unwrap();
    assert_eq!(
        carrel::scan::head_of(&short).as_deref(),
        Some("no newline at the end")
    );
    assert_eq!(carrel::scan::head_of(&d.path().join("missing.md")), None);
    assert_eq!(
        carrel::scan::head_of(d.path()),
        None,
        "a folder is not a document"
    );
}

// --- pasting a path ---

#[test]
fn a_pasted_path_opens_that_document() {
    let d = tempfile::tempdir().unwrap();
    let mut app = listed(d.path(), 80, 30);
    let target = d.path().join("sub/c.md");
    paste_on_home(&mut app, &format!("{}\n", target.display()));
    assert_eq!(
        app.file.as_deref(),
        Some(target.as_path()),
        "{:?}",
        app.note
    );
}

#[test]
fn a_bare_name_is_a_name_in_the_folder_and_a_line_is_honored() {
    let d = tempfile::tempdir().unwrap();
    let mut body = String::new();
    for i in 1..=80 {
        let _ = write!(body, "paragraph {i}\n\n");
    }
    std::fs::write(d.path().join("long.md"), body).unwrap();
    let mut app = listed(d.path(), 80, 20);
    paste_on_home(&mut app, "long.md:101");
    assert_eq!(
        app.file.as_deref(),
        Some(d.path().join("long.md").as_path())
    );
    let top = app.layout.block_at_row(app.view.scroll_row);
    assert_eq!(
        app.doc.block_text(top),
        "paragraph 51",
        "line 101 of the file"
    );
}

#[test]
fn a_pasted_folder_becomes_the_folder_being_listed() {
    let d = tempfile::tempdir().unwrap();
    let mut app = listed(d.path(), 80, 30);
    let sub = d.path().join("sub");
    paste_on_home(&mut app, &format!("'{}'", sub.display()));
    assert!(app.is_home());
    assert_eq!(app.home().unwrap().root, sub);
}

#[test]
fn a_paste_that_is_not_a_path_opens_nothing_and_says_so() {
    let d = tempfile::tempdir().unwrap();
    let mut app = listed(d.path(), 80, 30);
    paste_on_home(&mut app, "Some sentence copied by mistake, not a path.");
    assert!(app.is_home());
    assert!(
        app.home()
            .unwrap()
            .note
            .as_deref()
            .unwrap()
            .contains("does not look like")
    );

    paste_on_home(&mut app, "nowhere/at-all.md");
    assert!(app.is_home());
    let note = app.home().unwrap().note.clone().unwrap();
    assert!(note.contains("cannot open"), "{note}");
}

#[test]
fn a_paste_while_typing_is_typing() {
    let d = tempfile::tempdir().unwrap();
    let mut app = listed(d.path(), 80, 30);
    update(&mut app, Action::HomeFilterMode);
    // A path would open a file in browse mode. Here it is what was typed.
    paste_on_home(&mut app, &d.path().join("a.md").display().to_string());
    assert!(app.is_home(), "nothing opened");
    let h = app.home().unwrap();
    assert_eq!(h.mode, HomeMode::Filter);
    assert!(h.filter.ends_with("a.md"), "{:?}", h.filter);
    let _ = PathBuf::new();
}

// --- what an independent review found ---

/// Under "recently read", coming back from a document moves it to the top.
/// The highlight was kept by row, so it was left on whichever file slid into
/// that row and Enter opened something the reader never chose.
#[test]
fn the_highlight_stays_on_its_file_when_reading_reorders_the_list() {
    let d = tempfile::tempdir().unwrap();
    let mut app = listed(d.path(), 80, 30);
    let h = app.home_mut().unwrap();
    while h.sort != Sort::Read {
        h.cycle_sort();
    }
    h.selected = 2;
    let chosen = h.selected_path().unwrap().to_path_buf();
    let reading = std::iter::once((
        chosen.clone(),
        Reading {
            permille: 400,
            rank: 0,
        },
    ))
    .collect();
    h.set_reading(reading);
    assert_eq!(h.selected_path(), Some(chosen.as_path()));
    assert_eq!(h.selected, 0, "and it is the most recently read");
}

#[test]
fn a_pasted_parent_is_the_parent_and_not_a_path_with_dots_in_it() {
    let d = tempfile::tempdir().unwrap();
    let mut app = listed(d.path(), 80, 30);
    let sub = d.path().join("sub");
    paste_on_home(&mut app, &sub.display().to_string());
    assert_eq!(app.home().unwrap().root, sub);
    paste_on_home(&mut app, "..");
    assert_eq!(app.home().unwrap().root, d.path());
}

/// Replayed a character at a time, every character narrowed the whole list
/// again: a pasted paragraph froze the screen for a quarter of a minute.
#[test]
fn a_paragraph_pasted_into_the_filter_is_one_edit_and_not_two_thousand() {
    let d = tempfile::tempdir().unwrap();
    let mut app = listed(d.path(), 80, 30);
    let h = app.home_mut().unwrap();
    let more: Vec<Entry> = (0..6000)
        .map(|i| Entry {
            path: d
                .path()
                .join(format!("notes/section-{i:05}-of-the-long-report.md")),
            mtime: at(1000 + i),
        })
        .collect();
    h.push_many(more);
    h.finish_scan(0);
    h.mode = HomeMode::Filter;
    let prose = "the quick brown fox jumps over the lazy dog and keeps going ".repeat(40);
    let t = std::time::Instant::now();
    paste_on_home(&mut app, &prose);
    assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
    let filter = &app.home().unwrap().filter;
    assert!(prose.starts_with(filter.as_str()) && filter.chars().count() == 256);

    // And the short paste that is the ordinary case lands whole.
    update(&mut app, Action::HomeKey(SearchKey::Cancel));
    paste_on_home(&mut app, "section-00042\rsecond line");
    assert_eq!(app.home().unwrap().filter, "section-00042");
    assert_eq!(names(&app).len(), 1);
}

#[test]
fn turning_the_preview_off_while_reading_reaches_the_list_behind() {
    let d = tempfile::tempdir().unwrap();
    let mut app = listed(d.path(), 120, 30);
    assert!(app.home().unwrap().show_preview);
    update(&mut app, Action::HomeOpen);
    assert!(!app.is_home());
    update(&mut app, Action::SettingsToggle);
    let at = carrel::app::settings_rows(&app)
        .iter()
        .position(|r| r.key == carrel::app::Setting::Preview)
        .unwrap();
    update(&mut app, Action::SettingsPickAt(u32::try_from(at).unwrap()));
    update(&mut app, Action::SettingsToggle);
    update(&mut app, Action::CloseFile);
    assert!(app.is_home());
    assert!(!app.home().unwrap().show_preview);
}
