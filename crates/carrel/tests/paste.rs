//! A paste is input text, never a stream of reader commands.
use carrel::action::{Action, Direction, SearchKey};
use carrel::app::{App, Mode, Outcome, Pane, paste_input, update};
use carrel_core::Document;

fn reader() -> App {
    App::new(
        "paste.md".into(),
        Document::parse("# Alpha\n\nCafé 👩‍💻 and tea.\n\n## Beta\n\nOther words."),
        80,
        24,
    )
}

#[test]
fn pasted_search_is_one_literal_line_and_does_not_accept_or_run_commands() {
    let mut app = reader();
    update(&mut app, Action::SearchOpen(Direction::Forward));
    paste_input(&mut app, "Café 👩‍💻\nq\n");
    assert!(matches!(&app.mode, Mode::Search { input, .. } if input == "Café 👩‍💻"));
    let matches = app.matches.as_ref().unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(
        matches.current, None,
        "a newline must not accept the search"
    );
    let range = &matches.ranges[0];
    assert_eq!(
        &app.doc.text[range.start as usize..range.end as usize],
        "Café 👩‍💻"
    );
    update(&mut app, Action::SearchKey(SearchKey::Accept));
    assert_eq!(app.matches.as_ref().unwrap().current, Some(0));
}

#[test]
fn pasted_filters_remain_filters_and_reset_their_selection() {
    let mut app = reader();
    update(&mut app, Action::HelpToggle);
    app.help.as_mut().unwrap().scroll = 20;
    paste_input(&mut app, "copy\rq");
    assert_eq!(app.help.as_ref().unwrap().filter, "copy");
    assert_eq!(app.help.as_ref().unwrap().scroll, 0);
    update(&mut app, Action::PaneClose);
    update(&mut app, Action::OutlineToggle);
    paste_input(&mut app, "Beta\r\nq");
    assert_eq!(app.outline.as_ref().unwrap().filter, "Beta");
    assert_eq!(app.outline_matches().len(), 1);
    assert_eq!(app.pane(), Some(Pane::Outline));
}

#[test]
fn paste_is_inert_without_a_visible_text_input() {
    let mut app = reader();
    let before = app.view.anchor;
    assert_eq!(paste_input(&mut app, "qGzz"), Outcome::Idle);
    assert_eq!(app.view.anchor, before);
    update(&mut app, Action::OutlineToggle);
    app.on_resize(1, 1);
    assert!(app.blocked_pane().is_some());
    assert_eq!(paste_input(&mut app, "hidden"), Outcome::Idle);
    assert_eq!(app.outline.as_ref().unwrap().filter, "");
}

#[test]
fn pasted_queries_have_a_bounded_size_and_drop_control_characters() {
    let mut app = reader();
    update(&mut app, Action::SearchOpen(Direction::Backward));
    paste_input(&mut app, &format!("\n\r\n{}\u{7}", "界".repeat(1000)));
    assert!(matches!(&app.mode, Mode::Search { input, .. } if input.chars().count() == 256));
    update(&mut app, Action::SearchKey(SearchKey::Cancel));
    update(&mut app, Action::SearchOpen(Direction::Forward));
    paste_input(&mut app, "Ca\u{7}fé");
    assert!(matches!(&app.mode, Mode::Search { input, .. } if input == "Café"));
    assert_eq!(app.matches.as_ref().unwrap().len(), 1);
}

#[test]
fn help_paste_on_home_has_the_same_line_and_size_limits_as_the_reader() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = App::new_home(dir.path().into(), vec![], 80, 24);
    update(&mut app, Action::HelpToggle);
    paste_input(&mut app, "copy\nq");
    assert_eq!(app.help.as_ref().unwrap().filter, "copy");
    update(&mut app, Action::HelpKey(SearchKey::Cancel));
    paste_input(&mut app, &"界".repeat(1000));
    assert_eq!(app.help.as_ref().unwrap().filter.chars().count(), 256);
}

#[test]
fn the_paste_limit_does_not_split_a_visible_character() {
    for ending in ["e\u{301}", "👩‍💻", "🇨🇦", "👍🏽"] {
        let mut app = reader();
        update(&mut app, Action::SearchOpen(Direction::Forward));
        let prefix = "x".repeat(255);
        paste_input(&mut app, &format!("{prefix}{ending}ignored"));
        assert!(
            matches!(&app.mode, Mode::Search { input, .. } if input == &prefix),
            "{ending:?}"
        );
    }
}

#[test]
fn a_pasted_tab_keeps_the_words_on_each_side_separate() {
    let mut app = reader();
    update(&mut app, Action::SearchOpen(Direction::Forward));
    paste_input(&mut app, "Other\twords");
    assert!(matches!(&app.mode, Mode::Search { input, .. } if input == "Other words"));
    assert_eq!(app.matches.as_ref().unwrap().len(), 1);
    update(&mut app, Action::SearchKey(SearchKey::Cancel));
    update(&mut app, Action::HelpToggle);
    paste_input(&mut app, "copy\tcode");
    assert_eq!(app.help.as_ref().unwrap().filter, "copy code");
}
