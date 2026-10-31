//! The terminal's event vocabulary: every [`Intent`] a reader can ask for,
//! plus the TUI's own chrome.
//!
//! Discipline #4 — bind keys and pointer per-frontend, share the intents —
//! lives in `carrel_state::intent`, which says what an intent is and what
//! makes an action chrome instead: a terminal cell, a visual row or a row
//! count in its payload, or a surface only this frontend draws (its menus,
//! the scrim, the hint row, the key sheet, the settings rows, the palettes,
//! the measure in cells). [`Action`] is flat so the terminal can match on it
//! directly; [`Action::intent`] and `From<Intent>` convert, both exhaustive,
//! so an action added here does not compile until it is classified.
//!
//! Nothing here mentions a key. Where a variant does name the terminal, the
//! variant is chrome and [`Action::intent`] says so.

pub use carrel_state::intent::{Direction, Edge, Intent, NoteKey, SearchKey, Span, Where};

/// One thing the reader can be asked to do.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Action {
    HighlightAdd,
    HighlightAt(u32),
    NoteAt(u32),
    NoteEdit,
    NotesToggle,
    NoteMove(i32),
    NoteSelect(u32),
    NoteJump,
    NoteNext,
    NoteDelete,
    NotesExport,
    NoteInput(NoteKey),
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
    /// Open a document image; None chooses the next image from the reading position.
    ImageOpen(Option<carrel_core::BlockIdx>),
    /// Move through images while the full-screen viewer is open.
    ImageStep(i32),
    /// A click on a painted link: select it and follow it in one intent. The
    /// payload indexes `Document::links` — the same currency `LinkId` wraps,
    /// kept primitive here so this file stays free of state-layer types.
    LinkOpen(u32),
    /// `Ctrl-O`. Pop the history stack.
    Back,
    /// Return to where the last `Back` came from — the `›` on the status row.
    Forward,
    /// Go back to a document on the trail, by index into `App::history`: a
    /// click on its name in the status row. Clamped by the receiver.
    BackTo(u32),
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
    /// Pan a table in columns view. None chooses the first visible table.
    TableScroll {
        block: Option<carrel_core::BlockIdx>,
        delta: i32,
    },
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
    /// The context menu's "Go to the footnote text": go to the definition of
    /// the reference containing this doc byte — the one the pointer was on,
    /// not the one governing the top of the view, which is what `%` answers.
    /// Pushes a history entry, so `Back` returns.
    FootnoteJumpAt(u32),
    /// A click on a footnote mark: show its text where the mark is. `at` is
    /// the cell, `byte` a doc byte inside the `[^name]`.
    FootnotePeek {
        at: (u16, u16),
        byte: u32,
    },
    /// From an open peek, go to the footnote itself — `Enter`, or its button.
    PeekGo,
    /// The wheel over an open peek: scroll a footnote longer than its box
    /// by this many rows.
    PeekScroll(i32),
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
    /// Go to the directory one level above the home screen's root — the
    /// `↑` at the head of the path row.
    HomeUp,
    /// Go to one of the path row's segments, by index into
    /// [`crate::home::crumbs`]. Clamped by the receiver, so a segment from a
    /// frame that no longer describes the root is inert rather than wrong.
    ///
    /// Deliberately an index and not a path: `Action` is `Copy`, and every
    /// list in this enum is addressed the same way.
    HomeCrumb(usize),
    /// From the reader, back to the directory screen — the `⌂` on the status
    /// row.
    ///
    /// Not [`Action::CloseFile`]: `q` QUITS when the document was opened
    /// directly, because a pager should. This one always lands on the file
    /// list, rooted at the document's own directory when there is no home
    /// screen behind it to go back to.
    GoHome,
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
    /// `c`: jump to the next block that changed in the last reload, wrapping.
    /// Signed and count-multiplied, like the other steps.
    ChangeStep(i32),
    /// Open the document that was just written beside this one — the chip
    /// on the status row that says so.
    SiblingOpen,
    /// Jump to the next task that is still open, wrapping — the task count
    /// on the status row. `X` steps through every task; this one answers
    /// "what is left to do".
    TaskOpen,
    /// Copy the focused code block to the clipboard.
    YankBlock,
    /// Copy THIS code block — the chip under it, which already said which
    /// block it meant. Focuses it too, so `y` afterward means the same one.
    YankBlockAt(carrel_core::BlockIdx),
    /// Copy where a doc byte is, the way a tool says it: `PLAN.md:42`. For
    /// pasting into a prompt, so an agent is told exactly which line.
    CopyRef(u32),
    /// Copy a link to the section a doc byte is in: `PLAN.md#step-3`.
    CopySection(u32),
    /// Copy the selection as a markdown quote with where it came from.
    CopyQuote,
    /// Copy every note and highlight to the clipboard as markdown — the
    /// export, without a file in between.
    NotesCopy,
    /// `o`: open the outline picker (or close it, when open).
    OutlineToggle,
    /// Move the outline selection through the FILTERED list. Saturates.
    OutlineMove(i32),
    /// A keystroke into the outline filter. Reuses [`SearchKey`] exactly as
    /// [`Action::HomeKey`] does; `Cancel` clears the filter first and closes
    /// only when it is already empty.
    OutlineKey(SearchKey),
    /// A keystroke into the help sheet's filter. Same shape as
    /// [`Action::OutlineKey`]: printable keys narrow the sheet, `Cancel`
    /// clears the filter first and closes only when it is already empty.
    HelpKey(SearchKey),
    /// A click on a row of the help sheet: close the sheet and do what the
    /// row says, by the key it names. The payload indexes the help table.
    HelpRun(u32),
    /// Enter: jump to the selected heading and push a history entry, so
    /// `Ctrl-O` returns — an outline jump is a link follow in spirit.
    OutlineJump,
    /// Jump to an outline entry by absolute row in the FILTERED list. See
    /// [`Action::BacklinksOpenAt`].
    OutlineJumpAt(u32),
    /// Jump to a breadcrumb segment's heading. Carries the heading's `NodeId`
    /// — stable for the parse the click was painted from, and validated on
    /// arrival, so a reload between paint and press lands on nothing rather
    /// than on the wrong section.
    CrumbJump(carrel_core::NodeId),
    /// Jump to an absolute visual row from a pointer position.
    ScrollTo(u32),
    /// The pointer moved to this cell.
    ///
    /// **Never load-bearing.** It decorates what is under the pointer and
    /// decides nothing: every click resolves from its own coordinates, so a
    /// terminal that does not report motion loses the decoration and nothing
    /// else. (ratatui's own `custom-widget` example gets this wrong — its
    /// press reads no coordinates and acts on whatever the last motion
    /// selected, so in such a terminal it activates the wrong button.)
    ///
    /// The CELL is stored rather than what was under it, so a scroll under a
    /// still pointer moves the highlight to whatever is there now — the
    /// painter re-asks its own registry every frame.
    Hover((u16, u16)),
    /// Open a menu anchored at `(col, row)`.
    ///
    /// `byte` is the doc byte under the pointer: `Some` opens the context
    /// menu built for it, `None` the global one. The frontend decides which
    /// by asking whether the pointer was on text at all — so a right-click
    /// in the chrome, in the margin, or below the last line all reach the
    /// global menu, and everything the document paints reaches the other.
    MenuOpen {
        at: (u16, u16),
        byte: Option<u32>,
    },
    /// Move the menu's selection. Clamps at both ends — a menu is short
    /// enough that wrapping only ever surprises.
    MenuMove(i32),
    /// Put the selection on an absolute item index: what the pointer does by
    /// hovering. An index that is out of range, or on a gap or a greyed row,
    /// clears the selection rather than clamping onto a neighbour — the
    /// pointer is over nothing, and saying so is the honest answer.
    MenuHover(u32),
    /// Act on the selected item, and close.
    MenuChoose,
    /// Act on an item by absolute index — a click, which already said which
    /// row it meant. See [`Action::BacklinksOpenAt`].
    MenuPick(u32),
    /// Close without acting.
    MenuClose,
    /// Copy the selected link's destination.
    ///
    /// [`Action::LinkFollow`] already copies an EXTERNAL destination, because
    /// there is nothing else a reader may do with one. This is the other
    /// half: copy a local one instead of opening it.
    LinkCopy,
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
    /// A click OUTSIDE the open pane: close it, and do nothing else.
    ///
    /// The painter lays this over the whole frame, under the pane and
    /// above everything else it covers — the document, the status row's
    /// chips, the trail, the breadcrumb, the scrollbar — so a click beside
    /// a pane is used up closing it, the way a click beside a menu always
    /// was. It is not a button: hover never lights it.
    PaneClose,
    Quit,

    // --- home screen ---
    /// Move the selection. Signed; saturates.
    HomeMove(i32),
    /// One screenful up or down the home list. The page is the list's own
    /// height, derived in `update` from `home::list_geometry` — keys.rs has
    /// no viewport, so it cannot compute one and must not guess a constant.
    HomePage(i32),
    /// Put the selection on an absolute list index — what a mouse click
    /// produces. Clamped by the receiver, so a stale index is harmless.
    /// In search mode it indexes the hits rather than the files.
    HomeSelect(usize),
    HomeGo(Edge),
    /// Widen or narrow the reading measure by a step. The closest honest
    /// equivalent to a zoom in a terminal, where the font is the emulator's
    /// business and never ours.
    MeasureStep(i32),
    /// The settings pane: every persisted preference, visible and clickable.
    /// Carrel had nine config keys and named the file in no menu, help row
    /// or `--help` line — only in the README.
    SettingsToggle,
    /// Open the built-in first document, from the empty-folder dead end.
    WelcomeOpen,
    SettingsMove(i32),
    /// Change the highlighted setting. Booleans toggle whichever way this
    /// is nudged; the text width steps by the sign.
    SettingsAdjust(i32),
    /// Click row `n`: select it and change it, in one gesture.
    SettingsPickAt(u32),
    /// Open the selected file in the reader.
    HomeOpen,
    /// A keystroke into the filter, or into the picker's typed path.
    /// Reuses [`SearchKey`] rather than adding a parallel enum.
    HomeKey(SearchKey),
    HomeNormalMode,
    HomeFilterMode,
    /// `/` on the home screen: content search across every scanned file.
    HomeSearchMode,
    /// `Tab` in content search: read the hits as a document — a section per
    /// file with a link per match — instead of opening one file.
    HomeOpenResults,
    /// `#`, on the home screen or in the reader: read every document's
    /// frontmatter tags and open them as a document — a section per tag, a
    /// link per document.
    /// This only ASKS: the reading happens on a thread the event loop owns,
    /// and [`Action::HomeOpenTags`] arrives when it is done.
    HomeTags,
    /// A click on a tag on a document's metadata card: the tags page, open
    /// at that tag. The payload is a doc byte inside the tag as written.
    TagOpen(u32),
    /// The tag scan finished; open what it found. Sent by the event loop,
    /// never bound to a key or a button.
    HomeOpenTags,
    /// `s` on the file list: step to the next order — newest, by name,
    /// recently read. Persisted.
    HomeSort,
    PickerOpen,
    /// Put the picker's highlight on an absolute entry — a mouse click.
    /// An index into the picker's match list. Clamped by the receiver.
    PickerSelect(usize),
    PickerChoose,
    PickerCancel,
    /// Drill into the highlighted directory without choosing it: the dialog
    /// stays up and lists what is inside. `Tab` / `→`, and the footer's
    /// drill-in button.
    PickerDescend,
    /// Climb to the parent of the browsed directory. `←`, and `Backspace`
    /// on an empty filter.
    PickerUp,
}

impl From<Intent> for Action {
    #[allow(clippy::too_many_lines)] // one arm per intent; that IS the guarantee
    fn from(intent: Intent) -> Self {
        match intent {
            Intent::Scroll(v0, v1) => Self::Scroll(v0, v1),
            Intent::GoToStart => Self::GoToStart,
            Intent::GoToEnd => Self::GoToEnd,
            Intent::BlockStep(v0) => Self::BlockStep(v0),
            Intent::Recenter(v0) => Self::Recenter(v0),
            Intent::AutoToggle => Self::AutoToggle,
            Intent::FollowToggle => Self::FollowToggle,
            Intent::FocusToggle => Self::FocusToggle,
            Intent::InfoToggle => Self::InfoToggle,
            Intent::TableToggle => Self::TableToggle,
            Intent::TableScroll { block, delta } => Self::TableScroll { block, delta },
            Intent::RenderedToggle => Self::RenderedToggle,
            Intent::HelpToggle => Self::HelpToggle,
            Intent::SettingsToggle => Self::SettingsToggle,
            Intent::SearchOpen(v0) => Self::SearchOpen(v0),
            Intent::SearchKey(v0) => Self::SearchKey(v0),
            Intent::MatchStep(v0) => Self::MatchStep(v0),
            Intent::FoldToggle => Self::FoldToggle,
            Intent::FoldAll => Self::FoldAll,
            Intent::UnfoldAll => Self::UnfoldAll,
            Intent::FoldAt(v0) => Self::FoldAt(v0),
            Intent::OutlineToggle => Self::OutlineToggle,
            Intent::OutlineMove(v0) => Self::OutlineMove(v0),
            Intent::OutlineKey(v0) => Self::OutlineKey(v0),
            Intent::OutlineJump => Self::OutlineJump,
            Intent::OutlineJumpAt(v0) => Self::OutlineJumpAt(v0),
            Intent::OutlineJumpTo(v0) => Self::OutlineJumpTo(v0),
            Intent::CrumbJump(v0) => Self::CrumbJump(v0),
            Intent::LinkStep(v0) => Self::LinkStep(v0),
            Intent::LinkFollow => Self::LinkFollow,
            Intent::LinkOpen(v0) => Self::LinkOpen(v0),
            Intent::LinkCopy => Self::LinkCopy,
            Intent::Back => Self::Back,
            Intent::Forward => Self::Forward,
            Intent::BackTo(v0) => Self::BackTo(v0),
            Intent::BacklinksToggle => Self::BacklinksToggle,
            Intent::BacklinksMove(v0) => Self::BacklinksMove(v0),
            Intent::BacklinksOpen => Self::BacklinksOpen,
            Intent::BacklinksOpenAt(v0) => Self::BacklinksOpenAt(v0),
            Intent::ForwardToggle => Self::ForwardToggle,
            Intent::ForwardMove(v0) => Self::ForwardMove(v0),
            Intent::ForwardOpen => Self::ForwardOpen,
            Intent::ForwardOpenAt(v0) => Self::ForwardOpenAt(v0),
            Intent::FootnoteJump => Self::FootnoteJump,
            Intent::FootnoteJumpAt(v0) => Self::FootnoteJumpAt(v0),
            Intent::SiblingOpen => Self::SiblingOpen,
            Intent::ImageOpen(v0) => Self::ImageOpen(v0),
            Intent::ImageStep(v0) => Self::ImageStep(v0),
            Intent::SelectAnchor(v0) => Self::SelectAnchor(v0),
            Intent::SelectDrag(v0) => Self::SelectDrag(v0),
            Intent::SelectRelease => Self::SelectRelease,
            Intent::SelectWord(v0) => Self::SelectWord(v0),
            Intent::SelectBlock(v0) => Self::SelectBlock(v0),
            Intent::YankBlock => Self::YankBlock,
            Intent::YankBlockAt(v0) => Self::YankBlockAt(v0),
            Intent::CodeStep(v0) => Self::CodeStep(v0),
            Intent::CopyRef(v0) => Self::CopyRef(v0),
            Intent::CopySection(v0) => Self::CopySection(v0),
            Intent::CopyQuote => Self::CopyQuote,
            Intent::TaskStep(v0) => Self::TaskStep(v0),
            Intent::TaskOpen => Self::TaskOpen,
            Intent::ChangeStep(v0) => Self::ChangeStep(v0),
            Intent::MarkToggle => Self::MarkToggle,
            Intent::MarkNext => Self::MarkNext,
            Intent::MarkListToggle => Self::MarkListToggle,
            Intent::MarkListMove(v0) => Self::MarkListMove(v0),
            Intent::MarkListJump => Self::MarkListJump,
            Intent::MarkListJumpAt(v0) => Self::MarkListJumpAt(v0),
            Intent::HighlightAdd => Self::HighlightAdd,
            Intent::HighlightAt(v0) => Self::HighlightAt(v0),
            Intent::NoteAt(v0) => Self::NoteAt(v0),
            Intent::NoteEdit => Self::NoteEdit,
            Intent::NotesToggle => Self::NotesToggle,
            Intent::NoteMove(v0) => Self::NoteMove(v0),
            Intent::NoteSelect(v0) => Self::NoteSelect(v0),
            Intent::NoteJump => Self::NoteJump,
            Intent::NoteNext => Self::NoteNext,
            Intent::NoteDelete => Self::NoteDelete,
            Intent::NotesExport => Self::NotesExport,
            Intent::NotesCopy => Self::NotesCopy,
            Intent::NoteInput(v0) => Self::NoteInput(v0),
            Intent::Dismiss => Self::Dismiss,
            Intent::CloseFile => Self::CloseFile,
            Intent::GoHome => Self::GoHome,
            Intent::Quit => Self::Quit,
            Intent::HomeMove(v0) => Self::HomeMove(v0),
            Intent::HomePage(v0) => Self::HomePage(v0),
            Intent::HomeSelect(v0) => Self::HomeSelect(v0),
            Intent::HomeGo(v0) => Self::HomeGo(v0),
            Intent::HomeOpen => Self::HomeOpen,
            Intent::HomeKey(v0) => Self::HomeKey(v0),
            Intent::HomeNormalMode => Self::HomeNormalMode,
            Intent::HomeFilterMode => Self::HomeFilterMode,
            Intent::HomeSearchMode => Self::HomeSearchMode,
            Intent::HomeOpenResults => Self::HomeOpenResults,
            Intent::HomeTags => Self::HomeTags,
            Intent::HomeOpenTags => Self::HomeOpenTags,
            Intent::TagOpen(v0) => Self::TagOpen(v0),
            Intent::HomeSort => Self::HomeSort,
            Intent::HomeUp => Self::HomeUp,
            Intent::HomeCrumb(v0) => Self::HomeCrumb(v0),
            Intent::HomeResume(v0) => Self::HomeResume(v0),
            Intent::WelcomeOpen => Self::WelcomeOpen,
            Intent::PickerOpen => Self::PickerOpen,
            Intent::PickerSelect(v0) => Self::PickerSelect(v0),
            Intent::PickerChoose => Self::PickerChoose,
            Intent::PickerCancel => Self::PickerCancel,
            Intent::PickerDescend => Self::PickerDescend,
            Intent::PickerUp => Self::PickerUp,
        }
    }
}

impl Action {
    /// The frontend-neutral intent this action is, or `None` for TUI chrome.
    ///
    /// Exhaustive on purpose, with no catch-all: the `None` arm IS the list
    /// of what a second frontend does not inherit, kept by the compiler
    /// rather than by a document that drifts (`carrel_state::intent`).
    #[allow(clippy::too_many_lines)] // one arm per intent; that IS the guarantee
    #[must_use]
    pub fn intent(self) -> Option<Intent> {
        match self {
            Self::Scroll(v0, v1) => Some(Intent::Scroll(v0, v1)),
            Self::GoToStart => Some(Intent::GoToStart),
            Self::GoToEnd => Some(Intent::GoToEnd),
            Self::BlockStep(v0) => Some(Intent::BlockStep(v0)),
            Self::Recenter(v0) => Some(Intent::Recenter(v0)),
            Self::AutoToggle => Some(Intent::AutoToggle),
            Self::FollowToggle => Some(Intent::FollowToggle),
            Self::FocusToggle => Some(Intent::FocusToggle),
            Self::InfoToggle => Some(Intent::InfoToggle),
            Self::TableToggle => Some(Intent::TableToggle),
            Self::TableScroll { block, delta } => Some(Intent::TableScroll { block, delta }),
            Self::RenderedToggle => Some(Intent::RenderedToggle),
            Self::HelpToggle => Some(Intent::HelpToggle),
            Self::SettingsToggle => Some(Intent::SettingsToggle),
            Self::SearchOpen(v0) => Some(Intent::SearchOpen(v0)),
            Self::SearchKey(v0) => Some(Intent::SearchKey(v0)),
            Self::MatchStep(v0) => Some(Intent::MatchStep(v0)),
            Self::FoldToggle => Some(Intent::FoldToggle),
            Self::FoldAll => Some(Intent::FoldAll),
            Self::UnfoldAll => Some(Intent::UnfoldAll),
            Self::FoldAt(v0) => Some(Intent::FoldAt(v0)),
            Self::OutlineToggle => Some(Intent::OutlineToggle),
            Self::OutlineMove(v0) => Some(Intent::OutlineMove(v0)),
            Self::OutlineKey(v0) => Some(Intent::OutlineKey(v0)),
            Self::OutlineJump => Some(Intent::OutlineJump),
            Self::OutlineJumpAt(v0) => Some(Intent::OutlineJumpAt(v0)),
            Self::OutlineJumpTo(v0) => Some(Intent::OutlineJumpTo(v0)),
            Self::CrumbJump(v0) => Some(Intent::CrumbJump(v0)),
            Self::LinkStep(v0) => Some(Intent::LinkStep(v0)),
            Self::LinkFollow => Some(Intent::LinkFollow),
            Self::LinkOpen(v0) => Some(Intent::LinkOpen(v0)),
            Self::LinkCopy => Some(Intent::LinkCopy),
            Self::Back => Some(Intent::Back),
            Self::Forward => Some(Intent::Forward),
            Self::BackTo(v0) => Some(Intent::BackTo(v0)),
            Self::BacklinksToggle => Some(Intent::BacklinksToggle),
            Self::BacklinksMove(v0) => Some(Intent::BacklinksMove(v0)),
            Self::BacklinksOpen => Some(Intent::BacklinksOpen),
            Self::BacklinksOpenAt(v0) => Some(Intent::BacklinksOpenAt(v0)),
            Self::ForwardToggle => Some(Intent::ForwardToggle),
            Self::ForwardMove(v0) => Some(Intent::ForwardMove(v0)),
            Self::ForwardOpen => Some(Intent::ForwardOpen),
            Self::ForwardOpenAt(v0) => Some(Intent::ForwardOpenAt(v0)),
            Self::FootnoteJump => Some(Intent::FootnoteJump),
            Self::FootnoteJumpAt(v0) => Some(Intent::FootnoteJumpAt(v0)),
            Self::SiblingOpen => Some(Intent::SiblingOpen),
            Self::ImageOpen(v0) => Some(Intent::ImageOpen(v0)),
            Self::ImageStep(v0) => Some(Intent::ImageStep(v0)),
            Self::SelectAnchor(v0) => Some(Intent::SelectAnchor(v0)),
            Self::SelectDrag(v0) => Some(Intent::SelectDrag(v0)),
            Self::SelectRelease => Some(Intent::SelectRelease),
            Self::SelectWord(v0) => Some(Intent::SelectWord(v0)),
            Self::SelectBlock(v0) => Some(Intent::SelectBlock(v0)),
            Self::YankBlock => Some(Intent::YankBlock),
            Self::YankBlockAt(v0) => Some(Intent::YankBlockAt(v0)),
            Self::CodeStep(v0) => Some(Intent::CodeStep(v0)),
            Self::CopyRef(v0) => Some(Intent::CopyRef(v0)),
            Self::CopySection(v0) => Some(Intent::CopySection(v0)),
            Self::CopyQuote => Some(Intent::CopyQuote),
            Self::TaskStep(v0) => Some(Intent::TaskStep(v0)),
            Self::TaskOpen => Some(Intent::TaskOpen),
            Self::ChangeStep(v0) => Some(Intent::ChangeStep(v0)),
            Self::MarkToggle => Some(Intent::MarkToggle),
            Self::MarkNext => Some(Intent::MarkNext),
            Self::MarkListToggle => Some(Intent::MarkListToggle),
            Self::MarkListMove(v0) => Some(Intent::MarkListMove(v0)),
            Self::MarkListJump => Some(Intent::MarkListJump),
            Self::MarkListJumpAt(v0) => Some(Intent::MarkListJumpAt(v0)),
            Self::HighlightAdd => Some(Intent::HighlightAdd),
            Self::HighlightAt(v0) => Some(Intent::HighlightAt(v0)),
            Self::NoteAt(v0) => Some(Intent::NoteAt(v0)),
            Self::NoteEdit => Some(Intent::NoteEdit),
            Self::NotesToggle => Some(Intent::NotesToggle),
            Self::NoteMove(v0) => Some(Intent::NoteMove(v0)),
            Self::NoteSelect(v0) => Some(Intent::NoteSelect(v0)),
            Self::NoteJump => Some(Intent::NoteJump),
            Self::NoteNext => Some(Intent::NoteNext),
            Self::NoteDelete => Some(Intent::NoteDelete),
            Self::NotesExport => Some(Intent::NotesExport),
            Self::NotesCopy => Some(Intent::NotesCopy),
            Self::NoteInput(v0) => Some(Intent::NoteInput(v0)),
            Self::Dismiss => Some(Intent::Dismiss),
            Self::CloseFile => Some(Intent::CloseFile),
            Self::GoHome => Some(Intent::GoHome),
            Self::Quit => Some(Intent::Quit),
            Self::HomeMove(v0) => Some(Intent::HomeMove(v0)),
            Self::HomePage(v0) => Some(Intent::HomePage(v0)),
            Self::HomeSelect(v0) => Some(Intent::HomeSelect(v0)),
            Self::HomeGo(v0) => Some(Intent::HomeGo(v0)),
            Self::HomeOpen => Some(Intent::HomeOpen),
            Self::HomeKey(v0) => Some(Intent::HomeKey(v0)),
            Self::HomeNormalMode => Some(Intent::HomeNormalMode),
            Self::HomeFilterMode => Some(Intent::HomeFilterMode),
            Self::HomeSearchMode => Some(Intent::HomeSearchMode),
            Self::HomeOpenResults => Some(Intent::HomeOpenResults),
            Self::HomeTags => Some(Intent::HomeTags),
            Self::HomeOpenTags => Some(Intent::HomeOpenTags),
            Self::TagOpen(v0) => Some(Intent::TagOpen(v0)),
            Self::HomeSort => Some(Intent::HomeSort),
            Self::HomeUp => Some(Intent::HomeUp),
            Self::HomeCrumb(v0) => Some(Intent::HomeCrumb(v0)),
            Self::HomeResume(v0) => Some(Intent::HomeResume(v0)),
            Self::WelcomeOpen => Some(Intent::WelcomeOpen),
            Self::PickerOpen => Some(Intent::PickerOpen),
            Self::PickerSelect(v0) => Some(Intent::PickerSelect(v0)),
            Self::PickerChoose => Some(Intent::PickerChoose),
            Self::PickerCancel => Some(Intent::PickerCancel),
            Self::PickerDescend => Some(Intent::PickerDescend),
            Self::PickerUp => Some(Intent::PickerUp),
            Self::GoToRow(..)
            | Self::ThemeCycle
            | Self::HintsToggle
            | Self::BreadcrumbToggle
            | Self::FootnotePeek { .. }
            | Self::PeekGo
            | Self::PeekScroll(..)
            | Self::AutoTick
            | Self::HelpKey(..)
            | Self::HelpRun(..)
            | Self::ScrollTo(..)
            | Self::Hover(..)
            | Self::MenuOpen { .. }
            | Self::MenuMove(..)
            | Self::MenuHover(..)
            | Self::MenuChoose
            | Self::MenuPick(..)
            | Self::MenuClose
            | Self::Absorb
            | Self::PaneClose
            | Self::MeasureStep(..)
            | Self::SettingsMove(..)
            | Self::SettingsAdjust(..)
            | Self::SettingsPickAt(..) => None,
        }
    }
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

impl Action {
    /// A target that is a surface, not a button: a pane's own blank
    /// ([`Action::Absorb`]) or the scrim beside it ([`Action::PaneClose`]).
    /// Hover never lights one — lighting the scrim would light the whole
    /// frame — and nothing beneath one lights either, since a click there
    /// no longer reaches it.
    #[must_use]
    pub const fn is_backdrop(self) -> bool {
        matches!(self, Self::Absorb | Self::PaneClose)
    }
}

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

    /// The zone a hover should light: the topmost target under the pointer,
    /// if it actually does something.
    ///
    /// A pane registers its entire rectangle as [`Action::Absorb`], so that
    /// is what is topmost over a blank part of it — and the answer there is
    /// *nothing*. This used to skip `Absorb` and take the next target down,
    /// which was whatever button lay UNDER the pane: moving the pointer over
    /// an open pane lit links and chips through it, as if they were live.
    #[must_use]
    pub fn hoverable(&self, col: u16, row: u16) -> Option<Zone> {
        self.hit(col, row)
            .filter(|hit| !hit.action.is_backdrop())
            .map(|hit| hit.zone)
    }

    /// Is this cell's topmost target the one occupying `zone`? A button
    /// partly under a pane is lit only where it shows.
    #[must_use]
    pub fn shows(&self, zone: Zone, col: u16, row: u16) -> bool {
        self.hit(col, row)
            .is_some_and(|hit| hit.zone == zone && !hit.action.is_backdrop())
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
    fn a_hover_never_lights_a_panes_whole_rectangle() {
        let mut t = Targets::new();
        // A pane: its own area, then one row inside it.
        t.push(Action::Absorb, Zone::new(0, 0, 20, 6), 2);
        t.push(Action::OutlineJumpAt(0), Zone::new(1, 1, 18, 1), 2);
        assert_eq!(t.hoverable(5, 1), Some(Zone::new(1, 1, 18, 1)), "the row");
        assert_eq!(
            t.hoverable(5, 4),
            None,
            "a blank part of the pane lights nothing — `hit` would return the              pane's whole rectangle here"
        );
        assert_eq!(t.hit(5, 4).map(|h| h.action), Some(Action::Absorb));
    }

    /// The other half: a button UNDER the pane. Skipping `Absorb` found it,
    /// and moving the pointer across an open pane lit the links and chips
    /// behind it.
    #[test]
    fn a_hover_never_lights_what_is_under_a_pane() {
        let mut t = Targets::new();
        t.push(Action::LinkOpen(0), Zone::new(2, 3, 10, 1), 0);
        t.push(Action::Absorb, Zone::new(6, 0, 14, 6), 2);
        assert_eq!(t.hoverable(8, 3), None, "the pane is what is there");
        assert_eq!(t.hoverable(3, 3), Some(Zone::new(2, 3, 10, 1)));
        // And the link is lit only where it shows.
        let link = Zone::new(2, 3, 10, 1);
        assert!(t.shows(link, 5, 3));
        assert!(!t.shows(link, 6, 3), "this cell is the pane's");
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

#[cfg(test)]
mod intent_tests {
    use super::*;
    use carrel_core::{BlockIdx, NodeId};

    /// A distinct number for every intent. Exhaustive, so an intent added
    /// to the enum does not compile here until it has one — and then
    /// [`every_intent_round_trips_through_action`] fails until
    /// [`one_of_each`] carries it too.
    #[allow(clippy::too_many_lines)] // one arm per intent; that IS the guarantee
    fn witness(i: Intent) -> usize {
        match i {
            Intent::Scroll(..) => 0,
            Intent::GoToStart => 1,
            Intent::GoToEnd => 2,
            Intent::BlockStep(..) => 3,
            Intent::Recenter(..) => 4,
            Intent::AutoToggle => 5,
            Intent::FollowToggle => 6,
            Intent::FocusToggle => 7,
            Intent::InfoToggle => 8,
            Intent::TableToggle => 9,
            Intent::TableScroll { .. } => 10,
            Intent::RenderedToggle => 11,
            Intent::HelpToggle => 12,
            Intent::SettingsToggle => 13,
            Intent::SearchOpen(..) => 14,
            Intent::SearchKey(..) => 15,
            Intent::MatchStep(..) => 16,
            Intent::FoldToggle => 17,
            Intent::FoldAll => 18,
            Intent::UnfoldAll => 19,
            Intent::FoldAt(..) => 20,
            Intent::OutlineToggle => 21,
            Intent::OutlineMove(..) => 22,
            Intent::OutlineKey(..) => 23,
            Intent::OutlineJump => 24,
            Intent::OutlineJumpAt(..) => 25,
            Intent::OutlineJumpTo(..) => 26,
            Intent::CrumbJump(..) => 27,
            Intent::LinkStep(..) => 28,
            Intent::LinkFollow => 29,
            Intent::LinkOpen(..) => 30,
            Intent::LinkCopy => 31,
            Intent::Back => 32,
            Intent::Forward => 33,
            Intent::BackTo(..) => 34,
            Intent::BacklinksToggle => 35,
            Intent::BacklinksMove(..) => 36,
            Intent::BacklinksOpen => 37,
            Intent::BacklinksOpenAt(..) => 38,
            Intent::ForwardToggle => 39,
            Intent::ForwardMove(..) => 40,
            Intent::ForwardOpen => 41,
            Intent::ForwardOpenAt(..) => 42,
            Intent::FootnoteJump => 43,
            Intent::FootnoteJumpAt(..) => 44,
            Intent::SiblingOpen => 45,
            Intent::ImageOpen(..) => 46,
            Intent::ImageStep(..) => 47,
            Intent::SelectAnchor(..) => 48,
            Intent::SelectDrag(..) => 49,
            Intent::SelectRelease => 50,
            Intent::SelectWord(..) => 51,
            Intent::SelectBlock(..) => 52,
            Intent::YankBlock => 53,
            Intent::YankBlockAt(..) => 54,
            Intent::CodeStep(..) => 55,
            Intent::CopyRef(..) => 56,
            Intent::CopySection(..) => 57,
            Intent::CopyQuote => 58,
            Intent::TaskStep(..) => 59,
            Intent::TaskOpen => 60,
            Intent::ChangeStep(..) => 61,
            Intent::MarkToggle => 62,
            Intent::MarkNext => 63,
            Intent::MarkListToggle => 64,
            Intent::MarkListMove(..) => 65,
            Intent::MarkListJump => 66,
            Intent::MarkListJumpAt(..) => 67,
            Intent::HighlightAdd => 68,
            Intent::HighlightAt(..) => 69,
            Intent::NoteAt(..) => 70,
            Intent::NoteEdit => 71,
            Intent::NotesToggle => 72,
            Intent::NoteMove(..) => 73,
            Intent::NoteSelect(..) => 74,
            Intent::NoteJump => 75,
            Intent::NoteNext => 76,
            Intent::NoteDelete => 77,
            Intent::NotesExport => 78,
            Intent::NotesCopy => 79,
            Intent::NoteInput(..) => 80,
            Intent::Dismiss => 81,
            Intent::CloseFile => 82,
            Intent::GoHome => 83,
            Intent::Quit => 84,
            Intent::HomeMove(..) => 85,
            Intent::HomePage(..) => 86,
            Intent::HomeSelect(..) => 87,
            Intent::HomeGo(..) => 88,
            Intent::HomeOpen => 89,
            Intent::HomeKey(..) => 90,
            Intent::HomeNormalMode => 91,
            Intent::HomeFilterMode => 92,
            Intent::HomeSearchMode => 93,
            Intent::HomeOpenResults => 94,
            Intent::HomeTags => 95,
            Intent::HomeOpenTags => 96,
            Intent::TagOpen(..) => 97,
            Intent::HomeSort => 98,
            Intent::HomeUp => 99,
            Intent::HomeCrumb(..) => 100,
            Intent::HomeResume(..) => 101,
            Intent::WelcomeOpen => 102,
            Intent::PickerOpen => 103,
            Intent::PickerSelect(..) => 104,
            Intent::PickerChoose => 105,
            Intent::PickerCancel => 106,
            Intent::PickerDescend => 107,
            Intent::PickerUp => 108,
        }
    }

    #[allow(clippy::too_many_lines)] // one arm per intent; that IS the guarantee
    fn one_of_each() -> Vec<Intent> {
        vec![
            Intent::Scroll(Span::Page, -2),
            Intent::GoToStart,
            Intent::GoToEnd,
            Intent::BlockStep(-2),
            Intent::Recenter(Where::Bottom),
            Intent::AutoToggle,
            Intent::FollowToggle,
            Intent::FocusToggle,
            Intent::InfoToggle,
            Intent::TableToggle,
            Intent::TableScroll {
                block: None,
                delta: -1,
            },
            Intent::RenderedToggle,
            Intent::HelpToggle,
            Intent::SettingsToggle,
            Intent::SearchOpen(Direction::Backward),
            Intent::SearchKey(SearchKey::Char('x')),
            Intent::MatchStep(-2),
            Intent::FoldToggle,
            Intent::FoldAll,
            Intent::UnfoldAll,
            Intent::FoldAt(7),
            Intent::OutlineToggle,
            Intent::OutlineMove(-2),
            Intent::OutlineKey(SearchKey::Char('x')),
            Intent::OutlineJump,
            Intent::OutlineJumpAt(7),
            Intent::OutlineJumpTo(BlockIdx(4)),
            Intent::CrumbJump(NodeId(5)),
            Intent::LinkStep(-2),
            Intent::LinkFollow,
            Intent::LinkOpen(7),
            Intent::LinkCopy,
            Intent::Back,
            Intent::Forward,
            Intent::BackTo(7),
            Intent::BacklinksToggle,
            Intent::BacklinksMove(-2),
            Intent::BacklinksOpen,
            Intent::BacklinksOpenAt(7),
            Intent::ForwardToggle,
            Intent::ForwardMove(-2),
            Intent::ForwardOpen,
            Intent::ForwardOpenAt(7),
            Intent::FootnoteJump,
            Intent::FootnoteJumpAt(7),
            Intent::SiblingOpen,
            Intent::ImageOpen(Some(BlockIdx(4))),
            Intent::ImageStep(-2),
            Intent::SelectAnchor((3, 9)),
            Intent::SelectDrag((3, 9)),
            Intent::SelectRelease,
            Intent::SelectWord(7),
            Intent::SelectBlock(7),
            Intent::YankBlock,
            Intent::YankBlockAt(BlockIdx(4)),
            Intent::CodeStep(-2),
            Intent::CopyRef(7),
            Intent::CopySection(7),
            Intent::CopyQuote,
            Intent::TaskStep(-2),
            Intent::TaskOpen,
            Intent::ChangeStep(-2),
            Intent::MarkToggle,
            Intent::MarkNext,
            Intent::MarkListToggle,
            Intent::MarkListMove(-2),
            Intent::MarkListJump,
            Intent::MarkListJumpAt(7),
            Intent::HighlightAdd,
            Intent::HighlightAt(7),
            Intent::NoteAt(7),
            Intent::NoteEdit,
            Intent::NotesToggle,
            Intent::NoteMove(-2),
            Intent::NoteSelect(7),
            Intent::NoteJump,
            Intent::NoteNext,
            Intent::NoteDelete,
            Intent::NotesExport,
            Intent::NotesCopy,
            Intent::NoteInput(NoteKey::Save),
            Intent::Dismiss,
            Intent::CloseFile,
            Intent::GoHome,
            Intent::Quit,
            Intent::HomeMove(-2),
            Intent::HomePage(-2),
            Intent::HomeSelect(3),
            Intent::HomeGo(Edge::Last),
            Intent::HomeOpen,
            Intent::HomeKey(SearchKey::Char('x')),
            Intent::HomeNormalMode,
            Intent::HomeFilterMode,
            Intent::HomeSearchMode,
            Intent::HomeOpenResults,
            Intent::HomeTags,
            Intent::HomeOpenTags,
            Intent::TagOpen(7),
            Intent::HomeSort,
            Intent::HomeUp,
            Intent::HomeCrumb(3),
            Intent::HomeResume(3),
            Intent::WelcomeOpen,
            Intent::PickerOpen,
            Intent::PickerSelect(3),
            Intent::PickerChoose,
            Intent::PickerCancel,
            Intent::PickerDescend,
            Intent::PickerUp,
        ]
    }

    #[test]
    fn every_intent_round_trips_through_action() {
        let all = one_of_each();
        let mut seen: Vec<usize> = all.iter().map(|&i| witness(i)).collect();
        seen.sort_unstable();
        assert_eq!(
            seen,
            (0..seen.len()).collect::<Vec<_>>(),
            "one of each, no repeats"
        );
        assert_eq!(
            witness(*all.last().unwrap()),
            all.len() - 1,
            "the list is complete"
        );
        for i in all {
            assert_eq!(Action::from(i).intent(), Some(i), "{i:?}");
        }
    }

    /// The chrome half: nothing that carries a cell or a row, or drives a
    /// surface only the terminal draws, passes for an intent.
    #[test]
    fn terminal_chrome_is_no_intent() {
        for a in [
            Action::Hover((1, 2)),
            Action::MenuOpen {
                at: (1, 2),
                byte: Some(3),
            },
            Action::FootnotePeek {
                at: (1, 2),
                byte: 3,
            },
            Action::GoToRow(9),
            Action::ScrollTo(9),
            Action::PeekScroll(1),
            Action::AutoTick,
            Action::Absorb,
            Action::PaneClose,
            Action::MenuPick(0),
            Action::ThemeCycle,
            Action::MeasureStep(1),
            Action::HintsToggle,
            Action::BreadcrumbToggle,
            Action::SettingsAdjust(1),
            Action::HelpRun(0),
        ] {
            assert_eq!(a.intent(), None, "{a:?}");
        }
    }
}
