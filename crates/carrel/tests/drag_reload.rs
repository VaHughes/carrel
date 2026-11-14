//! A drag in progress outlives a re-parse: a streaming pipe re-parses on
//! every chunk, and a live reload on every save, and both used to drop the
//! selection mid-drag so the release copied nothing.
use carrel::action::Action;
use carrel::app::{App, update};
use carrel_core::Document;

fn byte(a: &App, needle: &str) -> u32 {
    u32::try_from(a.doc.text.find(needle).unwrap()).unwrap()
}

/// Press on "alpha", drag to "beta", a chunk arrives, drag on, release.
#[test]
fn a_drag_survives_a_chunk_arriving_and_copies_on_release() {
    let src = "alpha one\n\nbeta two\n\ngamma three\n";
    let mut a = App::new("t.md".into(), Document::parse(src), 60, 20);
    let al = byte(&a, "alpha");
    update(&mut a, Action::SelectAnchor((al, al + 1)));
    let b = byte(&a, "beta");
    update(&mut a, Action::SelectDrag((b, b + 4)));
    a.reload_from(&format!("{src}\nadded line\n"));
    assert!(a.selection.is_some(), "the highlight stays while dragging");
    let g = byte(&a, "gamma");
    update(&mut a, Action::SelectDrag((g, g + 5)));
    update(&mut a, Action::SelectRelease);
    let copied = a.clipboard.as_deref().unwrap_or_default();
    assert!(
        copied.starts_with("alpha one") && copied.ends_with("gamma"),
        "{copied:?}"
    );
}

/// An edit above the selection moves its bytes; it follows its text.
#[test]
fn a_drag_follows_its_text_through_an_edit_above() {
    let src = "intro\n\nalpha one\n\nbeta two\n";
    let mut a = App::new("t.md".into(), Document::parse(src), 60, 20);
    let al = byte(&a, "alpha");
    update(&mut a, Action::SelectAnchor((al, al + 1)));
    let b = byte(&a, "beta");
    update(&mut a, Action::SelectDrag((b, b + 4)));
    a.reload_from("a much longer introduction than before\n\nalpha one\n\nbeta two\n");
    update(&mut a, Action::SelectRelease);
    assert_eq!(a.clipboard.as_deref(), Some("alpha one\nbeta"));
}

/// The selected text itself gone: dropped, as before, rather than guessed.
#[test]
fn a_drag_whose_text_is_gone_copies_nothing() {
    let src = "alpha one\n\nbeta two\n";
    let mut a = App::new("t.md".into(), Document::parse(src), 60, 20);
    let al = byte(&a, "alpha");
    update(&mut a, Action::SelectAnchor((al, al + 1)));
    let b = byte(&a, "beta");
    update(&mut a, Action::SelectDrag((b, b + 4)));
    a.reload_from("something else entirely\n");
    update(&mut a, Action::SelectRelease);
    assert!(a.clipboard.is_none(), "{:?}", a.clipboard.as_deref());
}
