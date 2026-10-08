//! The lamplight footer's state: which lamp, which word, which hints.
//!
//! NO RATATUI — `scripts/check-discipline.sh` rule 6. A pure function of the
//! app, so the GTK frontend reuses it and the tests need no terminal. The
//! hint TABLES live in `keys.rs` beside the help tables, where the honesty
//! test can feed every hinted key back through the real dispatcher.

use crate::app::{App, Pane, Screen};
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
    // The pane that owns the screen owns the footer too: its chips are the
    // pane's own buttons, never a reader command for the page beside it.
    // **Exhaustive on purpose** — a new `Pane` fails to compile here until
    // it is given hints, so it cannot inherit the reading row's `/ search`,
    // `o outline` and `h help`, which are not the pane's to offer.
    // The notes pane, a menu and the lightbox did exactly that.
    match app.pane() {
        Some(Pane::Help) => return f('◎', "help", keys::HINT_HELP),
        Some(Pane::Settings) => return f('◎', "settings", keys::HINT_SETTINGS),
        Some(Pane::Marks) => return f('◎', "bookmarks", keys::HINT_MARKS),
        Some(Pane::Backlinks) => return f('◎', "links here", keys::HINT_BACKLINKS),
        Some(Pane::Forward) => return f('◎', "links", keys::HINT_FORWARD),
        Some(Pane::Info) => return f('◎', "document", keys::HINT_INFO),
        Some(Pane::Outline) => return f('◎', "outline", keys::HINT_OUTLINE),
        Some(Pane::Search) => return f('◉', "searching", keys::HINT_SEARCH_TYPING),
        Some(Pane::Picker) => return f('◎', "choose", keys::HINT_HOME_PICKER),
        Some(Pane::Menu) => return f('◎', "menu", keys::HINT_MENU),
        Some(Pane::Lightbox) => return f('◎', "image", keys::HINT_LIGHTBOX),
        Some(Pane::Notes) if app.notes.draft.is_some() => {
            return f('◉', "writing", keys::HINT_NOTE_DRAFT);
        }
        Some(Pane::Notes) => return f('◎', "notes", keys::HINT_NOTES),
        None => {}
    }
    if let Screen::Home(h) = &app.screen {
        return match h.mode {
            // Answered above, as the pane it is.
            HomeMode::Picker => f('◎', "choose", keys::HINT_HOME_PICKER),
            HomeMode::Filter => f('◉', "filter", keys::HINT_HOME_FILTER),
            HomeMode::Search => f('◉', "search", keys::HINT_HOME_SEARCH),
            // The invitation, on the screen a bare `carrel` opens on.
            HomeMode::Normal if app.first_run => f('●', "browse", keys::HINT_FIRST_RUN),
            HomeMode::Normal => f('●', "browse", keys::HINT_HOME_BROWSE),
        };
    }
    if app.selected_link.is_some() {
        return f('●', "link picked", keys::HINT_LINK);
    }
    if app.matches.is_some() {
        return f('●', "found", keys::HINT_MATCHES);
    }
    // Following outranks ambient streaming: it is a mode the reader chose,
    // and its keys differ — detaching is the thing they will want next.
    if app.following {
        return f('◉', "keeping up", keys::HINT_FOLLOWING);
    }
    // Ambient, not a mode: every mode above outranks it, and the keys are
    // the ordinary reading set — only the lamp says content is arriving.
    if app.streaming {
        return f('◉', "arriving", keys::HINT_STREAMING);
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
pub(crate) mod tests {
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
        assert_eq!((f.bulb, f.word), ('◉', "arriving"));
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
        assert_eq!((of(&a).bulb, of(&a).word), ('◉', "keeping up"));
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
        assert_eq!((of(&a).bulb, of(&a).word), ('●', "found"));

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
        assert_eq!(of(&a).word, "link picked");
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

    /// Every face of every pane `App::pane` can answer, each with the state
    /// that puts it up. **Exhaustive on purpose**: the `next` walk names
    /// every `Pane`, so a new one fails to compile here until it is given a
    /// way to open — and with it, a place in the footer guard below.
    pub(crate) fn every_pane_face() -> Vec<(crate::app::Pane, &'static str, App)> {
        use crate::app::Pane;
        const fn next(p: Option<Pane>) -> Option<Pane> {
            match p {
                None => Some(Pane::Lightbox),
                Some(Pane::Lightbox) => Some(Pane::Notes),
                Some(Pane::Notes) => Some(Pane::Menu),
                Some(Pane::Menu) => Some(Pane::Help),
                Some(Pane::Help) => Some(Pane::Settings),
                Some(Pane::Settings) => Some(Pane::Outline),
                Some(Pane::Outline) => Some(Pane::Marks),
                Some(Pane::Marks) => Some(Pane::Backlinks),
                Some(Pane::Backlinks) => Some(Pane::Forward),
                Some(Pane::Forward) => Some(Pane::Info),
                Some(Pane::Info) => Some(Pane::Search),
                Some(Pane::Search) => Some(Pane::Picker),
                Some(Pane::Picker) => None,
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("one.md");
        let src = "Some prose to annotate.\n\n# One\n\nSee [two](two.md).\n\n\
                   ![alt](pic.png)\n\n## Two\n";
        std::fs::write(&file, src).unwrap();
        let reader = |steps: &[Action]| {
            let mut a = App::new("one.md".into(), Document::parse(src), 60, 20);
            a.file = Some(file.clone());
            update(&mut a, Action::MarkToggle);
            for &s in steps {
                update(&mut a, s);
            }
            a
        };
        let home = |steps: &[Action]| {
            let mut a = App::new_home(dir.path().into(), vec![], 60, 20);
            for &s in steps {
                update(&mut a, s);
            }
            a
        };
        let menu = Action::MenuOpen {
            at: (2, 2),
            byte: None,
        };
        let mut faces = Vec::new();
        let mut at = next(None);
        while let Some(p) = at {
            let mut add = |face, a: App| faces.push((p, face, a));
            match p {
                Pane::Lightbox => add("lightbox", reader(&[Action::ImageOpen(None)])),
                Pane::Notes => {
                    add("notes list", reader(&[Action::NotesToggle]));
                    add("note draft", reader(&[Action::NoteEdit]));
                }
                Pane::Menu => {
                    add("menu", reader(&[menu]));
                    add("home menu", home(&[menu]));
                }
                Pane::Help => {
                    add("help", reader(&[Action::HelpToggle]));
                    add("home help", home(&[Action::HelpToggle]));
                }
                Pane::Settings => {
                    add("settings", reader(&[Action::SettingsToggle]));
                    add("home settings", home(&[Action::SettingsToggle]));
                }
                Pane::Outline => add("outline", reader(&[Action::OutlineToggle])),
                Pane::Marks => add("bookmarks", reader(&[Action::MarkListToggle])),
                Pane::Backlinks => add("backlinks", reader(&[Action::BacklinksToggle])),
                Pane::Forward => add("forward", reader(&[Action::ForwardToggle])),
                Pane::Info => add("info", reader(&[Action::InfoToggle])),
                Pane::Search => add("search", reader(&[Action::SearchOpen(Direction::Forward)])),
                Pane::Picker => add("picker", home(&[Action::PickerOpen])),
            }
            at = next(at);
        }
        faces
    }

    /// A footer chip is a button, and a button that does nothing is a lie
    /// (the honesty test's rule, from the pointer's side). With a pane up,
    /// its footer chips sit above the scrim as the pane's own buttons, so
    /// a chip offering another pane's opener — which is what the reading
    /// hints (`/ search`, `o outline`, `h help`) are — would act on the
    /// page beside the pane from a row that names the pane. Each pane's
    /// footer must offer only what that pane does. Walks EVERY pane (`every_pane_face`), not a
    /// list kept beside them: the list this used to keep skipped the notes
    /// pane, the menu and the folder dialog, and the first two offered the
    /// reading row's refused chips.
    #[test]
    fn no_footer_chip_offers_what_the_open_pane_refuses() {
        use crate::app::Opens;
        let mut checked = 0;
        for (pane, face, a) in every_pane_face() {
            assert_eq!(a.pane(), Some(pane), "{face} opened");
            for h in of(&a).hints {
                let Some(click) = h.click else { continue };
                assert!(
                    Pane::opened_by(click).is_none_or(|o| o == Opens::Pane(pane)),
                    "{face} is up, but its footer offers `{} {}`, which it refuses",
                    h.key,
                    h.label
                );
                checked += 1;
            }
        }
        assert!(checked > 20, "the footers must be walked, got {checked}");
    }
}
