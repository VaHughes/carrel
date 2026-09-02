//! The intent enum — **the seam a second frontend reuses.**
//!
//! Discipline #4: share an `Action` enum, bind keys and mouse per-frontend. A
//! GTK build will produce these from `GtkEventControllerKey` and menu items and
//! feed them to the same [`crate::app::update`]. Nothing here mentions a key,
//! a terminal, or a pixel.

/// How far one scroll step moves.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Span {
    Line,
    HalfPage,
    Page,
}

/// Where [`Action::Recenter`] puts the current match.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Where {
    Top,
    Middle,
    Bottom,
}

/// Which way a search runs when it opens. Both find every match; the direction
/// only decides which one you land on first.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Backward,
}

/// A keystroke while the search prompt is open.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SearchKey {
    Char(char),
    Backspace,
    /// Keep the matches, close the prompt.
    Accept,
    /// Discard the matches, close the prompt, restore the previous position.
    Cancel,
}

/// Which end of a list.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Edge {
    First,
    Last,
}

/// One thing the reader can be asked to do.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Signed: positive is toward the end of the document.
    Scroll(Span, i32),
    GoToStart,
    GoToEnd,
    /// `{count}G` — an absolute visual row, as `less` treats lines.
    GoToRow(u32),
    /// `{` and `}`. Signed, in blocks.
    BlockStep(i32),
    /// `zz` / `zt` / `zb`. Positions the CURRENT MATCH, not a cursor — a reader
    /// has no cursor. A no-op when no match is current.
    Recenter(Where),
    SearchOpen(Direction),
    SearchKey(SearchKey),
    /// `n` / `N`. Signed, count-multiplied.
    MatchStep(i32),
    /// `Tab` / `Shift-Tab`. Cycle link selection in document order.
    LinkStep(i32),
    /// `Enter`. Follow the selected link if it is a relative markdown file.
    LinkFollow,
    /// A click on a painted link: select it and follow it in one intent. The
    /// payload indexes `Document::links` — the same currency `LinkId` wraps,
    /// kept primitive here so this file stays free of state-layer types.
    LinkOpen(u32),
    /// `Ctrl-O`. Pop the history stack.
    Back,
    /// `Esc` with nothing pending: clear transient selection state.
    Dismiss,
    /// `q` in the reader: back to the home screen when one is behind this
    /// document, otherwise quit — pager semantics for direct opens.
    CloseFile,
    /// `T`: advance to the next theme, live. Handled by the event loop, not
    /// `update` — the active palette is presentation state and never enters
    /// the (ratatui-free) `App`.
    ThemeCycle,
    /// `t`: flip every table in the document between card view and the
    /// padded-wrap rendering. Transient — documents open in auto (cards).
    TableToggle,
    /// `h` / `F1`: toggle the key-binding overlay. While it is up, scroll
    /// actions scroll the sheet, dismiss-shaped actions close it, and
    /// everything else is inert.
    HelpToggle,
    /// `H`, or a click on the lamp: show / hide the lamplight hint footer.
    /// Persisted, so the choice survives relaunch.
    HintsToggle,
    /// `B`: show / hide the breadcrumb band. Persisted like the hints.
    /// Only a document with headings paints one either way.
    BreadcrumbToggle,
    /// `S`: spotlight — dim every block except the one nearest the centre
    /// of the view. Pure presentation: layout and positions never hear
    /// about it.
    FocusToggle,
    /// `I`: show / hide the document-info card — words, minutes, structure.
    /// Derived fresh every frame it shows; nothing to go stale.
    InfoToggle,
    /// `za`: fold or unfold the innermost section at the top of the view —
    /// the same byte the breadcrumb derives from, so the two agree on
    /// "current section".
    FoldToggle,
    /// `zM`: fold every section — the document as its own table of contents.
    FoldAll,
    /// `zR`: open everything back up.
    UnfoldAll,
    /// A click on a fold marker: fold or unfold whatever this doc byte is
    /// inside — a `<details>` summary, else the heading whose section it is.
    /// The marker paints outside every clickable text span, so it needs an
    /// intent of its own rather than arriving as a selection.
    FoldAt(u32),
    /// Mouse press in the text area: the `(start, end)` doc bytes of the
    /// grapheme cluster under the pointer. Replaces any existing selection;
    /// selects nothing until the pointer moves. The frontend converts
    /// pixels/cells to doc bytes BEFORE anything enters the state machine —
    /// a GTK frontend maps its own pointer events to these same intents.
    SelectAnchor((u32, u32)),
    /// Drag: the cluster now under the pointer. The selection spans both
    /// endpoint clusters, in either drag direction.
    SelectDrag((u32, u32)),
    /// Release: copy the selection to the clipboard outbox and keep it
    /// painted.
    SelectRelease,
    /// Double-click: select the word (alphanumeric/`_`/`-` run) at a byte.
    SelectWord(u32),
    /// Triple-click: select the whole block at a byte — THE copy-code-block
    /// gesture.
    SelectBlock(u32),
    /// `m`: flip every mermaid block between rendered box art and source,
    /// like `t` for tables.
    RenderedToggle,
    /// `%`: jump between a footnote reference and its definition. From a
    /// reference (or anywhere above one), to that reference's definition;
    /// from inside a definition, back to its first reference. Pushes a
    /// history entry either way, so `Ctrl-O` returns.
    FootnoteJump,
    /// Open or close the backlinks pane.
    BacklinksToggle,
    /// Move the backlinks cursor.
    BacklinksMove(i32),
    /// Open the selected backlink.
    BacklinksOpen,
    /// Open a backlink by absolute row — a click, which already said which
    /// row it meant. Clamped by the receiver, so a target left over from a
    /// frame that no longer describes the pane is inert rather than wrong.
    BacklinksOpenAt(u32),
    /// `l`: open or close the forward-links pane — what this document
    /// points at, the mirror of the backlinks pane.
    ForwardToggle,
    /// Move the forward-links cursor.
    ForwardMove(i32),
    /// Open the selected forward link when it resolves locally; an external
    /// destination is shown as a note instead of being fetched.
    ForwardOpen,
    /// Open a forward link by absolute row. See [`Action::BacklinksOpenAt`].
    ForwardOpenAt(u32),
    /// Jump to a heading block — the margin outline's click.
    OutlineJumpTo(carrel_core::BlockIdx),
    /// Open a continue-reading row by index.
    HomeResume(usize),
    /// Toggle a bookmark at the current position.
    MarkToggle,
    /// Jump to the next bookmark, wrapping.
    MarkNext,
    /// `"`: open or close the bookmark list — every mark with its context
    /// line, Enter jumps. `'` walks them blind; this shows the whole list.
    MarkListToggle,
    /// Move the bookmark-list cursor.
    MarkListMove(i32),
    /// Jump to the selected bookmark.
    MarkListJump,
    /// Jump to a bookmark by absolute row. See [`Action::BacklinksOpenAt`].
    MarkListJumpAt(u32),
    /// Pin the view to the end of a document that is still growing.
    FollowToggle,
    /// `A`: start or stop auto-read — the view drifts down one row per
    /// [`crate::app::AUTO_READ_MS`] until any deliberate motion takes over.
    AutoToggle,
    /// The event loop's heartbeat while auto-read is on. Inert otherwise,
    /// so a stray late tick can never move anything.
    AutoTick,
    /// Move the block cursor to the next/previous CODE block.
    CodeStep(i32),
    /// `X`: jump to the next GFM task item, wrapping. Count-multiplied.
    TaskStep(i32),
    /// Copy the focused code block to the clipboard.
    YankBlock,
    /// `o`: open the outline picker (or close it, when open).
    OutlineToggle,
    /// Move the outline selection through the FILTERED list. Saturates.
    OutlineMove(i32),
    /// A keystroke into the outline filter. Reuses [`SearchKey`] exactly as
    /// [`Action::HomeKey`] does; `Cancel` clears the filter first and closes
    /// only when it is already empty.
    OutlineKey(SearchKey),
    /// Enter: jump to the selected heading and push a history entry, so
    /// `Ctrl-O` returns — an outline jump is a link follow in spirit.
    OutlineJump,
    /// Jump to an outline entry by absolute row in the FILTERED list. See
    /// [`Action::BacklinksOpenAt`].
    OutlineJumpAt(u32),
    /// Jump to an absolute visual row from a pointer position.
    ScrollTo(u32),
    /// A click an overlay swallowed.
    ///
    /// A pane owns its rectangle the way it owns the keyboard: a click on a
    /// part of it that does nothing must still not reach the document behind
    /// it. Painting the pane registers its whole area as this, under its rows,
    /// so "inside the pane" needs no geometry the hit-test would have to
    /// re-derive.
    ///
    /// It never reaches `update`: the frontend drops it at the hit-test, and
    /// the state machine's catch-all would ignore it anyway.
    Absorb,
    Quit,

    // --- home screen ---
    /// Move the selection. Signed; saturates.
    HomeMove(i32),
    /// Put the selection on an absolute list index — what a mouse click
    /// produces. Clamped by the receiver, so a stale index is harmless.
    /// In search mode it indexes the hits rather than the files.
    HomeSelect(usize),
    HomeGo(Edge),
    /// Open the selected file in the reader.
    HomeOpen,
    /// A keystroke into the filter, or into the picker's typed path.
    /// Reuses [`SearchKey`] rather than adding a parallel enum.
    HomeKey(SearchKey),
    HomeNormalMode,
    HomeFilterMode,
    /// `/` on the home screen: content search across every scanned file.
    HomeSearchMode,
    PickerOpen,
    /// Put the picker's highlight on an absolute entry — a mouse click.
    /// An index into the picker's match list. Clamped by the receiver.
    PickerSelect(usize),
    PickerChoose,
    PickerCancel,
}

// --- pointer targets -------------------------------------------------------
//
// Discipline #4 again, from the other side: a click has to become an `Action`
// somewhere, and for chrome that "somewhere" cannot be a geometry function the
// hit-test re-derives. `paint_footer` decides where a hint lands by walking
// left to right through a four-stage elision ladder; inverting that walk would
// be a second copy of it, and the second copy is what drifts. (It already did
// once: `block_area` and `doc_span_at` disagreed by 13 columns at any terminal
// 95 cells wide or more, and no frame test saw it because they all run at 60.)
//
// So the painter records where it put things, and the event loop reads that
// back. The types are here, in the pure layer, for two reasons: rule 6 keeps
// `ratatui` out of the state layer, and a GTK frontend inherits the same
// target list without inheriting a `Rect`.
//
// The rule for which mechanism to use:
//
//   position depends only on (cols, rows, flags)  ->  invert a geometry fn
//   position depends on the data being painted    ->  record a target

/// A rectangle in terminal cells. Deliberately not `ratatui::Rect`: this type
/// crosses into the pure layer, and a GTK frontend has no cells.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Zone {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

impl Zone {
    #[must_use]
    pub const fn new(x: u16, y: u16, w: u16, h: u16) -> Self {
        Self { x, y, w, h }
    }

    /// Half-open on both axes, like `Rect` — the right and bottom edges are
    /// outside. (`ratatui` 0.30.2's own `Rect::contains` doc claims otherwise;
    /// the code is right and the comment was fixed after that release.)
    #[must_use]
    pub const fn contains(self, col: u16, row: u16) -> bool {
        col >= self.x && col < self.x + self.w && row >= self.y && row < self.y + self.h
    }
}

/// One thing on screen a pointer can act on.
///
/// `z` orders overlapping targets: an overlay pushes above the document it
/// covers, so a click inside a pane can never reach the text underneath.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub action: Action,
    pub zone: Zone,
    pub z: u8,
}

/// What a pointer landed on, with where inside it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub action: Action,
    pub zone: Zone,
    /// Zone-relative, so a target can act on which part of itself was hit.
    pub dx: u16,
    pub dy: u16,
}

/// The targets painted by one frame.
///
/// Cleared and refilled by every draw, exactly as the OSC 8 link list is, and
/// for the same reason: a target that was not painted this frame must not be
/// clickable. That is also what makes a click arriving before the first frame,
/// or after a resize, land on nothing instead of on a stale rectangle.
#[derive(Debug, Default)]
pub struct Targets(Vec<Target>);

impl Targets {
    #[must_use]
    pub const fn new() -> Self {
        Self(Vec::new())
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub fn push(&mut self, action: Action, zone: Zone, z: u8) {
        if zone.w > 0 && zone.h > 0 {
            self.0.push(Target { action, zone, z });
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[must_use]
    pub fn as_slice(&self) -> &[Target] {
        &self.0
    }

    /// The topmost target under `(col, row)`.
    ///
    /// Highest `z` wins; among equals the one pushed later wins, so a widget
    /// painted over another is hit first without either having to know about
    /// the other.
    #[must_use]
    pub fn hit(&self, col: u16, row: u16) -> Option<Hit> {
        self.0
            .iter()
            .filter(|t| t.zone.contains(col, row))
            .enumerate()
            .max_by_key(|(i, t)| (t.z, *i))
            .map(|(_, t)| Hit {
                action: t.action,
                zone: t.zone,
                dx: col - t.zone.x,
                dy: row - t.zone.y,
            })
    }

    /// Is any target at or above `z` under the pointer?
    ///
    /// The modal question: with a pane open, a click that hits none of its
    /// rows must still not reach the document behind it.
    #[must_use]
    pub fn covered_at(&self, col: u16, row: u16, z: u8) -> bool {
        self.0.iter().any(|t| t.z >= z && t.zone.contains(col, row))
    }
}

#[cfg(test)]
mod pointer_tests {
    use super::*;

    #[test]
    fn a_zone_is_half_open_on_both_axes() {
        let z = Zone::new(2, 3, 4, 2);
        assert!(z.contains(2, 3), "top-left corner is inside");
        assert!(z.contains(5, 4), "last cell is inside");
        assert!(!z.contains(6, 4), "the right edge is outside");
        assert!(!z.contains(5, 5), "the bottom edge is outside");
        assert!(!z.contains(1, 3));
    }

    #[test]
    fn an_empty_zone_is_never_pushed() {
        let mut t = Targets::new();
        t.push(Action::Quit, Zone::new(0, 0, 0, 1), 0);
        t.push(Action::Quit, Zone::new(0, 0, 1, 0), 0);
        assert!(
            t.is_empty(),
            "a zero-width or zero-height target cannot be hit"
        );
    }

    #[test]
    fn the_topmost_target_wins_and_later_breaks_a_tie() {
        let mut t = Targets::new();
        t.push(Action::Quit, Zone::new(0, 0, 10, 1), 0);
        t.push(Action::HelpToggle, Zone::new(0, 0, 10, 1), 0);
        assert_eq!(t.hit(1, 0).map(|h| h.action), Some(Action::HelpToggle));

        t.push(Action::ThemeCycle, Zone::new(0, 0, 10, 1), 9);
        assert_eq!(
            t.hit(1, 0).map(|h| h.action),
            Some(Action::ThemeCycle),
            "z outranks push order"
        );
    }

    #[test]
    fn a_hit_carries_zone_relative_coordinates() {
        let mut t = Targets::new();
        t.push(Action::Quit, Zone::new(4, 2, 6, 3), 0);
        let h = t.hit(7, 3).expect("inside");
        assert_eq!((h.dx, h.dy), (3, 1));
        assert!(t.hit(3, 3).is_none(), "outside is not a hit");
    }

    #[test]
    fn clearing_makes_last_frames_targets_unclickable() {
        let mut t = Targets::new();
        t.push(Action::Quit, Zone::new(0, 0, 4, 1), 0);
        t.clear();
        assert!(t.hit(1, 0).is_none());
    }

    #[test]
    fn covered_at_answers_the_modal_question() {
        let mut t = Targets::new();
        t.push(Action::Quit, Zone::new(0, 0, 20, 6), 5);
        assert!(t.covered_at(3, 3, 5), "inside the pane");
        assert!(!t.covered_at(3, 3, 6), "nothing is above it");
        assert!(!t.covered_at(30, 3, 5), "outside the pane");
    }
}
