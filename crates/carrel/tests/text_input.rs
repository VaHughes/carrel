//! Backspace erases one visible character in every single-line input.
use carrel::action::{Action, Direction, SearchKey};
use carrel::app::{App, Mode, update};
use carrel::home::HomeMode;
use carrel_core::Document;

#[test]
fn reader_inputs_erase_complete_graphemes() {
    for ending in ["e\u{301}", "👩‍💻", "🇨🇦", "👍🏽", "界"] {
        for (open, key) in [
            (
                Action::SearchOpen(Direction::Forward),
                Action::SearchKey as fn(SearchKey) -> Action,
            ),
            (Action::HelpToggle, Action::HelpKey),
            (Action::OutlineToggle, Action::OutlineKey),
        ] {
            let mut app = App::new("input.md".into(), Document::parse("# x\n\nx"), 80, 24);
            update(&mut app, open);
            for c in format!("x{ending}").chars() {
                update(&mut app, key(SearchKey::Char(c)));
            }
            update(&mut app, key(SearchKey::Backspace));
            let input = match &app.mode {
                Mode::Search { input, .. } => input,
                Mode::Normal => app.help.as_ref().map_or_else(
                    || &app.outline.as_ref().unwrap().filter,
                    |help| &help.filter,
                ),
            };
            assert_eq!(input, "x", "{open:?}, {ending:?}");
            if app.searching() {
                assert_eq!(app.matches.as_ref().unwrap().len(), 2);
            }
        }
    }
}

#[test]
fn home_inputs_erase_complete_graphemes() {
    let dir = tempfile::tempdir().unwrap();
    for mode in [HomeMode::Filter, HomeMode::Search, HomeMode::Picker] {
        for ending in ["e\u{301}", "👩‍💻", "🇨🇦", "👍🏽", "界"] {
            let mut app = App::new_home(dir.path().into(), vec![], 80, 24);
            let home = app.home_mut().unwrap();
            home.mode = mode;
            match mode {
                HomeMode::Filter => home.filter = format!("x{ending}"),
                HomeMode::Search => home.query = format!("x{ending}"),
                HomeMode::Picker => home.picker.typed = format!("x{ending}"),
                HomeMode::Normal => unreachable!(),
            }
            update(&mut app, Action::HomeKey(SearchKey::Backspace));
            let home = app.home().unwrap();
            let input = match mode {
                HomeMode::Filter => &home.filter,
                HomeMode::Search => &home.query,
                HomeMode::Picker => &home.picker.typed,
                HomeMode::Normal => unreachable!(),
            };
            assert_eq!(input, "x", "{mode:?}, {ending:?}");
        }
    }
}
