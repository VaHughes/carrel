//! What an independent review of the reader-beside-an-agent slate found in
//! navigation and per-document state. Each test is the reproduction that
//! failed before the fix.

use std::fmt::Write as _;
use std::path::Path;
use std::time::Instant;

use carrel::action::Action;
use carrel::app::{App, update};
use carrel_core::{Document, NodeKind};

fn file_app(dir: &Path, name: &str, src: &str) -> App {
    let path = dir.join(name);
    std::fs::write(&path, src).unwrap();
    let mut app = App::new(name.into(), Document::parse(src), 80, 24);
    app.file = Some(path);
    app.library_root = Some(dir.to_path_buf());
    app.on_resize(80, 24);
    app.index_links();
    app
}

fn long_doc(sections: usize) -> String {
    let mut s = String::new();
    for i in 0..sections {
        writeln!(s, "## Section {i}\n\nparagraph {i} alpha beta gamma\n").unwrap();
    }
    s
}

fn tags_ready(app: &mut App, dir: &Path) {
    std::fs::write(dir.join("t.md"), "---\ntags: [x]\n---\n# T\n").unwrap();
    app.tags = carrel::tags::Request::Ready(carrel::tags::Index {
        tagged: vec![carrel::tags::Tagged {
            path: dir.join("t.md"),
            mtime: std::time::SystemTime::UNIX_EPOCH,
            tags: vec!["x".into()],
            title: None,
        }],
        read: 1,
        capped: false,
    });
    update(app, Action::HomeOpenTags);
}

/// A pipe that is still arriving, as `main.rs::poll_stream` feeds it.
fn streaming(dir: &Path) -> App {
    let mut app = App::new("(stdin — streaming…)".into(), Document::parse(""), 80, 24);
    app.streaming = true;
    app.piped = Some(String::new());
    app.library_root = Some(dir.to_path_buf());
    app.on_resize(80, 24);
    app
}

fn feed(app: &mut App, chunk: &str) {
    let mut buf = app.piped.take().unwrap_or_default();
    buf.push_str(chunk);
    if app.showing_pipe() {
        app.reload_from(&buf);
    }
    app.piped = Some(buf);
}

fn heading_text(app: &App) -> Vec<String> {
    app.doc
        .nodes
        .iter()
        .filter(|n| app.folded.contains(&n.id))
        .map(|n| app.doc.text[n.doc.start as usize..n.doc.end as usize].to_string())
        .collect()
}

/// `#` opens the tags page from inside a pipe now, and the pipe may still be
/// arriving. Its next chunk used to be parsed over the tags page — the pipe's
/// text under the label "tags" — and the fold ids of that parse were then
/// stored against the desk, so coming Forward to it indexed a four-node
/// document with node fifty-two and the reader died.
#[test]
fn a_chunk_arriving_behind_the_tags_page_waits_for_the_reader_to_come_back() {
    let d = tempfile::tempdir().unwrap();
    let mut app = streaming(d.path());
    feed(&mut app, &long_doc(30));
    tags_ready(&mut app, d.path());
    assert_eq!(app.path, "tags");
    let page = app.doc.text.clone();

    feed(&mut app, "\n## Late\n\nmore from the pipe\n");
    assert_eq!(
        app.doc.text, page,
        "the page is what the reader is looking at"
    );

    update(&mut app, Action::FoldAll);
    update(&mut app, Action::Back);
    assert!(
        app.doc.text.contains("more from the pipe"),
        "and the pipe has everything that arrived meanwhile"
    );
    update(&mut app, Action::Forward);
    assert_eq!(app.doc.text, page);
}

#[test]
fn the_welcome_page_is_not_the_desk_it_was_opened_from() {
    let d = tempfile::tempdir().unwrap();
    let mut app = file_app(d.path(), "a.md", "# A\n\nhello\n");
    tags_ready(&mut app, d.path());
    assert!(app.on_desk.is_some());
    update(&mut app, Action::WelcomeOpen);
    assert_eq!(app.on_desk, None);
}

#[test]
fn a_pipes_notes_survive_a_trip_through_a_generated_page() {
    let d = tempfile::tempdir().unwrap();
    let src = "# Piped\n\nsome text to annotate\n";
    let mut app = App::new("(stdin)".into(), Document::parse(src), 80, 24);
    app.piped = Some(src.to_string());
    app.library_root = Some(d.path().to_path_buf());
    app.on_resize(80, 24);
    let note = carrel::marginalia::Annotation::new(&app.doc.text, 6..10, "mine".into()).unwrap();
    app.notes.entries.push(note);

    tags_ready(&mut app, d.path());
    assert_eq!(app.path, "tags");
    // On through the page to a document, so the pipe is two steps behind.
    update(&mut app, Action::LinkOpen(0));
    assert_eq!(app.path, "t.md", "{:?}", app.note);
    update(&mut app, Action::Back);
    update(&mut app, Action::Back);
    assert_eq!(app.path, "(stdin)");
    assert_eq!(app.notes.entries.len(), 1, "the pipe's notes were dropped");
}

/// `%` and a margin-outline click pushed `app.file` — the empty path, in a
/// pipe — so Back said "cannot go back to : No such file or directory".
#[test]
fn a_jump_inside_a_pipe_can_be_gone_back_from() {
    let mut src = String::from("Text with a note[^1].\n\n");
    src.push_str(&long_doc(40));
    src.push_str("\n[^1]: the note\n");
    let mut app = App::new("(stdin)".into(), Document::parse(&src), 80, 24);
    app.piped = Some(src.clone());
    app.on_resize(80, 24);
    update(&mut app, Action::FootnoteJump);
    assert_ne!(app.view.anchor, 0);
    assert!(
        app.history.iter().all(|(p, _)| !p.as_os_str().is_empty()),
        "{:?}",
        app.history
    );
    update(&mut app, Action::Back);
    assert_eq!(app.view.anchor, 0, "{:?}", app.note);
}

#[test]
fn a_generated_page_does_not_show_the_last_files_bookmarks() {
    let d = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut app = file_app(d.path(), "a.md", &long_doc(60));
    app.state_dir = Some(state.path().to_path_buf());
    carrel::state::save_marks_in(state.path(), &d.path().join("a.md"), &[2000, 2300]).unwrap();
    tags_ready(&mut app, d.path());
    assert!(app.marks.is_empty(), "{:?}", app.marks);
}

fn dead(app: &App) -> Vec<&str> {
    let mut v: Vec<&str> = app
        .dead_links
        .iter()
        .map(|id| app.doc.links[id.0 as usize].as_ref())
        .collect();
    v.sort_unstable();
    v
}

/// Struck through, and every one of them works somewhere: an email address,
/// a file with a `?query` after it, an anchor written by hand, a heading
/// named with its accent percent-encoded.
#[test]
fn a_link_that_works_is_not_called_dead() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("b.md"), "x").unwrap();
    let src = "\
<me@example.com> [q](b.md?plain=1) [id](#anchor) [cafe](#caf%C3%A9-au-lait) [gone](c.md?x=1) [no](#nope)

<a id=\"anchor\"></a>

# Café au lait
";
    let mut app = file_app(d.path(), "a.md", src);
    assert_eq!(
        dead(&app),
        ["#nope", "c.md?x=1"],
        "and the control is still dead"
    );

    // …and each of them goes where it says.
    let id = |app: &App, dest: &str| {
        u32::try_from(app.doc.links.iter().position(|l| &**l == dest).unwrap()).unwrap()
    };
    let cafe = id(&app, "#caf%C3%A9-au-lait");
    update(&mut app, Action::LinkOpen(cafe));
    assert_eq!(app.note, None);
    let q = id(&app, "b.md?plain=1");
    update(&mut app, Action::LinkOpen(q));
    assert_eq!(app.path, "b.md", "{:?}", app.note);
}

/// A table of contents: the judge walked every heading once per link.
#[test]
fn judging_a_long_table_of_contents_is_one_pass() {
    let d = tempfile::tempdir().unwrap();
    let n = 2000;
    let mut s = String::new();
    for i in 0..n {
        writeln!(s, "- [Section {i}](#section-{i})").unwrap();
    }
    s.push('\n');
    s.push_str(&long_doc(n));
    let path = d.path().join("toc.md");
    std::fs::write(&path, &s).unwrap();
    let mut app = App::new("toc.md".into(), Document::parse(&s), 80, 24);
    app.file = Some(path);
    app.library_root = Some(d.path().to_path_buf());
    let t = Instant::now();
    app.index_links();
    assert!(app.dead_links.is_empty());
    assert!(t.elapsed().as_millis() < 1500, "{:?}", t.elapsed());
}

/// Naming a collapsed section asked for every slug once per heading, and
/// finding it again asked once per heading per key: a reload of a document
/// left fully collapsed took 3.5 s at 400 headings and 27 s at 800.
#[test]
fn reloading_a_fully_collapsed_document_does_not_freeze() {
    let d = tempfile::tempdir().unwrap();
    let base = long_doc(800);
    let mut app = file_app(d.path(), "big.md", &base);
    update(&mut app, Action::FoldAll);
    let folded = app.folded.len();
    std::fs::write(d.path().join("big.md"), format!("{base}\nappended\n")).unwrap();
    let t = Instant::now();
    app.reload().unwrap();
    assert_eq!(app.folded.len(), folded, "and they are all still collapsed");
    assert!(t.elapsed().as_millis() < 3000, "{:?}", t.elapsed());
}

#[test]
fn a_collapsed_section_is_found_again_by_its_own_name() {
    let d = tempfile::tempdir().unwrap();
    let src = "# Setup\n\none\n\n# Setup\n\ntwo\n\n# Setup 1\n\nthree\n";
    let mut app = file_app(d.path(), "a.md", src);
    let third = app
        .doc
        .nodes
        .iter()
        .filter(|n| matches!(n.kind, NodeKind::Heading { .. }))
        .nth(2)
        .unwrap()
        .id;
    app.folded.insert(third);
    app.relayout();
    std::fs::write(d.path().join("a.md"), format!("{src}\nmore\n")).unwrap();
    app.reload().unwrap();
    assert_eq!(heading_text(&app), ["Setup 1"]);

    // And the link copied for it lands on it.
    app.launch_dir = Some(d.path().to_path_buf());
    let three = u32::try_from(app.doc.text.find("three").unwrap()).unwrap();
    update(&mut app, Action::CopySection(three));
    assert_eq!(app.clipboard.as_deref(), Some("a.md#setup-1-1"));
}

fn reloaded(app: &mut App, dir: &Path, name: &str, new: &str) -> usize {
    std::fs::write(dir.join(name), new).unwrap();
    app.reload().unwrap();
    app.changed.len()
}

/// The change an agent working down a plan makes most is ticking a task
/// off, and the mark lives in the item's marker rather than its text: the
/// task chip went from 0/2 to 1/2 and nothing was marked as changed.
#[test]
fn a_change_a_reader_can_see_is_marked_even_when_the_words_are_the_same() {
    let base = "# Plan\n\n- [ ] step one\n- [ ] step two\n\nSee [the docs](a.md).\n\n1. first\n2. second\n\n```sh\necho hi\n```\n\n## Sub\n\ntail\n";
    for (what, new) in [
        (
            "a task ticked off",
            base.replace("- [ ] step one", "- [x] step one"),
        ),
        ("a link pointed elsewhere", base.replace("(a.md)", "(b.md)")),
        ("a heading's level", base.replace("## Sub", "### Sub")),
        ("a fence's language", base.replace("```sh", "```py")),
    ] {
        let d = tempfile::tempdir().unwrap();
        let mut app = file_app(d.path(), "PLAN.md", base);
        assert_eq!(reloaded(&mut app, d.path(), "PLAN.md", &new), 1, "{what}");
    }
    // A list that starts somewhere else renumbers every item in it.
    let d = tempfile::tempdir().unwrap();
    let mut app = file_app(d.path(), "PLAN.md", base);
    let renumbered = base.replace("1. first", "5. first");
    assert_eq!(reloaded(&mut app, d.path(), "PLAN.md", &renumbered), 2);
    // The control: blank lines and trailing spaces are not a change.
    let d = tempfile::tempdir().unwrap();
    let mut app = file_app(d.path(), "PLAN.md", base);
    let spaced = base.replace("tail\n", "tail   \n\n\n");
    assert_eq!(reloaded(&mut app, d.path(), "PLAN.md", &spaced), 0);
}

/// Appended paragraphs all fit in the last screenful, where scrolling to
/// any of them leaves the same row on top: `c` said "change 1 of 3" forever.
#[test]
fn stepping_reaches_every_change_in_the_last_screenful_and_comes_round() {
    let d = tempfile::tempdir().unwrap();
    let base = long_doc(40);
    let mut app = file_app(d.path(), "PLAN.md", &base);
    let new = format!("{base}\nnew one\n\nnew two\n\nnew three\n");
    assert_eq!(reloaded(&mut app, d.path(), "PLAN.md", &new), 3);
    let mut seen = Vec::new();
    for _ in 0..5 {
        update(&mut app, Action::ChangeStep(1));
        seen.push(app.note.clone().unwrap());
    }
    let n = |i| format!("change {i} of 3");
    assert_eq!(seen, [n(1), n(2), n(3), n(1), n(2)]);
    update(&mut app, Action::ChangeStep(-1));
    assert_eq!(app.note, Some(n(1)));
    update(&mut app, Action::ChangeStep(-1));
    assert_eq!(app.note, Some(n(3)), "and backwards wraps the same way");
}

#[test]
fn stepping_reaches_every_task_in_the_last_screenful() {
    let d = tempfile::tempdir().unwrap();
    let src = format!("{}\n- [ ] one\n- [x] two\n- [ ] three\n", long_doc(40));
    let mut app = file_app(d.path(), "PLAN.md", &src);
    let mut seen = Vec::new();
    for _ in 0..4 {
        update(&mut app, Action::TaskStep(1));
        seen.push(app.note.clone().unwrap());
    }
    assert_eq!(
        seen,
        [
            "task 1 of 3 (open)",
            "task 2 of 3 (done)",
            "task 3 of 3 (open)",
            "task 1 of 3 (open)"
        ]
    );
}

/// Opening a document restores the sections left collapsed in it, and Back
/// set its anchor afterwards without expanding to it: the reader was
/// returned to the top of a collapsed document.
#[test]
fn back_lands_on_its_place_even_when_the_section_was_left_collapsed() {
    let d = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut a = String::from("# A\n\n[b](b.md)\n\n");
    a.push_str(&long_doc(40));
    std::fs::write(d.path().join("b.md"), "# B\n\n[a](a.md)\n").unwrap();
    let mut app = file_app(d.path(), "a.md", &a);
    app.state_dir = Some(state.path().to_path_buf());
    let target = u32::try_from(app.doc.text.find("paragraph 20").unwrap()).unwrap();
    let h = app.text_h();
    app.reveal_byte(target, h, carrel::action::Where::Top);
    let place = app.view.anchor;

    update(&mut app, Action::LinkOpen(0)); // to b
    update(&mut app, Action::LinkOpen(0)); // and back to a by its link
    update(&mut app, Action::FoldAll);
    update(&mut app, Action::Back); // to b, leaving a collapsed
    update(&mut app, Action::Back); // to a, at `place`
    assert_eq!(app.path, "a.md");

    let block = app.doc.block_at_doc(carrel_core::DocByte(place));
    assert!(app.layout.height(block) > 0, "the place is hidden");
    assert_eq!(app.layout.block_at_row(app.view.scroll_row), block);
}

#[test]
fn back_to_a_document_that_was_rewritten_shorter_stays_inside_it() {
    let d = tempfile::tempdir().unwrap();
    let mut src = long_doc(60);
    src.push_str("\n[b](b.md)\n");
    std::fs::write(d.path().join("b.md"), "# B\n").unwrap();
    let mut app = file_app(d.path(), "a.md", &src);
    update(&mut app, Action::ScrollTo(u32::MAX));
    let link = u32::try_from(app.doc.links.len() - 1).unwrap();
    update(&mut app, Action::LinkOpen(link));
    assert_eq!(app.path, "b.md");
    std::fs::write(d.path().join("a.md"), "# A\n\nshort\n").unwrap();
    update(&mut app, Action::Back);
    assert!((app.view.anchor as usize) < app.doc.text.len());
}
