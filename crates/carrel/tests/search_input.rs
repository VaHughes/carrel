//! Search input and feedback must describe the operation being performed.
use carrel::action::{Action, Direction, SearchKey};
use carrel::app::{App, update};
use carrel_core::Document;
use ratatui::{Terminal, backend::TestBackend};

fn app() -> App {
    App::new(
        "search.md".into(),
        Document::parse("alpha beta alpha beta"),
        80,
        24,
    )
}

fn frame(app: &App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|f| {
            carrel::render::draw_full(
                f,
                app,
                &mut carrel::render::Painted::default(),
                &mut std::collections::HashMap::new(),
            );
        })
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect()
}

#[test]
fn backward_search_has_a_backward_prompt() {
    let mut app = app();
    update(&mut app, Action::SearchOpen(Direction::Backward));
    for c in "alpha".chars() {
        update(&mut app, Action::SearchKey(SearchKey::Char(c)));
    }
    let text = frame(&app);
    assert!(text.contains("?alpha"), "{text}");
    assert!(!text.contains("/alpha"));
}

fn type_query(app: &mut App, query: &str) {
    update(app, Action::SearchOpen(Direction::Forward));
    carrel::app::paste_input(app, query);
}

fn accept(app: &mut App, query: &str) {
    type_query(app, query);
    update(app, Action::SearchKey(SearchKey::Accept));
}

fn query(app: &App) -> &str {
    let carrel::app::Mode::Search { input, .. } = &app.mode else {
        panic!("the search prompt must stay open")
    };
    input
}

fn recall(app: &mut App, direction: Direction) {
    update(app, Action::SearchKey(SearchKey::Recall(direction)));
}

#[test]
fn recent_searches_restore_an_unfinished_query_without_wrapping() {
    let mut app = app();
    accept(&mut app, "alpha");
    accept(&mut app, "beta");
    type_query(&mut app, "unfinished");
    recall(&mut app, Direction::Forward);
    assert_eq!(query(&app), "unfinished");
    recall(&mut app, Direction::Backward);
    assert_eq!(query(&app), "beta");
    assert_eq!(app.matches.as_ref().unwrap().len(), 2);
    recall(&mut app, Direction::Backward);
    assert_eq!(query(&app), "alpha");
    recall(&mut app, Direction::Backward);
    assert_eq!(query(&app), "alpha", "the oldest entry is a stop");
    app.on_resize(24, 8);
    recall(&mut app, Direction::Forward);
    assert_eq!(query(&app), "beta");
    recall(&mut app, Direction::Forward);
    assert_eq!(query(&app), "unfinished");
    assert_eq!(app.matches.as_ref().unwrap().len(), 0);
}

#[test]
fn cancelled_searches_are_not_remembered_and_duplicates_move_to_the_front() {
    let mut app = app();
    accept(&mut app, "alpha");
    accept(&mut app, "beta");
    accept(&mut app, "alpha");
    type_query(&mut app, "cancelled");
    update(&mut app, Action::SearchKey(SearchKey::Cancel));
    type_query(&mut app, "");
    recall(&mut app, Direction::Backward);
    assert_eq!(query(&app), "alpha");
    recall(&mut app, Direction::Backward);
    assert_eq!(query(&app), "beta");
    recall(&mut app, Direction::Backward);
    assert_eq!(query(&app), "beta", "duplicates do not take another slot");
}

#[test]
fn editing_a_recalled_query_keeps_the_edit_when_browsing_again() {
    let mut app = app();
    accept(&mut app, "alpha");
    type_query(&mut app, "draft");
    recall(&mut app, Direction::Backward);
    update(&mut app, Action::SearchKey(SearchKey::Backspace));
    assert_eq!(query(&app), "alph");
    recall(&mut app, Direction::Backward);
    assert_eq!(query(&app), "alpha");
    recall(&mut app, Direction::Forward);
    assert_eq!(query(&app), "alph");
}

#[test]
fn history_is_bounded_and_queries_work_in_the_next_document() {
    let mut app = app();
    for i in 0..25 {
        accept(&mut app, &format!("query{i}"));
    }
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("other.md");
    std::fs::write(&file, "query24 query24").unwrap();
    app.open_path(&file).unwrap();
    type_query(&mut app, "");
    recall(&mut app, Direction::Backward);
    assert_eq!(query(&app), "query24");
    assert_eq!(app.matches.as_ref().unwrap().len(), 2);
    for _ in 0..30 {
        recall(&mut app, Direction::Backward);
    }
    assert_eq!(query(&app), "query5", "only twenty queries are kept");
}

#[test]
fn search_arrows_recall_queries_but_reader_arrows_still_scroll() {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut keys = carrel::keys::Keys::new();
    for (code, direction) in [
        (KeyCode::Up, Direction::Backward),
        (KeyCode::Down, Direction::Forward),
    ] {
        let key = KeyEvent::new(code, KeyModifiers::NONE);
        assert_eq!(
            keys.map(key, true),
            Some(Action::SearchKey(SearchKey::Recall(direction)))
        );
        assert!(matches!(keys.map(key, false), Some(Action::Scroll(..))));
    }
}

#[test]
fn clickable_history_hints_recall_in_the_current_search_direction() {
    let mut app = app();
    accept(&mut app, "alpha");
    update(&mut app, Action::SearchOpen(Direction::Backward));
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
    let mut painted = carrel::render::Painted::default();
    app.on_resize(100, 24);
    terminal
        .draw(|f| {
            carrel::render::draw_full(f, &app, &mut painted, &mut std::collections::HashMap::new());
        })
        .unwrap();
    let recall_action = Action::SearchKey(SearchKey::Recall(Direction::Backward));
    assert!(
        painted
            .targets
            .as_slice()
            .iter()
            .any(|target| target.action == recall_action)
    );
    update(&mut app, recall_action);
    assert_eq!(query(&app), "alpha");
    update(&mut app, Action::SearchKey(SearchKey::Accept));
    assert_eq!(
        app.matches.as_ref().unwrap().current,
        Some(1),
        "backward search lands on the last match"
    );
}

#[test]
fn backward_wrap_reports_the_end_of_the_document() {
    let mut app = app();
    accept(&mut app, "alpha");
    update(&mut app, Action::MatchStep(-1));
    assert_eq!(app.matches.as_ref().unwrap().current, Some(1));
    assert_eq!(app.note.as_deref(), Some("wrapped round to the end"));
}

#[test]
fn cancelling_search_after_a_reload_keeps_a_valid_position_in_the_new_document() {
    let mut app = App::new(
        "live.md".into(),
        Document::parse(&"paragraph\n\n".repeat(50)),
        40,
        8,
    );
    update(&mut app, Action::GoToEnd);
    assert!(app.view.anchor > 100);
    type_query(&mut app, "paragraph");
    app.reload_from("界");
    assert_eq!(
        query(&app),
        "paragraph",
        "the active query survives a rewrite"
    );
    update(&mut app, Action::SearchKey(SearchKey::Cancel));
    assert!(app.view.anchor as usize <= app.doc.text.len());
    assert!(app.doc.text.is_char_boundary(app.view.anchor as usize));
    app.on_resize(20, 4);
    assert!(app.view.anchor as usize <= app.doc.text.len());
}

#[test]
fn reload_does_not_leave_a_position_inside_a_multibyte_character() {
    use carrel::action::Span;
    let mut app = App::new("live.md".into(), Document::parse("a\n\nb\n\nc\n\nd"), 10, 2);
    update(&mut app, Action::Scroll(Span::Line, 1));
    let replacement = "界".repeat(20);
    assert!(
        !Document::parse(&replacement)
            .text
            .is_char_boundary(app.view.anchor as usize),
        "the fixture must rewrite the old position to a continuation byte"
    );
    app.reload_from(&replacement);
    assert!(app.doc.text.is_char_boundary(app.view.anchor as usize));
}

#[test]
fn a_live_rewrite_keeps_the_selected_occurrence_when_earlier_matches_change() {
    let original = "needle first\n\nneedle selected\n\nneedle later\n";
    for replacement in [
        format!("needle inserted\n\n{original}"),
        original.replace("needle first\n\n", ""),
    ] {
        let mut app = App::new("live.md".into(), Document::parse(original), 80, 10);
        update(&mut app, Action::SearchOpen(Direction::Forward));
        carrel::app::paste_input(&mut app, "needle");
        update(&mut app, Action::SearchKey(SearchKey::Accept));
        update(&mut app, Action::MatchStep(1));
        app.reload_from(&replacement);
        let matches = app.matches.as_ref().unwrap();
        let selected = matches.ranges[matches.current.unwrap()].start as usize;
        assert!(app.doc.text[selected..].starts_with("needle selected"));
        update(&mut app, Action::MatchStep(1));
        let matches = app.matches.as_ref().unwrap();
        let next = matches.ranges[matches.current.unwrap()].start as usize;
        assert!(app.doc.text[next..].starts_with("needle later"));
    }
}
