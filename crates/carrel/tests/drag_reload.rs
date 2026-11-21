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

/// Press on "alpha", drag to `back` bytes short of that paragraph's end,
/// reload to `new_src`, release: what was copied.
fn drag_to_the_paragraph_end(new_src: &str, back: u32) -> Option<String> {
    let src = "intro\n\nalpha one\n\nbeta two\n";
    let mut a = App::new("t.md".into(), Document::parse(src), 60, 20);
    let al = byte(&a, "alpha");
    update(&mut a, Action::SelectAnchor((al, al + 1)));
    let e = byte(&a, "one") + 3 - back;
    update(&mut a, Action::SelectDrag((e - 1, e)));
    a.reload_from(new_src);
    update(&mut a, Action::SelectRelease);
    a.clipboard.as_deref().map(str::to_string)
}

/// A selection ends at an exclusive byte, and at a paragraph's end that
/// byte belongs to the NEXT block: the drag was dropped when a paragraph it
/// never touched changed.
#[test]
fn a_drag_to_a_paragraph_end_does_not_depend_on_the_next_paragraph() {
    // Controls: the next block intact, and a selection ending a byte short.
    assert_eq!(
        drag_to_the_paragraph_end("a longer intro\n\nalpha one\n\nbeta two\n", 0).as_deref(),
        Some("alpha one")
    );
    assert_eq!(
        drag_to_the_paragraph_end("a longer intro\n\nalpha one\n\ngamma\n", 1).as_deref(),
        Some("alpha on")
    );
    assert_eq!(
        drag_to_the_paragraph_end("a longer intro\n\nalpha one\n\ngamma\n", 0).as_deref(),
        Some("alpha one")
    );
    // The next block gone altogether, so the selection ends the text.
    assert_eq!(
        drag_to_the_paragraph_end("a longer intro\n\nalpha one\n", 0).as_deref(),
        Some("alpha one")
    );
}

/// A heading click remembers its byte so a second press can turn the click
/// into a double click. A re-parse between the two presses left that byte
/// pointing into other text: the second press toggled whatever heading
/// stood there now and selected a word nobody clicked.
#[test]
fn a_double_click_split_by_a_reload_acts_where_it_was_pressed() {
    let src = "# One\n\nalpha beta\n\n# Two\n\ngamma\n";
    let mut a = App::new("t.md".into(), Document::parse(src), 60, 20);
    let b = byte(&a, "Two");
    update(&mut a, Action::SelectAnchor((b, b + 1)));
    update(&mut a, Action::SelectRelease);
    assert_eq!(a.folded.len(), 1, "the click collapses Two");
    a.reload_from("# One\n\nalpha\n\n# Other\n\nwords here\n\n# Two\n\ngamma\n");
    let folded = a.folded.len();
    let w = byte(&a, "words");
    update(&mut a, Action::SelectWord(w));
    let sel = a
        .selection
        .clone()
        .map(|r| a.doc.text[r.start as usize..r.end as usize].to_string());
    assert_eq!(sel.as_deref(), Some("words"));
    assert_eq!(a.folded.len(), folded, "no fold toggled at the old byte");
}

/// The control: with no reload between the presses, the double click
/// selects the heading word and undoes the click's fold.
#[test]
fn a_double_click_on_a_heading_selects_its_word_and_unfolds() {
    let src = "# One\n\nalpha beta\n\n# Two\n\ngamma\n";
    let mut a = App::new("t.md".into(), Document::parse(src), 60, 20);
    let b = byte(&a, "Two");
    update(&mut a, Action::SelectAnchor((b, b + 1)));
    update(&mut a, Action::SelectRelease);
    update(&mut a, Action::SelectWord(b));
    let sel = a
        .selection
        .clone()
        .map(|r| a.doc.text[r.start as usize..r.end as usize].to_string());
    assert_eq!(sel.as_deref(), Some("Two"));
    assert!(a.folded.is_empty());
}
