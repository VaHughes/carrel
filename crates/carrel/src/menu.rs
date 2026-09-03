//! Menus: what a reader who does not know the keys reaches for.
//!
//! NO RATATUI — `scripts/check-discipline.sh` rule 6, the same standing as
//! [`crate::footer`]. The geometry here is in terminal cells because the
//! terminal is what carrel paints today, but nothing in this file draws: a
//! GTK frontend gets the item list, the grouping and the accelerators for
//! free and hangs its own widget off them.
//!
//! # Two menus
//!
//! A **context** menu, built for the doc byte under the pointer and opened
//! *after* that byte is known — Neovim's `mousemodel=popup_setpos` rule, so
//! its items act on what the pointer was on rather than on wherever the view
//! happens to be. A **global** menu, the same on every right-click in the
//! chrome and behind the `≡` at the right end of the status row, which is
//! the visible affordance that teaches a new reader menus exist at all.
//!
//! # Where the accelerators come from
//!
//! [`crate::keys::accel`], never a literal. It is an exhaustive match over
//! `Action`, so the key a menu row advertises is the key the dispatcher
//! actually binds, and a rebinding moves both at once. A menu that lies
//! about its shortcut teaches the wrong key to exactly the reader who was
//! trying to learn one.

use carrel_core::{DocByte, LinkId, NodeKind};

use crate::action::{Action, Direction, Zone};
use crate::app::App;
use crate::keys;

/// The floor on a menu's width, herdr's: a two-item menu of short labels
/// still reads as a box rather than as a sliver.
const MIN_W: u16 = 14;

/// Border plus one cell of padding on each side.
const CHROME_W: u16 = 4;

/// The gap between the label column and the accelerator column.
const GAP: u16 = 2;

/// One row of a menu.
///
/// `action: None` is a **gap** — the blank line CUA specifies between groups
/// (SC26-4583-00 §3.5.5), rather than Borland's `├───┤` rule, which is louder
/// than it needs to be. `enabled: false` is a greyed row: it shows what the
/// menu can do here, and declines to do it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub label: &'static str,
    pub accel: &'static str,
    pub action: Option<Action>,
    pub enabled: bool,
}

impl Item {
    /// A row whose accelerator is looked up from the keymap.
    #[must_use]
    fn new(label: &'static str, action: Action) -> Self {
        Self {
            label,
            accel: keys::accel(action).map_or("", keys::first),
            action: Some(action),
            enabled: true,
        }
    }

    /// A row that acts on a byte the pointer named, and therefore has no
    /// accelerator of its own — it borrows the one from the key that does
    /// the same job from the keyboard (`za` folds a section; so does this,
    /// on the section under the pointer).
    #[must_use]
    fn like(label: &'static str, keyed: Action, action: Action) -> Self {
        Self {
            label,
            accel: keys::accel(keyed).map_or("", keys::first),
            action: Some(action),
            enabled: true,
        }
    }

    /// A row reachable only through this menu, or only by a pointer gesture.
    ///
    /// Its accelerator column stays empty rather than borrowing a key that
    /// does something adjacent: `y` copies the focused CODE BLOCK, so
    /// printing it beside `Copy selection` would teach a key that does not
    /// do what the row above it says.
    #[must_use]
    const fn plain(label: &'static str, action: Action) -> Self {
        Self {
            label,
            accel: "",
            action: Some(action),
            enabled: true,
        }
    }

    /// A row with a literal accelerator, for the one binding that differs
    /// between the reader and the home screen.
    #[must_use]
    const fn spelled(label: &'static str, accel: &'static str, action: Action) -> Self {
        Self {
            label,
            accel,
            action: Some(action),
            enabled: true,
        }
    }

    /// The blank line between two groups.
    #[must_use]
    const fn gap() -> Self {
        Self {
            label: "",
            accel: "",
            action: None,
            enabled: false,
        }
    }

    #[must_use]
    fn greyed(self) -> Self {
        Self {
            enabled: false,
            ..self
        }
    }

    /// Can the pointer or `Enter` act on this row?
    #[must_use]
    pub const fn pickable(&self) -> bool {
        self.action.is_some() && self.enabled
    }
}

/// An open menu: its rows, where it was anchored, and which row is lit.
///
/// The anchor is stored and the box is derived from it every frame
/// ([`Menu::zone`]), so a resize under an open menu re-clamps rather than
/// leaving a box half off the screen — the same "derive, never store"
/// discipline the reader applies to rows.
#[derive(Clone, Debug)]
pub struct Menu {
    pub items: Vec<Item>,
    /// **Nothing is lit when a menu opens** — Borland's rule, and the reason
    /// given for it in the Turbo Vision sources was simply that it would
    /// look ugly. It is also honest: no row has been chosen yet.
    pub selected: Option<usize>,
    /// The cell the pointer was on when this opened.
    pub at: (u16, u16),
}

impl Menu {
    #[must_use]
    pub const fn new(items: Vec<Item>, at: (u16, u16)) -> Self {
        Self {
            items,
            selected: None,
            at,
        }
    }

    /// The widest label, and the widest accelerator, in display cells.
    fn columns(&self) -> (u16, u16) {
        self.items.iter().fold((0, 0), |(l, a), i| {
            (
                l.max(carrel_core::display_width(i.label)),
                a.max(carrel_core::display_width(i.accel)),
            )
        })
    }

    /// The box, clamped into a `cols × rows` viewport.
    ///
    /// Anchored at the pointer and dropped one row below it; **flipped above
    /// when it would run off the bottom** (vim's `popupmnu.cpp`), because a
    /// menu that opens off screen is a menu with no items.
    #[must_use]
    pub fn zone(&self, cols: u16, rows: u16) -> Zone {
        let (label, accel) = self.columns();
        let content = label + if accel == 0 { 0 } else { GAP + accel };
        let w = content.saturating_add(CHROME_W).max(MIN_W).min(cols.max(1));
        let h = u16::try_from(self.items.len())
            .unwrap_or(u16::MAX)
            .saturating_add(2)
            .min(rows.max(1));
        let (cx, cy) = self.at;
        let below = cy.saturating_add(1);
        let y = if below.saturating_add(h) <= rows {
            below
        } else {
            cy.saturating_sub(h)
        };
        Zone::new(
            cx.min(cols.saturating_sub(w)),
            y.min(rows.saturating_sub(h)),
            w,
            h,
        )
    }

    /// Where the accelerator column starts, relative to the box's left edge.
    #[must_use]
    pub fn accel_dx(&self, w: u16) -> u16 {
        let (_, accel) = self.columns();
        w.saturating_sub(2 + accel).max(2)
    }

    /// Move the selection by `n`, skipping gaps and greyed rows.
    ///
    /// Clamps at both ends rather than wrapping, and from nothing at all
    /// lands on the first row going down, the last going up — so the first
    /// `Down` after opening lights the top item, which is what a hand
    /// reaching for the keyboard expects.
    pub fn step(&mut self, n: i32) {
        let picks: Vec<usize> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, i)| i.pickable())
            .map(|(i, _)| i)
            .collect();
        let Some(last) = picks.len().checked_sub(1) else {
            return;
        };
        let at = match self
            .selected
            .and_then(|s| picks.iter().position(|&p| p == s))
        {
            Some(i) => (i64::try_from(i).unwrap_or(0) + i64::from(n))
                .clamp(0, i64::try_from(last).unwrap_or(0)),
            None if n >= 0 => 0,
            None => i64::try_from(last).unwrap_or(0),
        };
        self.selected = Some(picks[usize::try_from(at).unwrap_or(0)]);
    }

    /// Light row `i`, or nothing when `i` names no pickable row.
    pub fn hover(&mut self, i: usize) {
        self.selected = self.items.get(i).filter(|it| it.pickable()).map(|_| i);
    }

    /// What `Enter` would do.
    #[must_use]
    pub fn chosen(&self) -> Option<Action> {
        self.item(self.selected?)
    }

    /// What a click on row `i` would do — `None` for a gap, a greyed row, or
    /// an index a stale frame left behind.
    #[must_use]
    pub fn item(&self, i: usize) -> Option<Action> {
        self.items.get(i).filter(|it| it.pickable())?.action
    }
}

// ---------------------------------------------------------------------------
// The global menu
// ---------------------------------------------------------------------------

/// Every reader command that is a toggle or a destination, grouped.
///
/// Deliberately short. herdr keeps its launcher menu to a screenful of
/// single words and puts everything contextual in the other menu; a global
/// menu that grows into a copy of the help sheet stops being scannable, and
/// the help sheet already exists one row down.
#[must_use]
pub fn global(app: &App) -> Vec<Item> {
    if app.is_home() {
        return vec![
            Item::new("Filter names", Action::HomeFilterMode),
            Item::new("Search in files", Action::HomeSearchMode),
            Item::new("Directory…", Action::PickerOpen),
            Item::gap(),
            Item::new("Themes", Action::ThemeCycle),
            Item::new("Key hints", Action::HintsToggle),
            Item::gap(),
            Item::new("Help", Action::HelpToggle),
            // `map_home` binds `q` to Quit; the reader binds it to CloseFile
            // and spells quit `Q`. One accelerator cannot be right for both,
            // so the home menu says what the home screen does.
            Item::spelled("Quit", "q", Action::Quit),
        ];
    }
    vec![
        // The click-first reader's way back out of a link they followed.
        // `Ctrl-O` is the only route there from the keyboard, and it is not
        // a chord anyone guesses; greyed with nothing to go back to, so the
        // row still says the way exists.
        if app.history.is_empty() {
            Item::new("Back", Action::Back).greyed()
        } else {
            Item::new("Back", Action::Back)
        },
        Item::gap(),
        Item::new("Document info", Action::InfoToggle),
        Item::new("Spotlight", Action::FocusToggle),
        Item::new("Auto-read", Action::AutoToggle),
        // Following pins the view to the end of a document that is still
        // growing. Nothing is growing here, so the row says so rather than
        // jumping to the end and claiming to follow it.
        if app.streaming {
            Item::new("Follow the end", Action::FollowToggle)
        } else {
            Item::new("Follow the end", Action::FollowToggle).greyed()
        },
        Item::gap(),
        Item::new("Themes", Action::ThemeCycle),
        Item::new("Key hints", Action::HintsToggle),
        Item::new("Breadcrumb", Action::BreadcrumbToggle),
        Item::gap(),
        Item::new("Help", Action::HelpToggle),
        Item::new(
            if app.home_stash.is_some() {
                "Close file"
            } else {
                "Close"
            },
            Action::CloseFile,
        ),
        Item::new("Quit", Action::Quit),
    ]
}

// ---------------------------------------------------------------------------
// The context menu
// ---------------------------------------------------------------------------

/// What the pointer was on. The context menu's head is a function of this
/// and nothing else.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Under {
    Heading,
    /// A link, and whether its destination is external — a URL is copied,
    /// never opened, so there is only one thing to offer for one.
    Link(LinkId, bool),
    /// A code block, and whether it has a rendered form to flip to.
    Code(bool),
    Table,
    /// A `[^name]` reference, which has a definition to jump to.
    Footnote,
    Task,
    Text,
}

/// Classify the doc byte under the pointer.
fn under(app: &App, byte: u32) -> Under {
    let block = app.doc.block_at_doc(DocByte(byte));
    let node = app.doc.node_for_block(block);
    // A link is an inline run, so it outranks whatever block it sits in: a
    // link in a heading is a link first. Only this block's runs are searched
    // — the pointer cannot be on a run outside the block it landed in.
    if let Some(id) = node
        .inlines
        .iter()
        .find(|i| i.link.is_some() && i.doc.contains(&byte))
        .and_then(|i| i.link)
    {
        let dest = app.doc.links[id.0 as usize].as_ref();
        return Under::Link(id, crate::app::has_scheme(dest));
    }
    match &node.kind {
        NodeKind::Heading { .. } => Under::Heading,
        NodeKind::CodeBlock { lang } => {
            Under::Code(lang.as_deref() == Some("mermaid") && app.diagram_art.contains_key(&block))
        }
        NodeKind::Math => Under::Code(true),
        NodeKind::Table { .. } => Under::Table,
        _ if on_footnote_ref(app, byte) => Under::Footnote,
        _ if node.prefix.as_ref().and_then(|p| p.task).is_some() => Under::Task,
        _ => Under::Text,
    }
}

/// Is `byte` inside a `[^name]` reference run?
///
/// The reference is literal text — `footnote_refs` reports where each one
/// starts, and its painted width is `[^` + the name + `]`.
fn on_footnote_ref(app: &App, byte: u32) -> bool {
    app.doc.footnote_refs().into_iter().any(|(name, at)| {
        let end = at.saturating_add(u32::try_from(name.len() + 3).unwrap_or(u32::MAX));
        (at..end).contains(&byte)
    })
}

/// The menu for the doc byte under the pointer: a contextual head, a blank
/// line, then the short tail every context menu ends with.
#[must_use]
pub fn context(app: &App, byte: u32) -> Vec<Item> {
    let mut items = match under(app, byte) {
        Under::Heading => vec![
            Item::like(
                "Fold this section",
                Action::FoldToggle,
                Action::FoldAt(byte),
            ),
            Item::new("Fold all", Action::FoldAll),
            Item::new("Unfold all", Action::UnfoldAll),
        ],
        Under::Link(id, external) => {
            let mut v = Vec::new();
            // An external destination is copied and never opened — carrel
            // spawns no programs — so for one there is only the one row.
            if !external {
                v.push(Item::like(
                    "Open link",
                    Action::LinkFollow,
                    Action::LinkOpen(id.0),
                ));
            }
            v.push(Item::plain("Copy link", Action::LinkCopy));
            v.push(Item::new("What links here", Action::BacklinksToggle));
            v.push(Item::new("What this points at", Action::ForwardToggle));
            v
        }
        Under::Code(rendered) => {
            let mut v = vec![
                Item::new("Copy code block", Action::YankBlock),
                Item::new("Next code block", Action::CodeStep(1)),
            ];
            if rendered {
                v.push(Item::new("Rendered ↔ source", Action::RenderedToggle));
            }
            v
        }
        Under::Table => vec![Item::new("Cards ↔ wrapped", Action::TableToggle)],
        Under::Footnote => vec![Item::new("Go to its definition", Action::FootnoteJump)],
        Under::Task => vec![Item::new("Next task", Action::TaskStep(1))],
        Under::Text => {
            let mut v = Vec::new();
            // Only offered when there is one: a menu row that copies nothing
            // is worse than no row, because it looks like it worked.
            if app.selection.is_some() {
                v.push(Item::plain("Copy selection", Action::SelectRelease));
            }
            v.push(Item::plain("Select word", Action::SelectWord(byte)));
            v.push(Item::plain("Select block", Action::SelectBlock(byte)));
            v
        }
    };
    items.push(Item::gap());
    items.push(Item::new("Bookmark here", Action::MarkToggle));
    items.push(Item::gap());
    // `…` means "opens something that asks for more" — CUA again, and the
    // reason the three overlays carry it and the toggles above do not.
    items.push(Item::new("Search…", Action::SearchOpen(Direction::Forward)));
    items.push(Item::new("Outline…", Action::OutlineToggle));
    items.push(Item::new("Bookmarks…", Action::MarkListToggle));
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use carrel_core::Document;

    fn app(src: &str) -> App {
        App::new("t.md".into(), Document::parse(src), 80, 24)
    }

    fn menu(items: Vec<Item>) -> Menu {
        Menu::new(items, (0, 0))
    }

    #[test]
    fn a_box_is_two_wider_than_its_widest_row_on_each_side() {
        let m = menu(vec![Item::spelled("Themes", "T", Action::ThemeCycle)]);
        // 6 label + 2 gap + 1 accel = 9 content, + 4 chrome = 13 — under the
        // floor, so the floor wins.
        assert_eq!(m.zone(80, 24).w, MIN_W);

        let m = menu(vec![Item::spelled(
            "What this points at",
            "l",
            Action::ForwardToggle,
        )]);
        assert_eq!(m.zone(80, 24).w, 19 + GAP + 1 + CHROME_W);
    }

    #[test]
    fn a_menu_drops_below_the_pointer_and_flips_above_at_the_bottom() {
        let items = vec![Item::new("Themes", Action::ThemeCycle); 4];
        // Row 2 of 24: it fits below, so it opens below.
        let m = Menu::new(items.clone(), (10, 2));
        let z = m.zone(80, 24);
        assert_eq!((z.x, z.y, z.h), (10, 3, 6));

        // Row 22 of 24: six rows will not fit below, so it flips above and
        // its bottom edge sits on the pointer's row.
        let m = Menu::new(items, (10, 22));
        let z = m.zone(80, 24);
        assert_eq!((z.y, z.h), (16, 6), "flipped above the pointer");
        assert_eq!(z.y + z.h, 22, "its last row is the one above the pointer");
    }

    #[test]
    fn a_menu_at_the_right_edge_is_pulled_back_on_screen() {
        let m = Menu::new(
            vec![Item::new("Document info", Action::InfoToggle)],
            (78, 1),
        );
        let z = m.zone(80, 24);
        assert!(z.x + z.w <= 80, "clamped: {z:?}");
        // And it never goes negative on a viewport narrower than the box.
        let z = m.zone(6, 3);
        assert!(z.x + z.w <= 6 && z.y + z.h <= 3, "tiny viewport: {z:?}");
    }

    #[test]
    fn nothing_is_lit_until_something_moves_and_gaps_are_skipped() {
        let mut m = menu(vec![
            Item::new("Themes", Action::ThemeCycle),
            Item::gap(),
            Item::new("Help", Action::HelpToggle),
        ]);
        assert_eq!(m.selected, None, "Borland: nothing pre-highlighted");
        m.step(1);
        assert_eq!(m.selected, Some(0));
        m.step(1);
        assert_eq!(m.selected, Some(2), "the gap is not a row");
        m.step(1);
        assert_eq!(m.selected, Some(2), "clamps rather than wrapping");
        m.step(-5);
        assert_eq!(m.selected, Some(0), "and clamps the other way");
    }

    #[test]
    fn up_from_nothing_lands_on_the_last_row() {
        let mut m = menu(vec![
            Item::new("Themes", Action::ThemeCycle),
            Item::new("Help", Action::HelpToggle),
        ]);
        m.step(-1);
        assert_eq!(m.selected, Some(1));
    }

    #[test]
    fn a_greyed_row_can_be_neither_hovered_nor_chosen() {
        let mut m = menu(vec![
            Item::new("Follow the end", Action::FollowToggle).greyed(),
            Item::new("Help", Action::HelpToggle),
        ]);
        m.hover(0);
        assert_eq!(m.selected, None);
        assert_eq!(m.item(0), None, "a click on it does nothing");
        m.step(1);
        assert_eq!(m.selected, Some(1), "and the keyboard steps over it");
        assert_eq!(m.chosen(), Some(Action::HelpToggle));
    }

    #[test]
    fn hovering_off_a_row_puts_the_light_out() {
        let mut m = menu(vec![Item::new("Help", Action::HelpToggle), Item::gap()]);
        m.hover(0);
        assert_eq!(m.selected, Some(0));
        m.hover(1);
        assert_eq!(m.selected, None, "a gap is not a row");
        m.hover(99);
        assert_eq!(m.selected, None, "nor is an index off the end");
    }

    #[test]
    fn every_accelerator_a_menu_prints_is_a_key_the_help_sheet_names() {
        let a = app("# T\n\nsee [x](notes.md) and `code`\n");
        let rows: String = keys::READER_HELP
            .iter()
            .chain(keys::HOME_HELP)
            .map(|(k, _)| *k)
            .collect::<Vec<_>>()
            .join(" | ");
        let mut checked = 0;
        for item in global(&a).iter().chain(&context(&a, 0)) {
            if item.accel.is_empty() {
                assert!(
                    item.action.is_none() || keys::accel(item.action.unwrap()).is_none(),
                    "{}: has a key but does not print it",
                    item.label
                );
                continue;
            }
            assert!(
                rows.contains(item.accel),
                "{}: prints {:?}, which no help row documents",
                item.label,
                item.accel
            );
            assert!(
                !item.accel.contains(' '),
                "{}: a menu prints ONE key, not {:?}",
                item.label,
                item.accel
            );
            checked += 1;
        }
        assert!(checked > 10, "the menus must be walked, got {checked}");
    }

    #[test]
    fn the_context_menu_head_follows_the_pointer() {
        let a = app("# Heading\n\nplain words\n\n```rust\nlet x = 1;\n```\n");
        let head = |byte: u32| context(&a, byte)[0].label;
        let at = |needle: &str| u32::try_from(a.doc.text.find(needle).expect(needle)).unwrap();

        assert_eq!(head(at("Heading")), "Fold this section");
        assert_eq!(head(at("plain")), "Select word");
        assert_eq!(head(at("let x")), "Copy code block");
    }

    #[test]
    fn a_url_is_offered_for_copying_and_a_local_file_for_opening() {
        let a = app("see [out](https://example.com) and [in](notes.md)\n");
        let at = |needle: &str| u32::try_from(a.doc.text.find(needle).expect(needle)).unwrap();
        let labels =
            |b: u32| -> Vec<&'static str> { context(&a, b).iter().map(|i| i.label).collect() };
        assert!(
            !labels(at("out")).contains(&"Open link"),
            "carrel does not open a URL — it copies it"
        );
        assert!(labels(at("out")).contains(&"Copy link"));
        assert!(labels(at("in")).contains(&"Open link"));
    }

    #[test]
    fn every_context_menu_ends_with_the_same_tail() {
        let a = app("# H\n\ntext\n\n| a | b |\n| - | - |\n| 1 | 2 |\n");
        for byte in [0u32, 5, 12] {
            let items = context(&a, byte);
            let tail: Vec<&'static str> = items.iter().rev().take(3).map(|i| i.label).collect();
            assert_eq!(tail, vec!["Bookmarks…", "Outline…", "Search…"]);
            assert!(
                items.iter().any(|i| i.label == "Bookmark here"),
                "byte {byte}"
            );
        }
    }

    /// Where a pointer reaches an action, or why it deliberately cannot.
    ///
    /// **Exhaustive**, so a new `Action` fails to compile until someone
    /// decides which it is. This is the twin of
    /// `keys::every_action_is_documented_or_deliberately_not`: that one says
    /// every action names a key, this one says every action a reader would
    /// ever want has a way in that needs no keys at all — which is the whole
    /// claim the click-first pivot makes.
    #[derive(Debug, PartialEq, Eq)]
    enum Reach {
        /// A click on the document itself.
        Doc,
        /// A painted button in the chrome: the footer's hints, the status
        /// row's words, the lamp, the `≡`.
        Chrome,
        /// A row of an open pane, or a row of the home list.
        Pane,
        /// A menu row, named so the test can go and find it.
        Menu(&'static str),
        /// Reachable only from the keyboard, on purpose.
        KeyboardOnly,
        /// Not an intent a person forms: a clock tick, or plumbing.
        Internal,
    }

    // One arm per Action, each with its own comment — merging them by body
    // would be merging the reasons, and the reasons are the content here.
    #[allow(clippy::too_many_lines, clippy::match_same_arms)]
    fn reach(a: Action) -> Reach {
        use Action as A;
        use Reach::{Chrome, Doc, Internal, KeyboardOnly, Menu, Pane};
        match a {
            // --- the chrome is a row of buttons (step 4) ---
            A::Scroll(..) => Chrome,      // `j/k scroll`, `spc page`
            A::SearchOpen(_) => Chrome,   // `/ search`
            A::OutlineToggle => Chrome,   // `o outline`
            A::HelpToggle => Chrome,      // `h more`
            A::HintsToggle => Chrome,     // the lamp
            A::ThemeCycle => Chrome,      // `T theme`
            A::CloseFile => Chrome,       // `q home` / `q quit`
            A::MatchStep(_) => Chrome,    // `n/N next/prev`
            A::Recenter(_) => Chrome,     // `zz center`
            A::Dismiss => Chrome,         // `esc clear`
            A::LinkStep(_) => Chrome,     // `tab next`
            A::LinkFollow => Chrome,      // `enter follow`
            A::FollowToggle => Chrome,    // `F follow the end`
            A::YankBlock => Chrome,       // `y copy block`
            A::OutlineJump => Chrome,     // `enter go`
            A::OutlineKey(_) => Chrome,   // `esc back`
            A::HomeMove(_) => Chrome,     // `j/k move`, `↑/↓ move`
            A::HomeOpen => Chrome,        // `enter open`
            A::PickerOpen => Chrome,      // `d directory`
            A::HomeFilterMode => Chrome,  // `i filter`
            A::HomeSearchMode => Chrome,  // `/ search`
            A::HomeKey(_) => Chrome,      // `esc back`
            A::PickerChoose => Chrome,    // `enter choose`
            A::MenuOpen { .. } => Chrome, // the `≡`, and every right-click
            A::HomeUp => Chrome,          // the `↑` at the head of the path row
            A::HomeCrumb(_) => Chrome,    // a segment of the path row
            A::GoHome => Chrome,          // the `⌂` on the reader's status row

            // --- the document itself ---
            A::LinkOpen(_) => Doc,
            A::FoldAt(_) => Doc,
            A::OutlineJumpTo(_) => Doc, // the margin outline
            A::ScrollTo(_) => Doc,      // the scrollbar
            A::SelectAnchor(_) | A::SelectDrag(_) | A::SelectRelease => Doc,
            A::SelectWord(_) | A::SelectBlock(_) => Doc,

            // --- a row of a pane, or of the home list ---
            A::BacklinksOpenAt(_)
            | A::ForwardOpenAt(_)
            | A::MarkListJumpAt(_)
            | A::OutlineJumpAt(_)
            | A::HomeSelect(_)
            | A::PickerSelect(_)
            | A::HomeResume(_) => Pane,

            // --- only a menu offers these ---
            A::Back => Menu("Back"),
            A::InfoToggle => Menu("Document info"),
            A::FocusToggle => Menu("Spotlight"),
            A::AutoToggle => Menu("Auto-read"),
            A::BreadcrumbToggle => Menu("Breadcrumb"),
            A::Quit => Menu("Quit"),
            A::FoldAll => Menu("Fold all"),
            A::UnfoldAll => Menu("Unfold all"),
            A::LinkCopy => Menu("Copy link"),
            A::BacklinksToggle => Menu("What links here"),
            A::ForwardToggle => Menu("What this points at"),
            A::CodeStep(_) => Menu("Next code block"),
            A::RenderedToggle => Menu("Rendered ↔ source"),
            A::TableToggle => Menu("Cards ↔ wrapped"),
            A::FootnoteJump => Menu("Go to its definition"),
            A::TaskStep(_) => Menu("Next task"),
            A::MarkToggle => Menu("Bookmark here"),
            A::MarkListToggle => Menu("Bookmarks…"),

            // --- the menu's own machinery ---
            A::MenuMove(_) | A::MenuHover(_) | A::MenuChoose | A::MenuPick(_) | A::MenuClose => {
                Internal
            }

            // --- deliberately keyboard-only ---
            //
            // Every one of these is a REFINEMENT of something a pointer can
            // already do directly, and a menu row for it would be a row that
            // says "the same, but by a different route". A click picks the
            // row it means; it never needs to walk toward it.
            // `za` folds the section at the TOP OF THE VIEW. A pointer says
            // which section it means and sends `FoldAt`, from the fold marker
            // or from the menu row — so the toggle itself has no pointer
            // route, and does not need one.
            A::FoldToggle => KeyboardOnly,
            A::GoToStart | A::GoToEnd => KeyboardOnly, // the scrollbar goes there
            A::GoToRow(_) | A::BlockStep(_) => KeyboardOnly, // counts and steps
            A::HomeGo(_) => KeyboardOnly,
            A::MarkNext => KeyboardOnly, // `Bookmarks…` lists them
            A::BacklinksMove(_) | A::ForwardMove(_) | A::MarkListMove(_) | A::OutlineMove(_) => {
                KeyboardOnly // a click lands on the row; nothing steps toward it
            }
            A::BacklinksOpen | A::ForwardOpen | A::MarkListJump => KeyboardOnly,
            A::HomeNormalMode | A::PickerCancel => KeyboardOnly, // Esc, and a click outside

            // --- not intents at all ---
            A::SearchKey(_) => Internal, // typing
            A::Hover(_) => Internal,     // decoration; it decides nothing
            A::AutoTick => Internal,     // a clock
            A::Absorb => Internal,       // dropped at the hit-test
        }
    }

    /// Every menu row must be one the classifier claims is menu-reachable,
    /// under exactly the label it prints.
    ///
    /// This is the direction that can be walked exhaustively, and it is the
    /// one that catches drift: rename a row, or move it out of the menu, and
    /// the arm above it stops matching. (An arm naming a label no menu has
    /// is the residual gap — visible in review, because writing that arm is
    /// something you only do while adding the row it names.)
    #[test]
    fn every_action_is_reachable_by_pointer_or_deliberately_not() {
        let mut a = app(concat!(
            "# Heading\n\nplain prose, a [local](notes.md), a [url](https://e.com),\n",
            "and a footnote[^n].\n\n- [ ] a task\n\n| a | b |\n| - | - |\n| 1 | 2 |\n\n",
            "```rust\nlet x = 1;\n```\n\n$$ x^2 $$\n\n[^n]: the definition\n"
        ));
        let mut menus = vec![global(&a)];
        for byte in 0..u32::try_from(a.doc.text.len()).unwrap() {
            menus.push(context(&a, byte));
        }
        // With something selected, `Copy selection` joins the plain-text head.
        a.selection = Some(0..4);
        menus.push(context(&a, 0));
        let home = App::new_home(std::path::PathBuf::from("."), vec![], 80, 24);
        menus.push(global(&home));

        let mut seen = std::collections::BTreeSet::new();
        for item in menus.iter().flatten() {
            let Some(action) = item.action else { continue };
            // A row whose action the classifier calls unreachable by pointer
            // is a contradiction: it is in a menu, and a menu is a pointer.
            let r = reach(action);
            assert!(
                !matches!(r, Reach::KeyboardOnly | Reach::Internal),
                "the menu row {:?} runs {action:?}, which is classified {r:?}",
                item.label
            );
            // And a menu-ONLY action must be in the menu under the label its
            // arm names, so a rename cannot quietly orphan the classifier.
            if let Reach::Menu(named) = r {
                assert_eq!(
                    named, item.label,
                    "{action:?} is classified as the row {named:?}, printed as {:?}",
                    item.label
                );
            }
            seen.insert(item.label);
        }
        // The corpus has to actually exercise every branch, or the loop
        // above passes by visiting nothing.
        for label in [
            "Back",
            "Fold this section",
            "Copy link",
            "What links here",
            "Rendered ↔ source",
            "Cards ↔ wrapped",
            "Go to its definition",
            "Next task",
            "Bookmarks…",
        ] {
            assert!(seen.contains(label), "the corpus never produced {label:?}");
        }
    }

    #[test]
    fn following_is_greyed_until_something_is_growing() {
        let mut a = app("body");
        let row = |a: &App| -> Item {
            *global(a)
                .iter()
                .find(|i| i.label == "Follow the end")
                .expect("the row exists in both states")
        };
        assert!(!row(&a).pickable(), "nothing is growing yet");
        a.streaming = true;
        assert!(
            row(&a).pickable(),
            "a pipe is open: following means something"
        );
    }
}
