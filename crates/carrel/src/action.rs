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
    /// Jump to an absolute visual row from a pointer position.
    ScrollTo(u32),
    Quit,

    // --- home screen ---
    /// Move the selection. Signed; saturates.
    HomeMove(i32),
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
    PickerChoose,
    PickerCancel,
}
