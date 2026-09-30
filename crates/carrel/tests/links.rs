//! Links that lead nowhere, and links in a document opened by name.

use std::path::Path;

use carrel::action::Action;
use carrel::app::{App, update};
use carrel_core::{Document, LinkId};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Modifier;

const SRC: &str = "# Home\n\n\
[there](other.md) and [gone](missing.md) and [web](https://example.com/x)\n\n\
[up](#home) and [nowhere](#no-such-heading) and [line](#L3)\n\n\
[[other]] and [[absent]] and [spaced](my%20notes.md)\n\n\
[outside](../elsewhere/secret.md) and [sub](sub/deep.md#part)\n";

fn folder(dir: &Path) -> App {
    std::fs::write(dir.join("home.md"), SRC).unwrap();
    std::fs::write(dir.join("other.md"), "# Other\n").unwrap();
    std::fs::write(dir.join("my notes.md"), "spaced\n").unwrap();
    std::fs::create_dir(dir.join("sub")).unwrap();
    std::fs::write(dir.join("sub/deep.md"), "# Part\n").unwrap();
    // Built the way the binary builds a file named on the command line: by
    // hand, then told where it lives.
    let mut app = App::new("home.md".into(), Document::parse(SRC), 100, 30);
    app.file = Some(dir.join("home.md"));
    app.library_root = Some(dir.to_path_buf());
    app.index_links();
    app
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

#[test]
fn only_a_link_carrel_could_open_and_cannot_is_called_dead() {
    let d = tempfile::tempdir().unwrap();
    let app = folder(d.path());
    assert_eq!(
        dead(&app),
        ["#no-such-heading", "absent", "missing.md"],
        "a missing file, a missing heading and a missing note — and nothing else"
    );
}

#[test]
fn a_path_that_leaves_the_folder_is_never_probed_or_judged() {
    // `../elsewhere/secret.md` does not exist. It is still not called dead:
    // carrel does not go looking outside the folder on a document's say-so,
    // so it does not know.
    let d = tempfile::tempdir().unwrap();
    let app = folder(d.path());
    assert!(!dead(&app).iter().any(|l| l.contains("elsewhere")));

    // The same absent name INSIDE the folder is dead — the control.
    let mut app = app;
    app.reload_from("[inside](elsewhere/secret.md)\n");
    assert_eq!(dead(&app), ["elsewhere/secret.md"]);
}

#[test]
fn a_reload_judges_the_links_again() {
    let d = tempfile::tempdir().unwrap();
    let mut app = folder(d.path());
    assert!(dead(&app).contains(&"missing.md"));
    // The agent writes the file the plan was linking to.
    std::fs::write(d.path().join("missing.md"), "now here\n").unwrap();
    app.reload().unwrap();
    assert!(!dead(&app).contains(&"missing.md"));
}

#[test]
fn a_pipe_that_is_still_arriving_is_not_judged() {
    let mut app = App::new("(stdin)".into(), Document::parse("x"), 80, 20);
    app.streaming = true;
    app.reload_from("[gone](definitely-not-here.md)\n");
    assert!(app.dead_links.is_empty());
    app.streaming = false;
    app.reload_from("[gone](definitely-not-here.md)\n");
    assert_eq!(app.dead_links.len(), 1, "once it has all arrived, it is");
}

/// The bug the dead-link work walked into: a `[[note]]` in a document opened
/// by name was never resolved at all, so it could not be followed.
#[test]
fn a_wikilink_in_a_document_opened_by_name_can_be_followed() {
    let d = tempfile::tempdir().unwrap();
    let mut app = folder(d.path());
    let id = (0..app.doc.links.len())
        .map(|i| LinkId(i as u32))
        .find(|id| app.doc.is_wikilink(*id) && &*app.doc.links[id.0 as usize] == "other")
        .unwrap();
    update(&mut app, Action::LinkOpen(id.0));
    assert_eq!(
        app.file.as_deref(),
        Some(d.path().join("other.md").as_path()),
        "{:?}",
        app.note
    );
}

#[test]
fn a_dead_link_is_struck_through_and_a_live_one_is_not() {
    let d = tempfile::tempdir().unwrap();
    let app = folder(d.path());
    let (cols, rows) = (100u16, 30u16);
    let mut term = Terminal::new(TestBackend::new(cols, rows)).unwrap();
    let mut painted = carrel::render::Painted::default();
    term.draw(|f| carrel::render::draw_full(f, &app, &mut painted, &mut Default::default()))
        .unwrap();
    let buf = term.backend().buffer().clone();

    let struck = |label: &str| {
        let id = app
            .doc
            .nodes
            .iter()
            .flat_map(|n| n.inlines.iter())
            .find(|i| &app.doc.text[i.doc.start as usize..i.doc.end as usize] == label)
            .and_then(|i| i.link)
            .unwrap_or_else(|| panic!("no link labelled {label}"));
        let zone = painted
            .targets
            .as_slice()
            .iter()
            .find(|t| t.action == Action::LinkOpen(id.0))
            .map(|t| t.zone)
            .unwrap_or_else(|| panic!("{label} is not painted"));
        (zone.x..zone.x + zone.w).all(|x| {
            buf[(x, zone.y)]
                .style()
                .add_modifier
                .contains(Modifier::CROSSED_OUT)
        })
    };
    for label in ["gone", "nowhere", "absent"] {
        assert!(struck(label), "{label} leads nowhere and must say so");
    }
    for label in [
        "there", "web", "up", "line", "other", "spaced", "outside", "sub",
    ] {
        assert!(!struck(label), "{label} must not be struck through");
    }
    // A dead link is not handed to the terminal as a hyperlink either.
    assert!(
        !painted.links.iter().any(|l| l.url.contains("missing.md")),
        "{:?}",
        painted.links.iter().map(|l| &l.url).collect::<Vec<_>>()
    );
    assert!(painted.links.iter().any(|l| l.url.contains("other.md")));
}
