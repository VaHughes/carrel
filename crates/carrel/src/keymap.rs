//! What the keys are called, and what each does — as data.
//!
//! The terminal's key DISPATCHER lives in [`crate::keys`], and it is the one
//! module of the state layer's neighbours that names crossterm. Everything
//! else the state layer needs to know about keys is printed text: the help
//! sheet, the footer's hints, the accelerator a menu row shows, which keys
//! an open menu keeps for itself. Those live here, as tables of strings and
//! [`Action`]s, so `app`, `footer` and `menu` can read them without
//! compiling against a terminal library. Until 2026-10-31 they lived in
//! `keys.rs` beside the dispatcher, so the "pure" state layer reached
//! crossterm through it while rule 6 of `check-discipline.sh`, which only
//! looked for a direct `use ratatui`, reported it clean (research.md Q29).
//!
//! The tables cannot drift from the dispatcher, because `keys.rs`'s tests
//! press every key named here through it: the help sheet's third column,
//! every footer hint's click, every accelerator, and [`MENU_KEYS`].
//!
//! NO RATATUI, NO CROSSTERM — `scripts/check-discipline.sh` rule 6.

use crate::action::{Action, Direction, Edge, SearchKey, Span, Where};

/// One row of a help sheet: the keys as printed, what they do, and what a
/// CLICK on the row does — the action its first key that does anything
/// here produces. `None` for a `§` group header and for a row that is a
/// sentence rather than a key (`click a tag`). The third column is what
/// lets the state layer run a clicked row without a key dispatcher; the
/// test `every_help_row_runs_what_its_key_does` presses each row's keys
/// through the real one to keep it honest.
pub type HelpRow = (&'static str, &'static str, Option<Action>);

/// The key a menu row advertises for an action, or `None` when the action
/// has no keyboard binding at all.
///
/// **The match is exhaustive**, so a new `Action` fails to compile until
/// someone decides which key documents it — the same discipline the help
/// tables are held to, and for the same reason: a shortcut printed beside a
/// menu item teaches a key, and teaching the wrong one is worse than
/// teaching none. Where several keys reach an action the FORWARD one is
/// named, which is the rule `Hint::click` already follows.
///
/// Multi-key rows read as they do in the help sheet (`"j k"`, `"h F1"`);
/// [`first`] takes the one a menu should print.
#[must_use]
#[allow(clippy::too_many_lines)] // one arm per Action; that IS the guarantee
#[allow(clippy::match_same_arms)] // the same key documenting reader + home is the point
pub const fn accel(a: Action) -> Option<&'static str> {
    use Action as A;
    Some(match a {
        A::Scroll(..) => "j k",
        A::HomePage(_) => "PgDn PgUp",
        A::SettingsToggle => ",",
        // A button, not a key: nothing to press, so nothing to print.
        A::WelcomeOpen | A::ImageOpen(_) | A::ImageStep(_) => return None,
        A::SettingsMove(_) => "j k",
        A::SettingsAdjust(_) | A::SettingsPickAt(_) => "\u{21b5}",
        A::MeasureStep(_) => "+ -",
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
        A::HighlightAdd | A::HighlightAt(_) => "v",
        A::NoteEdit | A::NoteAt(_) => "a",
        A::NotesToggle => "V",
        A::NoteNext => "M",
        A::NotesExport | A::NotesCopy => return None,
        A::NoteMove(_) | A::NoteSelect(_) | A::NoteJump | A::NoteDelete | A::NoteInput(_) => {
            return None;
        }
        A::TableToggle => "t",
        A::TableScroll { delta, .. } => {
            if delta < 0 {
                "Shift-←"
            } else {
                "Shift-→"
            }
        }
        A::RenderedToggle => "r",
        A::MarkToggle => "m",
        A::MarkNext => "'",
        A::MarkListToggle => "\"",
        A::MarkListMove(_) | A::MarkListJump => "\"",
        A::FootnoteJump => "%",
        A::FollowToggle => "F",
        A::AutoToggle => "A",
        A::AutoTick => return None,
        A::CodeStep(_) => "] [",
        A::TaskStep(_) => "X",
        A::ChangeStep(_) => "c",
        A::YankBlock => "y",
        A::HelpToggle => "h F1",
        A::HintsToggle => "H",
        A::BreadcrumbToggle => "B",
        A::InfoToggle => "I",
        A::FocusToggle => "S",
        A::FoldToggle => "za",
        A::FoldAll => "zM",
        A::UnfoldAll => "zR",
        A::OutlineToggle => "o",
        A::Quit => "Q",
        A::HomeMove(_) => "j k",
        A::HomeGo(_) => "gg",
        A::HomeOpen => "Enter",
        A::HomeFilterMode => "i",
        A::HomeSearchMode => "/",
        A::HomeTags => "#",
        A::HomeSort => "s",
        A::HomeResume(_) => "1 2 3",
        A::HomeUp => "Backspace",
        A::PickerOpen => "d",
        A::OutlineJumpTo(_) => "click",
        A::BacklinksToggle => "L",
        A::BacklinksMove(_) | A::BacklinksOpen => "L",
        A::ForwardToggle => "l",
        A::ForwardMove(_) | A::ForwardOpen => "l",
        // No key of their own: internal, pointer-driven, or reachable only
        // through the menu that names them in words.
        A::SearchKey(_)
        | A::LinkOpen(_)
        | A::LinkCopy
        | A::FoldAt(_)
        | A::FootnoteJumpAt(_)
        | A::BacklinksOpenAt(_)
        | A::ForwardOpenAt(_)
        | A::MarkListJumpAt(_)
        | A::OutlineJumpAt(_)
        | A::Absorb
        | A::PaneClose
        | A::Hover(_)
        | A::HomeCrumb(_)
        | A::GoHome
        | A::MenuOpen { .. }
        | A::MenuMove(_)
        | A::MenuHover(_)
        | A::MenuChoose
        | A::MenuPick(_)
        | A::MenuClose
        | A::HomeSelect(_)
        | A::PickerSelect(_)
        | A::CrumbJump(_)
        | A::HomeKey(_)
        | A::HelpKey(_)
        | A::OutlineKey(_)
        | A::OutlineMove(_)
        | A::OutlineJump
        | A::ScrollTo(_)
        | A::SelectAnchor(_)
        | A::SelectDrag(_)
        | A::SelectRelease
        | A::SelectWord(_)
        | A::SelectBlock(_)
        | A::HomeOpenTags
        | A::TagOpen(_)
        | A::Forward
        | A::HelpRun(_)
        | A::SiblingOpen
        | A::BackTo(_)
        | A::FootnotePeek { .. }
        | A::PeekGo
        | A::PeekScroll(_)
        | A::TaskOpen
        | A::YankBlockAt(_)
        | A::CopyRef(_)
        | A::CopySection(_)
        | A::CopyQuote
        | A::HomeNormalMode
        | A::PickerChoose
        | A::PickerDescend
        | A::PickerUp
        | A::HomeOpenResults
        | A::PickerCancel => return None,
    })
}

/// The printable keys an open menu takes for itself, rather than leaving
/// them to mean what they mean outside it: `q`, which closes the menu, as
/// it closes every carrel pane. `Keys::map_menu` is the authority; the test
/// `every_printable_key_the_menu_takes_is_listed` holds this list to it, so
/// the menu builder can ask without a key-event type in sight.
pub const MENU_KEYS: &[char] = &['q'];

/// Does an open menu take this printed key for itself? A single character
/// listed in [`MENU_KEYS`]. A menu row must not print such a key
/// (`menu::printable`).
#[must_use]
pub fn menu_takes(key: &str) -> bool {
    let mut chars = key.chars();
    matches!((chars.next(), chars.next()), (Some(c), None) if MENU_KEYS.contains(&c))
}

/// The one key a menu prints from an [`accel`] row: the first.
#[must_use]
pub fn first(keys: &str) -> &str {
    keys.split_whitespace().next().unwrap_or(keys)
}

/// The help sheet, reader side. A `§` row opens a group. The drift test
/// below holds this table and the dispatcher together; edit them as one.
pub const READER_HELP: &[HelpRow] = &[
    ("§", "moving", None),
    (
        "↓ ↑ j k",
        "line down / up",
        Some(Action::Scroll(Span::Line, 1)),
    ),
    (
        "Ctrl-E Ctrl-Y",
        "line down / up",
        Some(Action::Scroll(Span::Line, 1)),
    ),
    (
        "Ctrl-D Ctrl-U",
        "half page",
        Some(Action::Scroll(Span::HalfPage, 1)),
    ),
    (
        "Space PgDn PgUp b",
        "page down / up",
        Some(Action::Scroll(Span::Page, 1)),
    ),
    ("Ctrl-F Ctrl-B", "page", Some(Action::Scroll(Span::Page, 1))),
    (
        "{ }",
        "previous / next paragraph",
        Some(Action::BlockStep(-1)),
    ),
    ("gg G Home End", "start / end", Some(Action::GoToStart)),
    ("42G", "go to line 42", None),
    ("§", "search", None),
    (
        "/ ?",
        "find text; ↑/↓ past searches",
        Some(Action::SearchOpen(Direction::Forward)),
    ),
    (
        "n N F3",
        "next / previous match",
        Some(Action::MatchStep(1)),
    ),
    (
        "zz zt zb",
        "match to middle/top/bottom",
        Some(Action::Recenter(Where::Middle)),
    ),
    ("§", "links", None),
    (
        "Tab Shift-Tab",
        "select next / previous link",
        Some(Action::LinkStep(1)),
    ),
    (
        "Enter",
        "follow the selected link",
        Some(Action::LinkFollow),
    ),
    (
        "Ctrl-O",
        "back to the previous document",
        Some(Action::Back),
    ),
    ("Esc", "clear selection and search", Some(Action::Dismiss)),
    ("§", "view", None),
    (
        "o",
        "outline: jump to a section",
        Some(Action::OutlineToggle),
    ),
    ("za", "collapse this section", Some(Action::FoldToggle)),
    ("zM zR", "collapse all / expand all", Some(Action::FoldAll)),
    (
        "v",
        "highlight selection/paragraph",
        Some(Action::HighlightAdd),
    ),
    ("a", "add or edit a note", Some(Action::NoteEdit)),
    ("V", "notes and highlights", Some(Action::NotesToggle)),
    ("M", "next note or highlight", Some(Action::NoteNext)),
    (
        "t",
        "wide tables: cards or columns",
        Some(Action::TableToggle),
    ),
    (
        "Shift-← Shift-→",
        "scroll a wide table sideways",
        Some(Action::TableScroll {
            block: None,
            delta: -1,
        }),
    ),
    (
        "r",
        "diagrams, math: drawn or text",
        Some(Action::RenderedToggle),
    ),
    ("T", "next theme", Some(Action::ThemeCycle)),
    (",", "settings", Some(Action::SettingsToggle)),
    ("h F1", "this help", Some(Action::HelpToggle)),
    ("H", "hide / show the hint row", Some(Action::HintsToggle)),
    (
        "B",
        "hide / show the heading bar",
        Some(Action::BreadcrumbToggle),
    ),
    ("I", "document info", Some(Action::InfoToggle)),
    ("S", "dim all but this paragraph", Some(Action::FocusToggle)),
    ("q", "close file (or quit)", Some(Action::CloseFile)),
    ("Q Ctrl-C", "quit", Some(Action::Quit)),
    (
        "m '",
        "bookmark here / go to next",
        Some(Action::MarkToggle),
    ),
    (
        "\"",
        "list bookmarks — enter jumps",
        Some(Action::MarkListToggle),
    ),
    ("%", "footnote ↔ its text", Some(Action::FootnoteJump)),
    ("#", "tags, and their documents", Some(Action::HomeTags)),
    (
        "c",
        "next change after a reload",
        Some(Action::ChangeStep(1)),
    ),
    ("L", "what links here", Some(Action::BacklinksToggle)),
    ("l", "what this points at", Some(Action::ForwardToggle)),
    (
        "] [",
        "next / previous code block",
        Some(Action::CodeStep(1)),
    ),
    ("X", "jump to the next task", Some(Action::TaskStep(1))),
    ("y", "copy the code block", Some(Action::YankBlock)),
    ("click copy", "under a code block: copy it", None),
    ("F", "keep up as it arrives", Some(Action::FollowToggle)),
    ("A", "scroll slowly on its own", Some(Action::AutoToggle)),
    ("§", "mouse", None),
    ("drag", "select — copies on release", None),
    ("2× click", "select word, 3× the block", None),
    ("wheel", "scroll; drag the bar to jump", None),
    ("right-click", "a menu for what is under it", None),
    ("menu button", "the ≡ on the status row", None),
    ("hover", "the thing under it lights", None),
    ("click a section", "of the heading bar: go there", None),
    ("click a name", "on the status row: go back", None),
    ("click a [^mark]", "its footnote, in place", None),
    ("in a footnote box", "scroll keys scroll a long one", None),
    ("click a tag", "on the card: the tags page", None),
    ("click a row", "of this sheet: do it", None),
    ("click beside", "an open pane: close it", None),
    // Carrel copies through OSC 52, which it writes blind: there is no
    // reply to read, so it CANNOT know whether the terminal took it. It
    // said "copied" either way, and a reader whose terminal drops the
    // sequence — default tmux, some GNOME Terminal builds — was left with
    // a success message and an empty clipboard and no way to tell why.
    ("§", "copying", None),
    ("how it works", "carrel asks the terminal", None),
    ("if nothing pastes", "your terminal refused it", None),
];

/// The help sheet, home-screen side.
pub const HOME_HELP: &[HelpRow] = &[
    ("§", "moving", None),
    ("j k ↓ ↑", "move", Some(Action::HomeMove(1))),
    (
        "Ctrl-N/P Ctrl-J/K",
        "move while typing",
        Some(Action::HomeMove(1)),
    ),
    (
        "gg G Home End",
        "first / last",
        Some(Action::HomeGo(Edge::First)),
    ),
    ("Enter", "open", Some(Action::HomeOpen)),
    // Prose, not a binding — the mouse rows are documented the same way the
    // reader's drag/double-click gestures are, and the honesty test skips
    // them because there is no key to press.
    ("click", "select a file", None),
    ("double-click", "open it", None),
    ("1 2 3", "continue reading", Some(Action::HomeResume(0))),
    ("§", "finding", None),
    (
        "i",
        "filter by name: just type",
        Some(Action::HomeFilterMode),
    ),
    ("/", "search inside files", Some(Action::HomeSearchMode)),
    ("Tab", "open the results as a page", None),
    ("#", "tags, and their documents", Some(Action::HomeTags)),
    ("s", "sort: newest, name, read", Some(Action::HomeSort)),
    ("Esc", "clear the filter, then close", None),
    ("§", "other", None),
    ("d", "choose another folder", Some(Action::PickerOpen)),
    ("Tab →", "go into the folder", None),
    ("← Backspace", "up one folder", Some(Action::HomeUp)),
    ("click a segment", "of the path row: go there", None),
    ("right-click", "a menu, anywhere", None),
    ("T", "next theme", Some(Action::ThemeCycle)),
    (",", "settings", Some(Action::SettingsToggle)),
    (
        "h F1",
        "this help (F1 while typing)",
        Some(Action::HelpToggle),
    ),
    ("H", "hide / show the hint row", Some(Action::HintsToggle)),
    ("q Q Ctrl-C", "quit", Some(Action::Quit)),
];

/// Indices into a help table surviving `filter`: fuzzy over "key
/// description", best first — the home filter's rule, so the two pickers
/// feel like one. `§` group headers never match; with a filter up they drop
/// out and the survivors read flat. An empty filter is every row, groups
/// and all.
#[must_use]
pub fn help_matching(table: &[HelpRow], filter: &str) -> Vec<usize> {
    let needle = filter.trim().to_lowercase();
    if needle.is_empty() {
        return (0..table.len()).collect();
    }
    let mut scored: Vec<(i32, usize)> = table
        .iter()
        .enumerate()
        .filter(|(_, (key, _, _))| *key != "§")
        .filter_map(|(i, (key, desc, _))| {
            crate::fuzzy::score(&format!("{key} {desc}"), &needle).map(|s| (s, i))
        })
        .collect();
    scored.sort_by_key(|&(rank, _)| std::cmp::Reverse(rank));
    scored.into_iter().map(|(_, i)| i).collect()
}

/// Lamplight footer hints — one table per state, selected by `footer::of`.
/// Same shape as the help tables. `("type", …)` entries are prose, exempt
/// from the honesty test's probe; every other key is fed back through the
/// real dispatcher, so a hinted key that would be inert fails the build.
/// One footer hint: the key, what it does, and what a CLICK on it does.
///
/// The third column is what makes the footer a row of buttons rather than a
/// row of advice. **A hint's click does what its FIRST key does** — `j/k`
/// scrolls down, `n/N` goes to the next match — because the first key listed
/// is the forward one, and a button has to pick. `click: None` is for a row
/// that names no key at all: `("type", "narrow")` is a sentence, not a button.
///
/// `every_footer_hint_key_acts_in_its_own_state` presses the key through the
/// real dispatcher and asserts it produces exactly this action, so the two
/// columns cannot drift apart.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Hint {
    pub key: &'static str,
    pub label: &'static str,
    pub click: Option<Action>,
}

const fn hint(key: &'static str, label: &'static str, click: Action) -> Hint {
    Hint {
        key,
        label,
        click: Some(click),
    }
}

/// A hint that is prose: it names no key, so there is nothing to click.
const fn says(key: &'static str, label: &'static str) -> Hint {
    Hint {
        key,
        label,
        click: None,
    }
}

/// The `↑` half of a `↑/↓` hint, whose `click` is the `↓` half.
///
/// A pane's border paints the two arrows as two buttons; this is what the
/// up arrow does, and the honesty test presses `Up` through the real
/// dispatcher to prove it. `None` for an action with no direction.
#[must_use]
pub const fn upward(a: Action) -> Option<Action> {
    Some(match a {
        Action::Scroll(span, n) => Action::Scroll(span, -n),
        Action::NoteMove(n) => Action::NoteMove(-n),
        Action::BacklinksMove(n) => Action::BacklinksMove(-n),
        Action::ForwardMove(n) => Action::ForwardMove(-n),
        Action::MarkListMove(n) => Action::MarkListMove(-n),
        Action::HomeMove(n) => Action::HomeMove(-n),
        Action::SettingsMove(n) => Action::SettingsMove(-n),
        Action::PeekScroll(n) => Action::PeekScroll(-n),
        _ => return None,
    })
}

pub const HINT_READING: &[Hint] = &[
    hint("↑/↓", "scroll", Action::Scroll(Span::Line, 1)),
    hint("spc", "page", Action::Scroll(Span::Page, 1)),
    hint("/", "search", Action::SearchOpen(Direction::Forward)),
    hint("o", "outline", Action::OutlineToggle),
    hint("h", "help", Action::HelpToggle),
];
pub const HINT_SEARCH_TYPING: &[Hint] = &[
    hint("enter", "jump", Action::SearchKey(SearchKey::Accept)),
    hint("esc", "cancel", Action::SearchKey(SearchKey::Cancel)),
    hint(
        "↑",
        "older search",
        Action::SearchKey(SearchKey::Recall(Direction::Backward)),
    ),
    hint(
        "↓",
        "newer search",
        Action::SearchKey(SearchKey::Recall(Direction::Forward)),
    ),
];
pub const HINT_STREAMING: &[Hint] = &[
    hint("↑/↓", "scroll", Action::Scroll(Span::Line, 1)),
    hint("F", "keep up", Action::FollowToggle),
    hint("y", "copy code", Action::YankBlock),
    hint("h", "help", Action::HelpToggle),
];
pub const HINT_FOLLOWING: &[Hint] = &[
    hint("F", "stop", Action::FollowToggle),
    hint("↑", "step back", Action::Scroll(Span::Line, -1)),
    hint("y", "copy code", Action::YankBlock),
    hint("h", "help", Action::HelpToggle),
];
pub const HINT_MATCHES: &[Hint] = &[
    hint("n/N", "next / previous", Action::MatchStep(1)),
    hint("zz", "center", Action::Recenter(Where::Middle)),
    hint("esc", "clear", Action::Dismiss),
];
pub const HINT_LINK: &[Hint] = &[
    hint("tab", "next", Action::LinkStep(1)),
    hint("enter", "follow", Action::LinkFollow),
    hint("esc", "clear", Action::Dismiss),
];
pub const HINT_OUTLINE: &[Hint] = &[
    says("type", "to filter"),
    hint("enter", "go", Action::OutlineJump),
    hint("esc", "close", Action::OutlineKey(SearchKey::Cancel)),
];
// The five list panes. Each used to fall through to the READING hints, whose
// `/ search`, `o outline` and `h help` chips are openers the open pane now
// refuses — buttons that did nothing (and, before the one-pane rule, opened a
// second pane under or over the first).
pub const HINT_SETTINGS: &[Hint] = &[
    hint("↑/↓", "move", Action::SettingsMove(1)),
    hint("enter", "change", Action::SettingsAdjust(1)),
    hint("esc", "close", Action::SettingsToggle),
];
pub const HINT_MARKS: &[Hint] = &[
    hint("↑/↓", "move", Action::MarkListMove(1)),
    hint("enter", "go", Action::MarkListJump),
    hint("esc", "close", Action::MarkListToggle),
];
pub const HINT_BACKLINKS: &[Hint] = &[
    hint("↑/↓", "move", Action::BacklinksMove(1)),
    hint("enter", "open", Action::BacklinksOpen),
    hint("esc", "close", Action::BacklinksToggle),
];
pub const HINT_FORWARD: &[Hint] = &[
    hint("↑/↓", "move", Action::ForwardMove(1)),
    hint("enter", "open", Action::ForwardOpen),
    hint("esc", "close", Action::ForwardToggle),
];
pub const HINT_INFO: &[Hint] = &[
    hint("↑/↓", "scroll", Action::Scroll(Span::Line, 1)),
    hint("esc", "close", Action::InfoToggle),
];
// The three panes that once fell through to the reading hints as well. The
// lightbox covers the whole screen and paints no footer, but `footer::of`
// still answers for it, and the answer should be true.
pub const HINT_NOTES: &[Hint] = &[
    hint("↑/↓", "move", Action::NoteMove(1)),
    hint("enter", "go", Action::NoteJump),
    hint("e", "edit", Action::NoteEdit),
    hint("esc", "close", Action::NotesToggle),
];
pub const HINT_NOTE_DRAFT: &[Hint] = &[
    says("type", "the note"),
    hint(
        "enter",
        "save",
        Action::NoteInput(crate::action::NoteKey::Save),
    ),
    hint(
        "esc",
        "cancel",
        Action::NoteInput(crate::action::NoteKey::Cancel),
    ),
];
// A footnote box too long for itself, which takes the scroll keys while it
// is up (`App::peek_key`). A box that fits takes none, and the footer keeps
// the reading row, which is then true. Not in the honesty test in keys.rs:
// these keys are decided by `main.rs::key_action` with a box open, so its
// own test (`every_footer_chip_over_a_long_footnote_does_what_its_key_does`)
// presses them through that dispatcher.
pub const HINT_PEEK: &[Hint] = &[
    hint("↑/↓", "scroll", Action::PeekScroll(1)),
    hint("enter", "go to it", Action::PeekGo),
    hint("esc", "close", Action::Dismiss),
];
pub const HINT_PEEK_FITS: &[Hint] = &[
    hint("enter", "go to it", Action::PeekGo),
    hint("esc", "close", Action::Dismiss),
];
pub const HINT_LIGHTBOX: &[Hint] = &[
    hint("→", "next image", Action::ImageStep(1)),
    hint("esc", "close", Action::Dismiss),
];
// A click outside an open menu closes it, so a chip under one can only ever
// close it: `esc close` is the one button that tells the truth. Moving and
// choosing are said, not offered.
pub const HINT_MENU: &[Hint] = &[
    says("click", "a row"),
    hint("esc", "close", Action::MenuClose),
];
pub const HINT_HELP: &[Hint] = &[
    says("type", "to filter"),
    hint("↑/↓", "scroll", Action::Scroll(Span::Line, 1)),
    hint("esc", "close", Action::HelpKey(SearchKey::Cancel)),
];
pub const HINT_HOME_BROWSE: &[Hint] = &[
    hint("↑/↓", "move", Action::HomeMove(1)),
    hint("enter", "open", Action::HomeOpen),
    hint("d", "folder", Action::PickerOpen),
    hint("i", "filter", Action::HomeFilterMode),
    hint("/", "search", Action::HomeSearchMode),
    hint("h", "help", Action::HelpToggle),
];
pub const HINT_HOME_FILTER: &[Hint] = &[
    says("type", "to filter"),
    hint("enter", "open", Action::HomeOpen),
    hint("esc", "close", Action::HomeKey(SearchKey::Cancel)),
];
pub const HINT_HOME_SEARCH: &[Hint] = &[
    says("type", "to search"),
    hint("enter", "open first", Action::HomeOpen),
    hint("tab", "results", Action::HomeOpenResults),
    hint("esc", "close", Action::HomeKey(SearchKey::Cancel)),
];
/// The one-time invitation, shown on the first launch that has never
/// remembered a reading position — and replaced by the ordinary hints the
/// moment the reader does anything at all.
///
/// herdr greets a new session with one line of the same shape. For the
/// reader this design is for — someone whose way into a terminal was an AI
/// agent, with a `PLAN.md` to read and no reason to think a terminal
/// program has menus — this single line is the difference between a reader
/// and a `cat`.
pub const HINT_FIRST_RUN: &[Hint] = &[says("click", "anything"), says("right-click", "for a menu")];
pub const HINT_HOME_PICKER: &[Hint] = &[
    says("type", "to filter"),
    // The arrows, not `^j/^k`, even though the Ctrl pair is the less
    // guessable one: whoever is reading the footer is reading it because
    // they do not know what to do, and a cryptic key there leaves them
    // unsure the obvious one even works. A vim user who presses `j`, watches
    // it land in the filter, and opens help finds `Ctrl-J`/`Ctrl-K` at once.
    hint("↑/↓", "move", Action::HomeMove(1)),
    hint("→", "go in", Action::PickerDescend),
    hint("enter", "open", Action::PickerChoose),
    hint("esc", "close", Action::HomeKey(SearchKey::Cancel)),
];
