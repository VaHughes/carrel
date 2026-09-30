//! The tags document, from the keystroke to coming back to it.
//!
//! `tags.rs`'s own tests cover what a tag is and what the document says.
//! These cover the part that is state: that asking is only asking, that the
//! document opens collapsed into a list of tags, that its links resolve
//! against the folder, and that `Back` returns to it as it was left.

use std::path::Path;
use std::time::{Duration, SystemTime};

use carrel::action::{Action, SearchKey};
use carrel::app::{App, Outcome, update};
use carrel::scan::Entry;
use carrel::tags::{Index, Request, Tagged};
use carrel_core::NodeKind;

/// A folder of tagged notes on disk, and the index a scan of it produces.
fn vault(dir: &Path, notes: usize) -> (Vec<Entry>, Index) {
    let mut entries = Vec::new();
    let mut index = Index::default();
    for i in 0..notes {
        let path = dir.join(format!("note{i}.md"));
        let tags = vec!["rust".to_string(), format!("topic{}", i % 3)];
        std::fs::write(
            &path,
            format!(
                "---\ntags: [{}]\n---\n\n# Note {i}\n\nbody {i}\n",
                tags.join(", ")
            ),
        )
        .unwrap();
        let mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(i as u64);
        entries.push(Entry {
            path: path.clone(),
            mtime,
        });
        index.tagged.push(Tagged {
            path,
            mtime,
            tags,
            title: None,
        });
    }
    index.read = notes;
    (entries, index)
}

fn home(dir: &Path, entries: Vec<Entry>, cols: u16, rows: u16) -> App {
    let mut app = App::new_home(dir.to_path_buf(), Vec::new(), cols, rows);
    if let Some(h) = app.home_mut() {
        // As the live walk delivers them; a finished scan keeps only what
        // the walk itself reported.
        h.push_many(entries);
        h.finish_scan(0);
    }
    app
}

/// Hand the state layer a finished scan, the way the event loop does.
fn deliver(app: &mut App, index: Index) -> Outcome {
    app.tags = Request::Ready(index);
    update(app, Action::HomeOpenTags)
}

fn headings(app: &App, level: u8) -> Vec<String> {
    app.doc
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Heading { level })
        .map(|n| app.doc.text[n.doc.start as usize..n.doc.end as usize].to_string())
        .collect()
}

#[test]
fn asking_for_tags_only_asks() {
    let d = tempfile::tempdir().unwrap();
    let (entries, _) = vault(d.path(), 3);
    let mut app = home(d.path(), entries, 60, 20);

    update(&mut app, Action::HomeTags);
    let h = app.home().unwrap();
    assert_eq!(app.tags, Request::Wanted, "recorded for the event loop");
    assert_eq!(h.note.as_deref(), Some("reading tags…"));
    assert!(app.is_home(), "and nothing has opened");
}

#[test]
fn whatever_the_reader_does_next_withdraws_the_request() {
    let d = tempfile::tempdir().unwrap();
    let (entries, _) = vault(d.path(), 3);
    for next in [
        Action::HomeMove(1),
        Action::HomeFilterMode,
        Action::HomeKey(SearchKey::Cancel),
        Action::HelpToggle,
    ] {
        let mut app = home(d.path(), entries.clone(), 60, 20);
        update(&mut app, Action::HomeTags);
        update(&mut app, next);
        assert_eq!(
            app.tags,
            Request::Idle,
            "{next:?} must withdraw a pending tags request"
        );
    }
    // The pointer moving is not the reader doing something.
    let mut app = home(d.path(), entries, 60, 20);
    update(&mut app, Action::HomeTags);
    update(&mut app, Action::Hover((3, 3)));
    assert_eq!(app.tags, Request::Wanted);
}

#[test]
fn an_empty_folder_says_so_instead_of_waiting() {
    let d = tempfile::tempdir().unwrap();
    let mut app = home(d.path(), Vec::new(), 60, 20);
    update(&mut app, Action::HomeTags);
    let h = app.home().unwrap();
    assert_eq!(app.tags, Request::Idle, "there is nothing to read");
    assert!(h.note.as_deref().unwrap().contains("no documents"));
}

#[test]
fn a_folder_with_no_tags_is_a_note_not_an_empty_document() {
    let d = tempfile::tempdir().unwrap();
    let (entries, _) = vault(d.path(), 3);
    let mut app = home(d.path(), entries, 60, 20);
    deliver(
        &mut app,
        Index {
            tagged: Vec::new(),
            read: 3,
            capped: false,
        },
    );
    assert!(app.is_home(), "nothing to read, so nothing opens");
    let note = app.home().unwrap().note.clone().unwrap();
    assert!(note.contains("no tags"), "{note}");
    assert!(
        note.contains("3 documents"),
        "it says how many it read: {note}"
    );
    assert!(
        note.contains("tags:"),
        "and what would change the answer: {note}"
    );
}

#[test]
fn the_tags_open_as_a_document_with_a_section_per_tag() {
    let d = tempfile::tempdir().unwrap();
    let (entries, index) = vault(d.path(), 6);
    let mut app = home(d.path(), entries, 60, 60);
    deliver(&mut app, index);

    assert!(!app.is_home(), "the document opened in the reader");
    assert_eq!(app.path, "tags");
    assert_eq!(app.file, None, "generated, so pathless");
    assert_eq!(headings(&app, 1), ["Tags"]);
    assert_eq!(
        headings(&app, 2),
        ["rust (6)", "topic0 (2)", "topic1 (2)", "topic2 (2)"]
    );
    assert_eq!(app.doc.links.len(), 12, "a link per document per tag");
}

#[test]
fn a_long_tags_document_opens_as_a_list_of_tags() {
    let d = tempfile::tempdir().unwrap();
    let (entries, index) = vault(d.path(), 30);
    // Short enough to need it...
    let mut app = home(d.path(), entries.clone(), 60, 16);
    deliver(&mut app, index.clone());
    let sections = app
        .doc
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Heading { level: 2 })
        .count();
    assert_eq!(app.folded.len(), sections, "every tag is collapsed");
    assert!(
        !app.doc
            .nodes
            .iter()
            .any(|n| { n.kind == NodeKind::Heading { level: 1 } && app.folded.contains(&n.id) }),
        "but never the title — collapsing that hides the whole page"
    );
    assert!(
        app.layout.total_rows() <= u32::from(app.text_h()) + 2,
        "collapsed, the tag list is about a screenful: {} rows",
        app.layout.total_rows()
    );

    // ...and a window tall enough to show everything collapses nothing.
    let mut tall = home(d.path(), entries, 60, 400);
    deliver(&mut tall, index);
    assert!(tall.folded.is_empty());
}

/// The point of keeping the document: open one, come back, open the next.
#[test]
fn back_returns_to_the_tags_as_they_were_left() {
    let d = tempfile::tempdir().unwrap();
    let (entries, index) = vault(d.path(), 30);
    let mut app = home(d.path(), entries, 60, 16);
    app.library_root = Some(d.path().to_path_buf());
    deliver(&mut app, index);

    // Expand the second tag and scroll to it, then follow its first link.
    let second = app
        .doc
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Heading { level: 2 })
        .nth(1)
        .map(|n| (n.id, n.doc.clone()))
        .unwrap();
    app.folded.remove(&second.0);
    app.relayout();
    let folded = app.folded.clone();
    let link = app
        .doc
        .nodes
        .iter()
        .flat_map(|n| n.inlines.iter())
        .find(|i| i.link.is_some() && i.doc.start > second.1.end)
        .and_then(|i| i.link)
        .unwrap();
    update(&mut app, Action::Scroll(carrel::action::Span::Line, 3));
    let anchor = app.view.anchor;
    assert!(anchor > 0, "the fixture must scroll");

    assert_eq!(update(&mut app, Action::LinkOpen(link.0)), Outcome::Redraw);
    assert!(
        app.file.is_some(),
        "the link opened a real file: {:?}",
        app.note
    );
    assert!(app.doc.text.contains("body"));

    update(&mut app, Action::Back);
    assert_eq!(app.path, "tags", "Back returned to the tags document");
    assert_eq!(app.file, None);
    assert_eq!(app.folded, folded, "with the same sections expanded");
    assert_eq!(app.view.anchor, anchor, "at the same place");

    // And it works again from there: the desk was not used up.
    update(&mut app, Action::LinkOpen(link.0));
    update(&mut app, Action::Back);
    assert_eq!(app.path, "tags");
}

#[test]
fn closing_a_document_opened_from_tags_returns_to_the_file_list() {
    let d = tempfile::tempdir().unwrap();
    let (entries, index) = vault(d.path(), 6);
    let mut app = home(d.path(), entries, 60, 60);
    app.library_root = Some(d.path().to_path_buf());
    deliver(&mut app, index);
    let link = app
        .doc
        .nodes
        .iter()
        .flat_map(|n| n.inlines.iter())
        .find_map(|i| i.link)
        .unwrap();
    update(&mut app, Action::LinkOpen(link.0));
    update(&mut app, Action::CloseFile);
    assert!(app.is_home());
    assert!(app.desks.is_empty(), "the file list is not on the trail");
    assert!(app.history.is_empty());
}

#[test]
fn the_real_scan_feeds_the_real_document() {
    // One pass through the actual thread and the actual parser, so the
    // pieces the tests above hand-build are known to fit together.
    let d = tempfile::tempdir().unwrap();
    let (entries, expected) = vault(d.path(), 5);
    let rx = carrel::tags::spawn(carrel::tags::Source::Entries(entries.clone()), false, 1);
    let mut index = Index::default();
    while let Ok(msg) = rx.recv() {
        match msg {
            carrel::tags::Msg::Found(t, _) => index.tagged.push(t),
            carrel::tags::Msg::Done { read, capped, .. } => {
                index.read = read;
                index.capped = capped;
                break;
            }
        }
    }
    assert_eq!(index, expected);

    let mut app = home(d.path(), entries, 60, 60);
    deliver(&mut app, index);
    assert_eq!(headings(&app, 2)[0], "rust (5)");
}

/// The links pane (`l`) is a second way out of the tags document, and it
/// has to leave the same trail the link itself does.
#[test]
fn back_also_returns_after_leaving_through_the_links_pane() {
    let d = tempfile::tempdir().unwrap();
    let (entries, index) = vault(d.path(), 6);
    let mut app = home(d.path(), entries, 60, 60);
    app.library_root = Some(d.path().to_path_buf());
    deliver(&mut app, index);

    update(&mut app, Action::ForwardToggle);
    assert!(app.forward.is_some(), "the pane lists the page's links");
    update(&mut app, Action::ForwardOpen);
    assert!(app.file.is_some(), "a document opened: {:?}", app.note);

    update(&mut app, Action::Back);
    assert_eq!(app.path, "tags");
}

#[test]
fn a_link_that_cannot_be_opened_leaves_nothing_on_the_trail() {
    let d = tempfile::tempdir().unwrap();
    let (entries, index) = vault(d.path(), 6);
    let mut app = home(d.path(), entries.clone(), 60, 60);
    app.library_root = Some(d.path().to_path_buf());
    deliver(&mut app, index);
    // The document was there when the tags were read and is gone now.
    for e in &entries {
        std::fs::remove_file(&e.path).unwrap();
    }
    let link = app
        .doc
        .nodes
        .iter()
        .flat_map(|n| n.inlines.iter())
        .find_map(|i| i.link)
        .unwrap();
    update(&mut app, Action::LinkOpen(link.0));
    assert_eq!(app.path, "tags", "still on the tags document");
    assert!(
        app.note
            .as_deref()
            .unwrap_or_default()
            .contains("cannot open")
    );
    assert!(
        app.history.is_empty(),
        "a failed open is not somewhere to come back from"
    );
}

/// A folder's file names are whatever the reader's notes are called, and a
/// link to each one has to open it. The destination is read as a URL on the
/// way back in — split at `#`, copied if it looks like it has a scheme — so
/// a name with any of those in it needs to survive the round trip.
#[test]
fn every_document_on_the_page_opens_whatever_it_is_called() {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join("sub dir")).unwrap();
    let names = [
        "plain.md",
        "C# notes.md",
        "issue #12.md",
        "#inbox.md",
        "re: plan.md",
        "http:notes.md",
        "what?.md",
        "a&amp;b.md",
        "100%.md",
        "50%20off.md",
        "sub dir/x (1) [draft].md",
        "back\\slash <angle>.md",
    ];
    let mut index = Index::default();
    let mut entries = Vec::new();
    for (i, name) in names.iter().enumerate() {
        let path = d.path().join(name);
        std::fs::write(&path, format!("---\ntags: [t]\n---\n\nbody of {i}\n")).unwrap();
        let mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(i as u64);
        entries.push(Entry {
            path: path.clone(),
            mtime,
        });
        index.tagged.push(Tagged {
            path,
            mtime,
            tags: vec!["t".into()],
            title: None,
        });
    }
    index.read = names.len();

    let mut app = home(d.path(), entries, 80, 200);
    app.library_root = Some(d.path().to_path_buf());
    deliver(&mut app, index);
    assert_eq!(app.doc.links.len(), names.len());

    for link in 0..names.len() {
        // Newest first, so link 0 is the last name.
        let want = d.path().join(names[names.len() - 1 - link]);
        update(&mut app, Action::LinkOpen(u32::try_from(link).unwrap()));
        assert_eq!(
            app.file.as_deref(),
            Some(want.as_path()),
            "following the link to {:?}: {:?}",
            want.file_name().unwrap(),
            app.note
        );
        update(&mut app, Action::Back);
        assert_eq!(app.path, "tags");
    }

    // The links pane resolves the same destinations by its own route.
    update(&mut app, Action::ForwardToggle);
    let rows = app.forward.as_ref().unwrap().rows.clone();
    assert_eq!(rows.len(), names.len());
    for (row, name) in rows.iter().zip(names.iter().rev()) {
        assert_eq!(
            row.target.as_deref(),
            Some(d.path().join(name).as_path()),
            "the links pane's target for {name:?}"
        );
    }
}

/// A file name is untrusted text too, and unlike a tag it lands in the link
/// TARGET. A newline there used to end the destination and leave the rest
/// of the name as markdown.
#[test]
fn a_file_name_cannot_write_markdown_into_the_page() {
    let d = tempfile::tempdir().unwrap();
    let name = "x\n\n# PWNED\n\n[free money](mailto:evil@example.org)\n\n.md";
    let path = d.path().join(name);
    std::fs::write(&path, "---\ntags: [t]\n---\n\nthe real body\n").unwrap();
    let entry = Entry {
        path: path.clone(),
        mtime: SystemTime::UNIX_EPOCH,
    };
    let index = Index {
        tagged: vec![Tagged {
            path: path.clone(),
            mtime: SystemTime::UNIX_EPOCH,
            tags: vec!["t".into()],
            title: None,
        }],
        read: 1,
        capped: false,
    };
    let mut app = home(d.path(), vec![entry], 80, 60);
    app.library_root = Some(d.path().to_path_buf());
    deliver(&mut app, index);

    assert_eq!(headings(&app, 1), ["Tags"], "no heading from a file name");
    assert_eq!(app.doc.links.len(), 1, "and no link but the file's own");
    update(&mut app, Action::LinkOpen(0));
    assert_eq!(app.file.as_deref(), Some(path.as_path()), "{:?}", app.note);
}

#[test]
fn the_reading_note_does_not_outlive_the_request() {
    let d = tempfile::tempdir().unwrap();
    let (entries, index) = vault(d.path(), 3);

    // Withdrawn: the note that said it was reading goes with the request.
    let mut app = home(d.path(), entries.clone(), 60, 20);
    update(&mut app, Action::HomeTags);
    update(&mut app, Action::HomeMove(1));
    assert_eq!(app.home().unwrap().note, None);

    // Answered: coming back to the file list must not say it is still reading.
    let mut app = home(d.path(), entries, 60, 20);
    update(&mut app, Action::HomeTags);
    deliver(&mut app, index);
    update(&mut app, Action::CloseFile);
    assert!(app.is_home());
    assert_eq!(app.home().unwrap().note, None);
}

// --- from inside a document ---

use carrel_core::Document;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

const TAGGED: &str = "---\ntitle: A note about rust\ntags: [rust, topic1]\n---\n\n# Note\n\nbody\n";

/// A document opened by name: no file list in front of it or behind it.
fn reader_in(dir: &Path) -> App {
    let path = dir.join("note0.md");
    std::fs::write(&path, TAGGED).unwrap();
    let mut app = App::new("note0.md".into(), Document::parse(TAGGED), 70, 40);
    app.file = Some(path);
    app.library_root = Some(dir.to_path_buf());
    app.on_resize(70, 40);
    app
}

#[test]
fn every_tag_on_the_card_is_a_button_on_its_own_word() {
    let d = tempfile::tempdir().unwrap();
    let app = reader_in(d.path());
    let mut term = Terminal::new(TestBackend::new(70, 40)).unwrap();
    let mut painted = carrel::render::Painted::default();
    term.draw(|f| carrel::render::draw_full(f, &app, &mut painted, &mut Default::default()))
        .unwrap();
    let buf = term.backend().buffer().clone();
    let words: Vec<String> = painted
        .targets
        .as_slice()
        .iter()
        .filter(|t| matches!(t.action, Action::TagOpen(_)))
        .map(|t| {
            (t.zone.x..t.zone.x + t.zone.w)
                .map(|x| buf[(x, t.zone.y)].symbol())
                .collect()
        })
        .collect();
    assert_eq!(
        words,
        ["rust", "topic1"],
        "the tags, and not the `rust` in the title above them"
    );
    // Same word, different row: the `rust` that is a button is the one on
    // the tags line.
    for t in painted
        .targets
        .as_slice()
        .iter()
        .filter(|t| matches!(t.action, Action::TagOpen(_)))
    {
        let row: String = (0..70).map(|x| buf[(x, t.zone.y)].symbol()).collect();
        assert!(
            row.contains("tags"),
            "a tag button on the wrong row: {row:?}"
        );
    }
}

#[test]
fn clicking_a_tag_asks_for_the_page_at_that_tag_and_back_returns() {
    let d = tempfile::tempdir().unwrap();
    let (_, index) = vault(d.path(), 30);
    let mut app = reader_in(d.path());
    app.on_resize(70, 16);
    let at = u32::try_from(app.doc.text.find("topic1").unwrap()).unwrap();

    assert_eq!(update(&mut app, Action::TagOpen(at + 2)), Outcome::Redraw);
    assert_eq!(app.tags, Request::Wanted);
    assert_eq!(app.tags_focus.as_deref(), Some("topic1"));
    assert_eq!(app.note.as_deref(), Some("reading tags…"));
    assert!(
        app.file.is_some(),
        "still reading the document while it scans"
    );

    deliver(&mut app, index);
    assert_eq!(app.path, "tags");
    // Collapsed into a tag list — except the tag that was clicked, which is
    // open and at the top of the view.
    let top = app.layout.block_at_row(app.view.scroll_row);
    assert_eq!(app.doc.block_text(top), "topic1 (10)");
    let node = app.doc.node_for_block(top);
    assert!(!app.folded.contains(&node.id), "its section is expanded");
    assert!(!app.folded.is_empty(), "and the others are not");

    update(&mut app, Action::Back);
    assert_eq!(
        app.file.as_deref(),
        Some(d.path().join("note0.md").as_path()),
        "Back is the document the tag was clicked in"
    );
}

#[test]
fn a_click_that_is_not_on_a_tag_asks_for_nothing() {
    let d = tempfile::tempdir().unwrap();
    let mut app = reader_in(d.path());
    // In the title, which mentions `rust` but is not a tag; and in the body.
    let title = u32::try_from(app.doc.text.find("about rust").unwrap()).unwrap() + 7;
    let body = u32::try_from(app.doc.text.find("body").unwrap()).unwrap();
    for byte in [title, body, 9_999_999] {
        assert_eq!(update(&mut app, Action::TagOpen(byte)), Outcome::Idle);
        assert_eq!(app.tags, Request::Idle);
    }
}

#[test]
fn the_reader_can_ask_for_tags_with_no_file_list_behind_it() {
    let d = tempfile::tempdir().unwrap();
    let mut app = reader_in(d.path());
    assert_eq!(update(&mut app, Action::HomeTags), Outcome::Redraw);
    assert_eq!(app.tags, Request::Wanted);
    assert_eq!(app.tags_root().as_deref(), Some(d.path()));
    // Anything else withdraws it, here as on the file list.
    update(&mut app, Action::Scroll(carrel::action::Span::Line, 1));
    assert_eq!(app.tags, Request::Idle);
    assert_eq!(app.note, None);

    // A pipe has no folder: it says so rather than waiting forever.
    let mut piped = App::new("(stdin)".into(), Document::parse(TAGGED), 70, 20);
    update(&mut piped, Action::HomeTags);
    assert_eq!(piped.tags, Request::Idle);
    assert!(piped.note.as_deref().unwrap().contains("no folder"));
}
