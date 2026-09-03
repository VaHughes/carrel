//! The lamplight footer's state: which lamp, which word, which hints.
//!
//! NO RATATUI — `scripts/check-discipline.sh` rule 6. A pure function of the
//! app, so the GTK frontend reuses it and the tests need no terminal. The
//! hint TABLES live in `keys.rs` beside the help tables, where the honesty
//! test can feed every hinted key back through the real dispatcher.

use crate::app::{App, Screen};
use crate::home::HomeMode;
use crate::keys;

#[derive(Debug)]
pub struct Footer {
    /// `╭` lit; the folded `╰` is painted by the status row when hidden.
    pub arm: char,
    /// `●` steady · `◉` focused typing · `◎` overlay up · `○` off.
    pub bulb: char,
    pub word: &'static str,
    pub hints: &'static [keys::Hint],
}

/// Precedence (spec §2): help > outline > typing a search > link selected >
/// matches live > reading; on home, help > picker > filter > search > browse.
#[must_use]
pub fn of(app: &App) -> Footer {
    let f = |bulb, word, hints| Footer {
        arm: '╭',
        bulb,
        word,
        hints,
    };
    if app.help.is_some() {
        return f('◎', "help", keys::HINT_HELP);
    }
    if let Screen::Home(h) = &app.screen {
        return match h.mode {
            HomeMode::Picker => f('◎', "choose", keys::HINT_HOME_PICKER),
            HomeMode::Filter => f('◉', "filter", keys::HINT_HOME_FILTER),
            HomeMode::Search => f('◉', "search", keys::HINT_HOME_SEARCH),
            // The invitation, on the screen a bare `carrel` opens on.
            HomeMode::Normal if app.first_run => f('●', "browse", keys::HINT_FIRST_RUN),
            HomeMode::Normal => f('●', "browse", keys::HINT_HOME_BROWSE),
        };
    }
    if app.outline.is_some() {
        return f('◎', "outline", keys::HINT_OUTLINE);
    }
    if app.searching() {
        return f('◉', "searching", keys::HINT_SEARCH_TYPING);
    }
    if app.selected_link.is_some() {
        return f('●', "link", keys::HINT_LINK);
    }
    if app.matches.is_some() {
        return f('●', "matches", keys::HINT_MATCHES);
    }
    // Following outranks ambient streaming: it is a mode the reader chose,
    // and its keys differ — detaching is the thing they will want next.
    if app.following {
        return f('◉', "following", keys::HINT_FOLLOWING);
    }
    // Ambient, not a mode: every mode above outranks it, and the keys are
    // the ordinary reading set — only the lamp says content is arriving.
    if app.streaming {
        return f('◉', "streaming", keys::HINT_STREAMING);
    }
    // Lowest precedence of all: every mode above knows something more
    // specific to say, and `streaming` in particular has `F`, which exists
    // only while a document is growing.
    if app.first_run {
        return f('●', "reading", keys::HINT_FIRST_RUN);
    }
    f('●', "reading", keys::HINT_READING)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{Action, Direction, SearchKey};
    use crate::app::update;
    use carrel_core::Document;

    fn reader() -> App {
        App::new("t.md".into(), Document::parse("# T\n\nneedle body"), 40, 12)
    }

    #[test]
    fn a_streaming_document_says_so_until_eof() {
        let mut a = reader();
        a.streaming = true;
        let f = of(&a);
        assert_eq!((f.bulb, f.word), ('◉', "streaming"));
        // Streaming used to show the plain reading hints — "same keys,
        // different weather". It shows its own now, because `F` exists ONLY
        // while a document is growing and the footer is the one place anyone
        // would find it.
        assert_eq!(f.hints, keys::HINT_STREAMING);
        assert!(f.hints.iter().any(|h| h.key == "F"));

        // A mode still outranks the ambient stream…
        update(&mut a, Action::SearchOpen(Direction::Forward));
        assert_eq!(of(&a).word, "searching");
        update(&mut a, Action::SearchKey(SearchKey::Cancel));

        // …and following outranks it, being a mode the reader chose.
        update(&mut a, Action::FollowToggle);
        assert_eq!((of(&a).bulb, of(&a).word), ('◉', "following"));
        update(&mut a, Action::FollowToggle);

        // …and EOF hands the lamp back to reading.
        a.streaming = false;
        assert_eq!(of(&a).word, "reading");
    }

    #[test]
    fn reader_states_pick_the_right_lamp_word_and_hints() {
        let mut a = reader();
        let f = of(&a);
        assert_eq!((f.bulb, f.word), ('●', "reading"));
        assert_eq!(f.hints, keys::HINT_READING);

        update(&mut a, Action::SearchOpen(Direction::Forward));
        assert_eq!((of(&a).bulb, of(&a).word), ('◉', "searching"));

        for c in "needle".chars() {
            update(&mut a, Action::SearchKey(SearchKey::Char(c)));
        }
        update(&mut a, Action::SearchKey(SearchKey::Accept));
        assert_eq!((of(&a).bulb, of(&a).word), ('●', "matches"));

        update(&mut a, Action::HelpToggle);
        assert_eq!(
            (of(&a).bulb, of(&a).word),
            ('◎', "help"),
            "help outranks all"
        );
    }

    #[test]
    fn the_first_run_invitation_shows_once_and_goes_when_you_act() {
        let mut a = reader();
        a.first_run = true;
        assert_eq!(of(&a).hints, keys::HINT_FIRST_RUN);
        assert_eq!(of(&a).word, "reading", "the lamp still says where you are");

        // Moving the pointer is not acting: the invitation is what tells you
        // moving it is worth anything.
        update(&mut a, Action::Hover((3, 4)));
        assert_eq!(of(&a).hints, keys::HINT_FIRST_RUN);

        update(&mut a, Action::Scroll(crate::action::Span::Line, 1));
        assert_eq!(of(&a).hints, keys::HINT_READING, "acting retires it");
    }

    #[test]
    fn the_invitation_reaches_the_home_screen_a_bare_carrel_opens_on() {
        let d = tempfile::tempdir().unwrap();
        let mut a = App::new_home(d.path().into(), vec![], 60, 16);
        a.first_run = true;
        assert_eq!(of(&a).hints, keys::HINT_FIRST_RUN);
        // …and never over a mode that has something more specific to say.
        update(&mut a, Action::HomeFilterMode);
        assert_eq!(of(&a).hints, keys::HINT_HOME_FILTER);
    }

    #[test]
    fn every_mode_outranks_the_invitation() {
        let mut a = reader();
        a.streaming = true;
        a.first_run = true;
        assert_eq!(
            of(&a).hints,
            keys::HINT_STREAMING,
            "`F` exists only while a document is growing; the invitation can wait"
        );
    }

    #[test]
    fn link_selection_outranks_matches() {
        let mut a = App::new(
            "t.md".into(),
            Document::parse("see [x](https://e.com) needle"),
            40,
            12,
        );
        update(&mut a, Action::SearchOpen(Direction::Forward));
        update(&mut a, Action::SearchKey(SearchKey::Char('n')));
        update(&mut a, Action::SearchKey(SearchKey::Accept));
        update(&mut a, Action::LinkStep(1));
        assert_eq!(of(&a).word, "link");
    }

    #[test]
    fn home_states_pick_the_right_lamp_word_and_hints() {
        let d = tempfile::tempdir().unwrap();
        let mut a = App::new_home(d.path().into(), vec![], 60, 16);
        assert_eq!((of(&a).bulb, of(&a).word), ('●', "browse"));
        update(&mut a, Action::HomeFilterMode);
        assert_eq!((of(&a).bulb, of(&a).word), ('◉', "filter"));
        update(&mut a, Action::HomeKey(SearchKey::Cancel));
        update(&mut a, Action::HomeSearchMode);
        assert_eq!((of(&a).bulb, of(&a).word), ('◉', "search"));
        update(&mut a, Action::HomeNormalMode);
        update(&mut a, Action::PickerOpen);
        let f = of(&a);
        assert_eq!((f.bulb, f.word), ('◎', "choose"));
        assert_eq!(f.hints, keys::HINT_HOME_PICKER);
        // The picker is an input: its hints must say so wherever the
        // highlight sits, because `j` types there rather than moving.
        let n = i32::try_from(a.home().unwrap().picker.roots.len()).unwrap();
        update(&mut a, Action::HomeMove(n));
        assert_eq!(of(&a).hints, keys::HINT_HOME_PICKER);
    }
}
