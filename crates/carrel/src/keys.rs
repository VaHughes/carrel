//! Key events to [`Action`]s, with vim's count register.
//!
//! Binding lives here and nowhere else — discipline #4: a GTK frontend
//! produces the same [`Action`]s from entirely different input, so nothing
//! downstream of this file knows a keyboard exists.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::action::{Action, Direction, Edge, SearchKey, Span, Where};
use crate::home::HomeMode;

/// Vim's pending input state: a count, and the one-key prefixes `g` and `z`.
#[derive(Debug, Default)]
pub struct Keys {
    count: Option<u32>,
    pending_g: bool,
    pending_z: bool,
}

impl Keys {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Take the pending count, defaulting to 1, and clear it.
    fn take(&mut self) -> i32 {
        i32::try_from(self.count.take().unwrap_or(1)).unwrap_or(i32::MAX)
    }

    fn clear(&mut self) {
        self.count = None;
        self.pending_g = false;
        self.pending_z = false;
    }

    /// Map one key press. `searching` selects the search-prompt binding set.
    pub fn map(&mut self, key: KeyEvent, searching: bool) -> Option<Action> {
        if searching {
            return Self::map_search(key);
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        // Two-key prefixes resolve before anything else.
        if self.pending_g {
            self.pending_g = false;
            return match key.code {
                KeyCode::Char('g') => Some(Action::GoToStart),
                _ => None,
            };
        }
        if self.pending_z {
            self.pending_z = false;
            return match key.code {
                KeyCode::Char('z') => Some(Action::Recenter(Where::Middle)),
                KeyCode::Char('t') => Some(Action::Recenter(Where::Top)),
                KeyCode::Char('b') => Some(Action::Recenter(Where::Bottom)),
                _ => None,
            };
        }

        match key.code {
            // Ctrl chords first: Ctrl-B must not fall through to plain `b`.
            KeyCode::Char('c') if ctrl => Some(Action::Quit),
            KeyCode::Char('Q') => Some(Action::Quit),
            KeyCode::Char('T') => Some(Action::ThemeCycle),
            KeyCode::Char('t') => Some(Action::TableToggle),
            KeyCode::Char('d') if ctrl => Some(Action::Scroll(Span::HalfPage, self.take())),
            KeyCode::Char('u') if ctrl => Some(Action::Scroll(Span::HalfPage, -self.take())),
            KeyCode::Char('f') if ctrl => Some(Action::Scroll(Span::Page, self.take())),
            KeyCode::Char('b') if ctrl => Some(Action::Scroll(Span::Page, -self.take())),
            KeyCode::Char('e') if ctrl => Some(Action::Scroll(Span::Line, self.take())),
            KeyCode::Char('y') if ctrl => Some(Action::Scroll(Span::Line, -self.take())),

            // `0` extends an open count and is otherwise ignored: with no
            // horizontal scroll it has no "start of line" job to compete with.
            KeyCode::Char(c @ '0'..='9') if c != '0' || self.count.is_some() => {
                let d = u32::from(c as u8 - b'0');
                self.count = Some(self.count.unwrap_or(0).saturating_mul(10).saturating_add(d));
                None
            }

            KeyCode::Char('q') => Some(Action::CloseFile),
            KeyCode::Char('h') | KeyCode::F(1) => Some(Action::HelpToggle),
            KeyCode::Char('H') => Some(Action::HintsToggle),
            KeyCode::Char('m') => Some(Action::RenderedToggle),

            KeyCode::Char('j') | KeyCode::Down => Some(Action::Scroll(Span::Line, self.take())),
            KeyCode::Char('k') | KeyCode::Up => Some(Action::Scroll(Span::Line, -self.take())),
            KeyCode::Char(' ') | KeyCode::PageDown => Some(Action::Scroll(Span::Page, self.take())),
            KeyCode::Char('b') | KeyCode::PageUp => Some(Action::Scroll(Span::Page, -self.take())),

            KeyCode::Char('g') => {
                self.pending_g = true;
                None
            }
            KeyCode::Char('z') => {
                self.pending_z = true;
                None
            }
            KeyCode::Home => Some(Action::GoToStart),
            // Vim counts rows from 1; the row index is 0-based, so `1G` and
            // `gg` agree and `0G` cannot underflow.
            KeyCode::Char('G') | KeyCode::End => match self.count.take() {
                Some(n) => Some(Action::GoToRow(n.saturating_sub(1))),
                None => Some(Action::GoToEnd),
            },

            KeyCode::Char('}') => Some(Action::BlockStep(self.take())),
            KeyCode::Char('{') => Some(Action::BlockStep(-self.take())),

            KeyCode::Char('/') => Some(Action::SearchOpen(Direction::Forward)),
            KeyCode::Char('?') => Some(Action::SearchOpen(Direction::Backward)),
            KeyCode::Char('n') => Some(Action::MatchStep(self.take())),
            KeyCode::Char('N') => Some(Action::MatchStep(-self.take())),

            KeyCode::Tab => Some(Action::LinkStep(self.take())),
            KeyCode::BackTab => Some(Action::LinkStep(-self.take())),
            KeyCode::Enter => Some(Action::LinkFollow),
            KeyCode::Char('o') if ctrl => Some(Action::Back),
            KeyCode::Char('o') => Some(Action::OutlineToggle),

            KeyCode::Esc => {
                self.clear();
                Some(Action::Dismiss)
            }
            _ => None,
        }
    }

    /// Forget every pending prefix and count.
    ///
    /// Called on screen transitions: a lone `g` armed on the home screen must
    /// not swallow the reader's first keystroke.
    pub fn reset(&mut self) {
        self.clear();
    }

    /// Home-screen bindings. Telescope's model: filter mode types, normal mode
    /// is vim, and `Ctrl-N`/`Ctrl-P` and the arrows move in both.
    /// `typing_path` is true while the picker's `Other…` row is highlighted:
    /// printable keys then edit the path, so `j`/`k` must type, not move —
    /// a path like `/home/jay` is untypeable otherwise. Arrows and Ctrl-N/P still move.
    pub fn map_home(&mut self, key: KeyEvent, mode: HomeMode, typing_path: bool) -> Option<Action> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        // A pending `g` consumes this key, exactly as the reader's map() does —
        // otherwise an abandoned prefix silently eats a later keystroke.
        if self.pending_g {
            self.pending_g = false;
            return match key.code {
                KeyCode::Char('g') => Some(Action::HomeGo(Edge::First)),
                _ => None,
            };
        }

        // Movement and commit work identically in every mode. So does F1 —
        // a function key can open help even while typing into the filter.
        match key.code {
            KeyCode::F(1) => return Some(Action::HelpToggle),
            KeyCode::Char('n') if ctrl => return Some(Action::HomeMove(1)),
            KeyCode::Char('p') if ctrl => return Some(Action::HomeMove(-1)),
            KeyCode::Char('c') if ctrl => return Some(Action::Quit),
            KeyCode::Down => return Some(Action::HomeMove(1)),
            KeyCode::Up => return Some(Action::HomeMove(-1)),
            KeyCode::Enter => {
                return Some(if mode == HomeMode::Picker {
                    Action::PickerChoose
                } else {
                    Action::HomeOpen
                });
            }
            _ => {}
        }

        match mode {
            HomeMode::Picker => match key.code {
                KeyCode::Char('j') if !typing_path => Some(Action::HomeMove(1)),
                KeyCode::Char('k') if !typing_path => Some(Action::HomeMove(-1)),
                KeyCode::Esc => Some(Action::PickerCancel),
                KeyCode::Char(c) => Some(Action::HomeKey(SearchKey::Char(c))),
                KeyCode::Backspace => Some(Action::HomeKey(SearchKey::Backspace)),
                _ => None,
            },
            // Typing filters (or edits the content query). There are no
            // plain-letter commands here, which is why `Esc` exists.
            HomeMode::Filter | HomeMode::Search => match key.code {
                KeyCode::Char(c) => Some(Action::HomeKey(SearchKey::Char(c))),
                KeyCode::Backspace => Some(Action::HomeKey(SearchKey::Backspace)),
                KeyCode::Esc => Some(Action::HomeKey(SearchKey::Cancel)),
                _ => None,
            },
            HomeMode::Normal => match key.code {
                KeyCode::Char('j') => Some(Action::HomeMove(1)),
                KeyCode::Char('k') => Some(Action::HomeMove(-1)),
                KeyCode::Char('h') => Some(Action::HelpToggle),
                KeyCode::Char('q') => Some(Action::Quit),
                KeyCode::Char('T') => Some(Action::ThemeCycle),
                KeyCode::Char('H') => Some(Action::HintsToggle),
                KeyCode::Char('d') => Some(Action::PickerOpen),
                // `/` means "search content" everywhere else in carrel, so
                // it does here too (wave E); `i` remains the filename filter.
                KeyCode::Char('i') => Some(Action::HomeFilterMode),
                KeyCode::Char('/') => Some(Action::HomeSearchMode),
                KeyCode::Char('G') | KeyCode::End => Some(Action::HomeGo(Edge::Last)),
                KeyCode::Home => Some(Action::HomeGo(Edge::First)),
                KeyCode::Char('g') => {
                    self.pending_g = true;
                    None
                }
                _ => None,
            },
        }
    }

    /// Outline-picker bindings: the home screen's idiom exactly — printable
    /// keys type, arrows and Ctrl-N/P move, Enter commits, Esc backs out.
    #[must_use]
    pub fn map_outline(key: KeyEvent) -> Option<Action> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('n') if ctrl => Some(Action::OutlineMove(1)),
            KeyCode::Char('p') if ctrl => Some(Action::OutlineMove(-1)),
            KeyCode::Char('c') if ctrl => Some(Action::Quit),
            KeyCode::Down => Some(Action::OutlineMove(1)),
            KeyCode::Up => Some(Action::OutlineMove(-1)),
            KeyCode::Enter => Some(Action::OutlineJump),
            KeyCode::Char(c) => Some(Action::OutlineKey(SearchKey::Char(c))),
            KeyCode::Backspace => Some(Action::OutlineKey(SearchKey::Backspace)),
            KeyCode::Esc => Some(Action::OutlineKey(SearchKey::Cancel)),
            _ => None,
        }
    }

    fn map_search(key: KeyEvent) -> Option<Action> {
        let k = match key.code {
            KeyCode::Char(c) => SearchKey::Char(c),
            KeyCode::Backspace => SearchKey::Backspace,
            KeyCode::Enter => SearchKey::Accept,
            KeyCode::Esc => SearchKey::Cancel,
            _ => return None,
        };
        Some(Action::SearchKey(k))
    }
}

/// The help sheet, reader side. A `§` row opens a group. The drift test
/// below holds this table and the dispatcher together; edit them as one.
pub const READER_HELP: &[(&str, &str)] = &[
    ("§", "motions"),
    ("j k ↓ ↑", "line down / up"),
    ("Ctrl-E Ctrl-Y", "line down / up"),
    ("Ctrl-D Ctrl-U", "half page"),
    ("Space b PgDn PgUp", "page"),
    ("Ctrl-F Ctrl-B", "page"),
    ("{ }", "previous / next block"),
    ("gg G Home End", "start / end"),
    ("42G", "go to row 42"),
    ("§", "search"),
    ("/ ?", "search forward / backward"),
    ("n N", "next / previous match"),
    ("zz zt zb", "match to middle/top/bottom"),
    ("§", "links"),
    ("Tab Shift-Tab", "select next / previous link"),
    ("Enter", "follow the selected link"),
    ("Ctrl-O", "back"),
    ("Esc", "clear selection"),
    ("§", "view"),
    ("o", "outline: jump to a section"),
    ("t", "tables: cards / wrapped"),
    ("m", "diagrams & math: art / source"),
    ("T", "cycle themes"),
    ("h F1", "this help"),
    ("H", "hide / show the key hints"),
    ("q", "close file (or quit)"),
    ("Q Ctrl-C", "quit"),
    ("§", "mouse"),
    ("drag", "select — copies on release"),
    ("2× click", "select word, 3× the block"),
    ("wheel", "scroll; drag the bar to jump"),
];

/// The help sheet, home-screen side.
pub const HOME_HELP: &[(&str, &str)] = &[
    ("§", "moving"),
    ("j k ↓ ↑", "move"),
    ("Ctrl-N Ctrl-P", "move"),
    ("gg G Home End", "first / last"),
    ("Enter", "open"),
    ("§", "finding"),
    ("i", "filter names: type to narrow"),
    ("/", "search inside files"),
    ("Esc", "clear filter, then leave it"),
    ("§", "other"),
    ("d", "choose a directory"),
    ("T", "cycle themes"),
    ("h F1", "this help (normal mode)"),
    ("H", "hide / show the key hints"),
    ("q Ctrl-C", "quit"),
];

/// Lamplight footer hints — one table per state, selected by `footer::of`.
/// Same shape as the help tables. `("type", …)` entries are prose, exempt
/// from the honesty test's probe; every other key is fed back through the
/// real dispatcher, so a hinted key that would be inert fails the build.
pub const HINT_READING: &[(&str, &str)] = &[
    ("j/k", "scroll"),
    ("spc", "page"),
    ("/", "search"),
    ("o", "outline"),
    ("h", "more"),
];
pub const HINT_SEARCH_TYPING: &[(&str, &str)] = &[("enter", "jump"), ("esc", "cancel")];
pub const HINT_MATCHES: &[(&str, &str)] =
    &[("n/N", "next/prev"), ("zz", "center"), ("esc", "clear")];
pub const HINT_LINK: &[(&str, &str)] = &[("tab", "next"), ("enter", "follow"), ("esc", "clear")];
pub const HINT_OUTLINE: &[(&str, &str)] = &[("type", "narrow"), ("enter", "go"), ("esc", "back")];
pub const HINT_HELP: &[(&str, &str)] = &[("j/k", "scroll"), ("esc", "close")];
pub const HINT_HOME_BROWSE: &[(&str, &str)] = &[
    ("j/k", "move"),
    ("enter", "open"),
    ("d", "directory"),
    ("i", "filter"),
    ("/", "search"),
    ("h", "more"),
];
pub const HINT_HOME_FILTER: &[(&str, &str)] =
    &[("type", "narrow"), ("enter", "open"), ("esc", "back")];
pub const HINT_HOME_SEARCH: &[(&str, &str)] =
    &[("type", "query"), ("enter", "open first"), ("esc", "back")];
pub const HINT_HOME_PICKER: &[(&str, &str)] =
    &[("j/k", "move"), ("enter", "choose"), ("esc", "back")];
pub const HINT_HOME_PICKER_OTHER: &[(&str, &str)] =
    &[("type", "the path"), ("enter", "choose"), ("esc", "back")];

/// The scrollbar thumb's `(top, len)` in bar rows.
///
/// Carrel paints its own scrollbar from this same function, so the grabbed
/// thumb and the painted thumb cannot disagree — the earlier attempt mirrored
/// ratatui's widget arithmetic and still fought it. The position map is
/// deliberately end-anchored: at `max_scroll` the thumb touches the bottom of
/// the bar, because a thumb that stops short reads as "you are not at the
/// end" to someone who is.
#[must_use]
pub fn thumb_geometry(text_h: u16, total_rows: u32, scroll_row: u32) -> (u16, u16) {
    if text_h == 0 || u32::from(text_h) >= total_rows {
        return (0, text_h);
    }
    let h = f64::from(text_h);
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let len = ((h * h / f64::from(total_rows)).round() as u16).clamp(1, text_h);
    let track = text_h - len;
    let max_scroll = total_rows - u32::from(text_h);
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let top = ((f64::from(scroll_row) / f64::from(max_scroll)) * f64::from(track)).round() as u16;
    (top.min(track), len)
}

/// Where a grabbed thumb puts the document: the exact inverse of
/// [`thumb_geometry`]'s position map, clamped to the scrollable range.
///
/// `grab_offset` is where within the thumb the pointer took hold; preserving
/// it is what makes a drag track the hand instead of snapping the thumb's
/// top edge to the pointer — the difference between smooth and jarring.
#[must_use]
pub fn drag_target(
    pointer_y: u16,
    grab_offset: u16,
    text_h: u16,
    total_rows: u32,
    max_scroll: u32,
) -> u32 {
    let (_, len) = thumb_geometry(text_h, total_rows, 0);
    let track = text_h.saturating_sub(len);
    if track == 0 {
        return 0;
    }
    let new_top = pointer_y.saturating_sub(grab_offset).min(track);
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let row = ((f64::from(new_top) / f64::from(track)) * f64::from(max_scroll)).round() as u32;
    row.min(max_scroll)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }
    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }
    fn code(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    fn seq(keys: &[KeyEvent]) -> Vec<Action> {
        let mut m = Keys::new();
        keys.iter().filter_map(|e| m.map(*e, false)).collect()
    }

    #[test]
    fn j_scrolls_one_line() {
        assert_eq!(seq(&[k('j')]), vec![Action::Scroll(Span::Line, 1)]);
    }

    #[test]
    fn a_count_multiplies_the_motion() {
        assert_eq!(
            seq(&[k('1'), k('0'), k('j')]),
            vec![Action::Scroll(Span::Line, 10)]
        );
    }

    #[test]
    fn the_count_resets_after_it_is_consumed() {
        assert_eq!(
            seq(&[k('3'), k('j'), k('j')]),
            vec![Action::Scroll(Span::Line, 3), Action::Scroll(Span::Line, 1)],
        );
    }

    #[test]
    fn a_leading_zero_is_not_a_count() {
        assert_eq!(seq(&[k('0'), k('j')]), vec![Action::Scroll(Span::Line, 1)]);
    }

    #[test]
    fn gg_goes_to_the_start_and_takes_two_keys() {
        assert_eq!(seq(&[k('g')]), vec![]);
        assert_eq!(seq(&[k('g'), k('g')]), vec![Action::GoToStart]);
    }

    #[test]
    fn capital_g_goes_to_the_end_but_to_a_row_with_a_count() {
        assert_eq!(seq(&[k('G')]), vec![Action::GoToEnd]);
        // Users count from 1, the row index from 0.
        assert_eq!(seq(&[k('4'), k('2'), k('G')]), vec![Action::GoToRow(41)]);
    }

    #[test]
    fn ctrl_d_and_ctrl_u_are_half_pages() {
        assert_eq!(
            seq(&[ctrl('d'), ctrl('u')]),
            vec![
                Action::Scroll(Span::HalfPage, 1),
                Action::Scroll(Span::HalfPage, -1)
            ],
        );
    }

    #[test]
    fn ctrl_b_is_a_page_not_the_plain_b_binding() {
        assert_eq!(seq(&[ctrl('b')]), vec![Action::Scroll(Span::Page, -1)]);
    }

    #[test]
    fn z_prefixed_keys_position_the_current_match() {
        assert_eq!(
            seq(&[k('z'), k('z')]),
            vec![Action::Recenter(Where::Middle)]
        );
        assert_eq!(seq(&[k('z'), k('t')]), vec![Action::Recenter(Where::Top)]);
        assert_eq!(
            seq(&[k('z'), k('b')]),
            vec![Action::Recenter(Where::Bottom)]
        );
    }

    #[test]
    fn esc_clears_a_pending_count() {
        let mut m = Keys::new();
        m.map(k('9'), false);
        m.map(code(KeyCode::Esc), false);
        assert_eq!(m.map(k('j'), false), Some(Action::Scroll(Span::Line, 1)));
    }

    #[test]
    fn search_mode_sends_characters_to_the_prompt() {
        let mut m = Keys::new();
        assert_eq!(
            m.map(k('j'), true),
            Some(Action::SearchKey(SearchKey::Char('j')))
        );
        assert_eq!(
            m.map(code(KeyCode::Enter), true),
            Some(Action::SearchKey(SearchKey::Accept)),
        );
    }

    #[test]
    fn home_filter_mode_sends_printable_characters_to_the_filter() {
        let mut m = Keys::new();
        assert_eq!(
            m.map_home(k('j'), HomeMode::Filter, false),
            Some(Action::HomeKey(SearchKey::Char('j'))),
        );
    }

    #[test]
    fn home_normal_mode_uses_vim_keys() {
        let mut m = Keys::new();
        assert_eq!(
            m.map_home(k('j'), HomeMode::Normal, false),
            Some(Action::HomeMove(1))
        );
        assert_eq!(
            m.map_home(k('d'), HomeMode::Normal, false),
            Some(Action::PickerOpen)
        );
        assert_eq!(
            m.map_home(k('q'), HomeMode::Normal, false),
            Some(Action::Quit)
        );
        assert_eq!(
            m.map_home(k('g'), HomeMode::Normal, false),
            None,
            "gg is two keys"
        );
        assert_eq!(
            m.map_home(k('g'), HomeMode::Normal, false),
            Some(Action::HomeGo(Edge::First)),
        );
    }

    #[test]
    fn ctrl_n_and_p_move_in_both_home_modes() {
        let mut m = Keys::new();
        assert_eq!(
            m.map_home(ctrl('n'), HomeMode::Filter, false),
            Some(Action::HomeMove(1))
        );
        assert_eq!(
            m.map_home(ctrl('p'), HomeMode::Normal, false),
            Some(Action::HomeMove(-1))
        );
    }

    #[test]
    fn enter_opens_a_file_but_chooses_a_root_in_the_picker() {
        let mut m = Keys::new();
        assert_eq!(
            m.map_home(code(KeyCode::Enter), HomeMode::Filter, false),
            Some(Action::HomeOpen)
        );
        assert_eq!(
            m.map_home(code(KeyCode::Enter), HomeMode::Picker, false),
            Some(Action::PickerChoose),
        );
    }

    #[test]
    fn j_and_k_navigate_the_picker_but_type_into_the_other_path() {
        let mut m = Keys::new();
        // On a listed root, vim keys move…
        assert_eq!(
            m.map_home(k('j'), HomeMode::Picker, false),
            Some(Action::HomeMove(1)),
        );
        // …but while `Other…` is highlighted they belong to the path —
        // otherwise a path like /home/jay is untypeable, its j stolen by movement.
        assert_eq!(
            m.map_home(k('j'), HomeMode::Picker, true),
            Some(Action::HomeKey(SearchKey::Char('j'))),
        );
        assert_eq!(
            m.map_home(k('k'), HomeMode::Picker, true),
            Some(Action::HomeKey(SearchKey::Char('k'))),
        );
        // Arrows still move even while typing.
        assert_eq!(
            m.map_home(code(KeyCode::Up), HomeMode::Picker, true),
            Some(Action::HomeMove(-1)),
        );
    }

    #[test]
    fn capital_h_toggles_hints_but_still_types_where_typing_goes_elsewhere() {
        let mut m = Keys::new();
        assert_eq!(m.map(k('H'), false), Some(Action::HintsToggle));
        assert_eq!(
            m.map(k('H'), true),
            Some(Action::SearchKey(SearchKey::Char('H'))),
            "while typing a search, H is a letter"
        );
        assert_eq!(
            m.map_home(k('H'), HomeMode::Normal, false),
            Some(Action::HintsToggle)
        );
        assert_eq!(
            m.map_home(k('H'), HomeMode::Filter, false),
            Some(Action::HomeKey(SearchKey::Char('H'))),
            "while filtering, H is a letter"
        );
    }

    /// A hint that would not act is a lie. Every footer hint key, pressed in
    /// the state that advertises it, must produce an action from the REAL
    /// dispatcher. `("type", …)` entries are prose and skipped.
    #[test]
    fn every_footer_hint_key_acts_in_its_own_state() {
        // The keypress sequence exercising a hint; the LAST press must yield
        // Some(action). `zz` needs two presses — the first arms the prefix.
        fn probe(key: &str) -> Option<Vec<KeyEvent>> {
            Some(match key {
                "type" => return None,
                "j/k" => vec![k('j')],
                "spc" => vec![k(' ')],
                "/" => vec![k('/')],
                "o" => vec![k('o')],
                "h" => vec![k('h')],
                "d" => vec![k('d')],
                "i" => vec![k('i')],
                "n/N" => vec![k('n')],
                "zz" => vec![k('z'), k('z')],
                "tab" => vec![code(KeyCode::Tab)],
                "enter" => vec![code(KeyCode::Enter)],
                "esc" => vec![code(KeyCode::Esc)],
                other => panic!("unmapped footer hint key {other:?} — extend probe()"),
            })
        }
        type Dispatch = Box<dyn Fn(&[KeyEvent]) -> Option<Action>>;
        type Case = (
            &'static str,
            &'static [(&'static str, &'static str)],
            Dispatch,
        );
        // Feed the whole sequence IN ORDER (a prefix key like the first `z`
        // must actually arm) and judge only the final press.
        let reader = |searching: bool| -> Dispatch {
            Box::new(move |evs| {
                let mut m = Keys::new();
                evs.iter().fold(None, |_, e| m.map(*e, searching))
            })
        };
        let home = |mode: HomeMode, typing: bool| -> Dispatch {
            Box::new(move |evs| {
                let mut m = Keys::new();
                evs.iter().fold(None, |_, e| m.map_home(*e, mode, typing))
            })
        };
        let outline: Dispatch = Box::new(|evs| evs.iter().fold(None, |_, e| Keys::map_outline(*e)));

        let cases: Vec<Case> = vec![
            ("reading", HINT_READING, reader(false)),
            ("search-typing", HINT_SEARCH_TYPING, reader(true)),
            ("matches", HINT_MATCHES, reader(false)),
            ("link", HINT_LINK, reader(false)),
            ("outline", HINT_OUTLINE, outline),
            ("help", HINT_HELP, reader(false)),
            (
                "home-browse",
                HINT_HOME_BROWSE,
                home(HomeMode::Normal, false),
            ),
            (
                "home-filter",
                HINT_HOME_FILTER,
                home(HomeMode::Filter, false),
            ),
            (
                "home-search",
                HINT_HOME_SEARCH,
                home(HomeMode::Search, false),
            ),
            (
                "home-picker",
                HINT_HOME_PICKER,
                home(HomeMode::Picker, false),
            ),
            (
                "home-picker-other",
                HINT_HOME_PICKER_OTHER,
                home(HomeMode::Picker, true),
            ),
        ];
        for (state, table, dispatch) in cases {
            for (key, label) in table {
                let Some(evs) = probe(key) else { continue };
                assert!(
                    dispatch(&evs).is_some(),
                    "footer lies: {state} hints `{key} {label}` but the key is inert"
                );
            }
        }
    }

    #[test]
    fn the_thumb_is_end_anchored_top_and_bottom() {
        // 20 rows over 100: viewport share = 4 rows of thumb, 16 of track.
        assert_eq!(thumb_geometry(20, 100, 0), (0, 4), "top at the top");
        assert_eq!(thumb_geometry(20, 100, 80), (16, 4), "BOTTOM at the bottom");
        assert_eq!(thumb_geometry(20, 100, 40), (8, 4), "middle in the middle");
        assert_eq!(thumb_geometry(20, 10, 0), (0, 20), "fits -> full-bar thumb");
    }

    #[test]
    fn dragging_is_the_exact_inverse_and_clamps() {
        // Round-trip at every track position: paint(drag(y)) == y.
        for y in 0..=16u16 {
            let row = drag_target(y, 0, 20, 100, 80);
            let (top, _) = thumb_geometry(20, 100, row);
            assert_eq!(top, y, "row {row} for pointer {y}");
        }
        assert_eq!(drag_target(200, 0, 20, 100, 80), 80, "past the end clamps");
        assert_eq!(drag_target(0, 5, 20, 100, 80), 0, "above the grab clamps");
        assert_eq!(drag_target(5, 0, 20, 10, 0), 0, "full-bar thumb: no track");
    }

    #[test]
    fn n_steps_matches_and_respects_a_count() {
        assert_eq!(seq(&[k('3'), k('n')]), vec![Action::MatchStep(3)]);
        assert_eq!(seq(&[k('N')]), vec![Action::MatchStep(-1)]);
    }

    #[test]
    fn every_help_row_fits_the_panel_without_truncating() {
        // The painter gives rows `4 + 18 + 1` columns of prefix inside a
        // 52-wide panel: keys get 18 cells, descriptions 29. A row that
        // doesn't fit silently truncates on screen — this makes it red
        // instead (it happened: "scroll; drag the bar to jump" once lost
        // its tail).
        for (key, desc) in READER_HELP.iter().chain(HOME_HELP) {
            if *key == "§" {
                continue;
            }
            assert!(
                carrel_core::display_width(key) <= 18,
                "key column overflows: {key:?}"
            );
            assert!(
                carrel_core::display_width(desc) <= 29,
                "description overflows the panel: {desc:?}"
            );
        }
    }

    #[test]
    fn o_toggles_the_outline_and_outline_mode_types_and_moves() {
        let mut m = Keys::new();
        assert_eq!(m.map(k('o'), false), Some(Action::OutlineToggle));
        assert_eq!(m.map(ctrl('o'), false), Some(Action::Back), "Ctrl-O intact");
        assert_eq!(
            Keys::map_outline(k('x')),
            Some(Action::OutlineKey(SearchKey::Char('x')))
        );
        assert_eq!(
            Keys::map_outline(code(KeyCode::Enter)),
            Some(Action::OutlineJump)
        );
        assert_eq!(
            Keys::map_outline(code(KeyCode::Esc)),
            Some(Action::OutlineKey(SearchKey::Cancel))
        );
        assert_eq!(
            Keys::map_outline(code(KeyCode::Down)),
            Some(Action::OutlineMove(1))
        );
        assert_eq!(Keys::map_outline(ctrl('p')), Some(Action::OutlineMove(-1)));
    }

    #[test]
    fn h_and_f1_open_help_in_the_reader_and_home_normal_mode() {
        let mut m = Keys::new();
        assert_eq!(m.map(k('h'), false), Some(Action::HelpToggle));
        assert_eq!(m.map(code(KeyCode::F(1)), false), Some(Action::HelpToggle));
        assert_eq!(
            m.map_home(k('h'), HomeMode::Normal, false),
            Some(Action::HelpToggle)
        );
        assert_eq!(
            m.map_home(code(KeyCode::F(1)), HomeMode::Filter, false),
            Some(Action::HelpToggle),
            "F1 works even while filtering"
        );
        assert_eq!(
            m.map_home(k('h'), HomeMode::Filter, false),
            Some(Action::HomeKey(SearchKey::Char('h'))),
            "h types into the filter"
        );
    }

    /// Every Action variant must either appear in a help table or be listed
    /// as deliberately undocumented. The match is EXHAUSTIVE: a new variant
    /// fails to compile until someone decides which it is — help that lies
    /// is worse than no help.
    #[test]
    fn every_action_is_documented_or_deliberately_not() {
        use crate::action::Action as A;
        // Same key documenting two variants (reader + home) is the point.
        #[allow(clippy::match_same_arms)]
        fn doc_key(a: A) -> Option<&'static str> {
            Some(match a {
                A::Scroll(..) => "j k",
                A::GoToStart => "gg",
                A::GoToEnd => "G",
                A::GoToRow(_) => "42G",
                A::BlockStep(_) => "{ }",
                A::Recenter(_) => "zz",
                A::SearchOpen(_) => "/ ?",
                A::MatchStep(_) => "n N",
                A::LinkStep(_) => "Tab",
                A::LinkFollow => "Enter",
                A::Back => "Ctrl-O",
                A::Dismiss => "Esc",
                A::CloseFile => "q",
                A::ThemeCycle => "T",
                A::TableToggle => "t",
                A::RenderedToggle => "m",
                A::HelpToggle => "h F1",
                A::HintsToggle => "H",
                A::OutlineToggle => "o",
                A::Quit => "Q",
                A::HomeMove(_) => "j k",
                A::HomeGo(_) => "gg",
                A::HomeOpen => "Enter",
                A::HomeFilterMode => "i",
                A::HomeSearchMode => "/",
                // Deliberately undocumented: internal or pointer-driven.
                // (The mouse gestures ARE documented — as prose rows in the
                // help table's mouse group, not as key bindings.)
                A::SearchKey(_)
                | A::HomeKey(_)
                | A::OutlineKey(_)
                | A::OutlineMove(_)
                | A::OutlineJump
                | A::ScrollTo(_)
                | A::SelectAnchor(_)
                | A::SelectDrag(_)
                | A::SelectRelease
                | A::SelectWord(_)
                | A::SelectBlock(_)
                | A::HomeNormalMode
                | A::PickerOpen
                | A::PickerChoose
                | A::PickerCancel => return None,
            })
        }
        let all_rows: String = READER_HELP
            .iter()
            .chain(HOME_HELP)
            .map(|(key, _)| *key)
            .collect::<Vec<_>>()
            .join(" | ");
        // One representative per documented variant must appear in a table.
        for a in [
            A::Scroll(Span::Line, 1),
            A::GoToStart,
            A::GoToEnd,
            A::GoToRow(0),
            A::BlockStep(1),
            A::Recenter(Where::Middle),
            A::SearchOpen(Direction::Forward),
            A::MatchStep(1),
            A::LinkStep(1),
            A::LinkFollow,
            A::Back,
            A::Dismiss,
            A::CloseFile,
            A::ThemeCycle,
            A::TableToggle,
            A::RenderedToggle,
            A::HelpToggle,
            A::OutlineToggle,
            A::Quit,
            A::HomeMove(1),
            A::HomeGo(Edge::First),
            A::HomeOpen,
            A::HomeFilterMode,
            A::HomeSearchMode,
        ] {
            if let Some(key) = doc_key(a) {
                let first = key.split_whitespace().next().unwrap();
                assert!(
                    all_rows.contains(first),
                    "action {a:?} documents key {first:?} but no help row mentions it"
                );
            }
        }
    }
}
