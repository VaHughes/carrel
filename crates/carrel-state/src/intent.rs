//! What a reader can ask for — the vocabulary every frontend shares.
//!
//! Discipline 4 was "share an `Action` enum". The terminal's `Action` grew
//! into the terminal's event vocabulary: pointer cells, visual rows, the
//! z-order of panes it draws, a menu it draws itself. A GTK frontend gets
//! menus, popovers, hover and hit-testing from the toolkit and would use
//! none of those (research.md Q29). So the shared vocabulary is narrower:
//! **an intent is addressed by the document** — a byte of
//! [`carrel_core::Document::text`], a block, a heading, a link, an index into
//! a list the document or the library defines — or by nothing at all.
//!
//! An action is TUI chrome, and stays out of this enum, when it carries a
//! terminal cell, a visual row or a count of rows, or when it addresses a
//! surface only the terminal draws and a GUI toolkit provides for itself:
//! its menus, the scrim beside a pane, the hint row, the key sheet's rows,
//! the settings rows, the terminal palettes, the measure in cells. The
//! terminal's `Action` holds every intent plus those, and converts both ways
//! with exhaustive matches, so a new action does not compile until someone
//! decides which kind it is — Ghostty's exhaustive switch over its runtime
//! actions, which turns the disparity between frontends into a list the
//! compiler keeps.
//!
//! Nothing here is a type the core does not already define, a primitive, or
//! one of the small enums below.

use carrel_core::{BlockIdx, NodeId};

/// How far one scroll step moves. Each frontend decides what a line and a
/// page are; this says which was asked for.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Span {
    Line,
    HalfPage,
    Page,
}

/// Where [`Intent::Recenter`] puts the current match.
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

/// A keystroke while a typed filter or the search prompt is open.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SearchKey {
    /// Browse accepted searches in the current session; other filters ignore it.
    Recall(Direction),
    Char(char),
    Backspace,
    /// Keep the matches, close the prompt.
    Accept,
    /// Discard the matches, close the prompt, restore the previous position.
    Cancel,
}

/// Editing a note changes its sidecar only, never the source document.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum NoteKey {
    Char(char),
    Newline,
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    Save,
    Cancel,
}

/// Which end of a list.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Edge {
    First,
    Last,
}

/// One thing a reader can ask for, in any frontend.
///
/// Every `u32` that is not documented otherwise is a doc byte: an offset into
/// [`carrel_core::Document::text`]. A `*At(u32)` on a list is an index into
/// that list, which the receiver clamps, so an intent built from a stale view
/// is inert rather than wrong.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    // --- reading ---
    /// Signed: positive is toward the end of the document.
    Scroll(Span, i32),
    GoToStart,
    GoToEnd,
    /// Signed, in blocks.
    BlockStep(i32),
    /// Put the CURRENT MATCH at the top, middle or bottom of the view. A
    /// reader has no cursor; with no current match, nothing moves.
    Recenter(Where),
    /// Drift down the document until any deliberate motion takes over.
    AutoToggle,
    /// Pin the view to the end of a document that is still growing.
    FollowToggle,
    /// Dim every block but the one at the centre of the view.
    FocusToggle,
    /// The document-info card: words, minutes, structure.
    InfoToggle,
    /// Flip every table between cards and aligned columns.
    TableToggle,
    /// Pan a table in columns view by steps; `None` picks the first visible
    /// table.
    TableScroll {
        block: Option<BlockIdx>,
        delta: i32,
    },
    /// Flip every rendered block — diagrams, display math — between its
    /// rendering and its source.
    RenderedToggle,
    /// The key and gesture reference.
    HelpToggle,
    /// Preferences.
    SettingsToggle,

    // --- search ---
    SearchOpen(Direction),
    SearchKey(SearchKey),
    /// Signed, count-multiplied.
    MatchStep(i32),

    // --- sections ---
    /// Collapse or expand the section at the top of the view.
    FoldToggle,
    FoldAll,
    UnfoldAll,
    /// Collapse or expand whatever this doc byte is inside: a `<details>`
    /// summary, else the heading whose section it is.
    FoldAt(u32),
    OutlineToggle,
    /// Move the outline selection through the filtered list.
    OutlineMove(i32),
    /// A keystroke into the outline filter.
    OutlineKey(SearchKey),
    /// Go to the selected heading, remembering where you were.
    OutlineJump,
    /// Go to an outline entry by index in the FILTERED list.
    OutlineJumpAt(u32),
    /// Go to a heading block.
    OutlineJumpTo(BlockIdx),
    /// Go to a heading by node, as the heading trail names it. Validated on
    /// arrival: a reload in between lands on nothing.
    CrumbJump(NodeId),

    // --- links and history ---
    /// Step the link selection in document order.
    LinkStep(i32),
    /// Follow the selected link. A URL is copied, never opened.
    LinkFollow,
    /// Select and follow a link, by index into `Document::links`.
    LinkOpen(u32),
    /// Copy the selected link's destination instead of following it.
    LinkCopy,
    Back,
    /// Undo the last `Back`.
    Forward,
    /// Go back to a document on the trail, by index into the history.
    BackTo(u32),
    BacklinksToggle,
    BacklinksMove(i32),
    BacklinksOpen,
    BacklinksOpenAt(u32),
    /// What this document links to — the mirror of the backlinks.
    ForwardToggle,
    ForwardMove(i32),
    ForwardOpen,
    ForwardOpenAt(u32),
    /// Between a footnote reference and its definition, either way.
    FootnoteJump,
    /// To the definition of the reference containing this doc byte.
    FootnoteJumpAt(u32),
    /// Open the document just written beside this one.
    SiblingOpen,

    // --- images ---
    /// Open an image full-window; `None` picks the next from the reading
    /// position.
    ImageOpen(Option<BlockIdx>),
    ImageStep(i32),

    // --- selection and copying ---
    /// Pointer down on the `(start, end)` doc bytes of the cluster under it.
    /// Each frontend maps its own pointer to doc bytes BEFORE this point.
    SelectAnchor((u32, u32)),
    /// The pointer dragged to the cluster at these doc bytes.
    SelectDrag((u32, u32)),
    /// Pointer up: copy the selection.
    SelectRelease,
    /// Select the word at a doc byte.
    SelectWord(u32),
    /// Select the whole block at a doc byte.
    SelectBlock(u32),
    /// Copy the focused code block.
    YankBlock,
    /// Copy this code block, and focus it.
    YankBlockAt(BlockIdx),
    /// Step the code-block focus.
    CodeStep(i32),
    /// Copy where a doc byte is, as `PLAN.md:42`.
    CopyRef(u32),
    /// Copy a link to the section a doc byte is in, as `PLAN.md#step-3`.
    CopySection(u32),
    /// Copy the selection as a markdown quote with its source.
    CopyQuote,

    // --- tasks and changes ---
    /// Step through every GFM task item, wrapping.
    TaskStep(i32),
    /// Go to the next task still open, wrapping.
    TaskOpen,
    /// Step through the blocks the last reload changed, wrapping.
    ChangeStep(i32),

    // --- bookmarks ---
    MarkToggle,
    MarkNext,
    MarkListToggle,
    MarkListMove(i32),
    MarkListJump,
    MarkListJumpAt(u32),

    // --- highlights and notes ---
    HighlightAdd,
    HighlightAt(u32),
    NoteAt(u32),
    NoteEdit,
    NotesToggle,
    NoteMove(i32),
    /// Select a note by index.
    NoteSelect(u32),
    NoteJump,
    NoteNext,
    NoteDelete,
    NotesExport,
    NotesCopy,
    NoteInput(NoteKey),

    // --- leaving ---
    /// Close what is open: a pending search, a selection, a pane.
    Dismiss,
    /// Back to the file list when one is behind this document, else quit —
    /// a pager's `q`.
    CloseFile,
    /// To the file list, rooted at this document's folder if nothing is
    /// behind it.
    GoHome,
    Quit,

    // --- the file list ---
    HomeMove(i32),
    /// By the list's own page.
    HomePage(i32),
    /// Select an entry by index (a hit, in content search).
    HomeSelect(usize),
    HomeGo(Edge),
    HomeOpen,
    HomeKey(SearchKey),
    HomeNormalMode,
    HomeFilterMode,
    HomeSearchMode,
    /// Read the content-search hits as one document.
    HomeOpenResults,
    /// Ask for the tags page; the scan runs off the event loop.
    HomeTags,
    /// The tag scan finished. Sent by the frontend's loop, never bound.
    HomeOpenTags,
    /// The tags page, open at the tag containing this doc byte.
    TagOpen(u32),
    /// The next order: newest, by name, recently read.
    HomeSort,
    /// To the folder above the list's root.
    HomeUp,
    /// To a folder on the root's path, by index from the top.
    HomeCrumb(usize),
    /// Open a continue-reading entry by index.
    HomeResume(usize),
    /// Open the built-in first document.
    WelcomeOpen,

    // --- the folder chooser ---
    PickerOpen,
    PickerSelect(usize),
    PickerChoose,
    PickerCancel,
    PickerDescend,
    PickerUp,
}
