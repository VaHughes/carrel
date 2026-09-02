//! Reader state and the one pure transition over it.
//!
//! **NO RATATUI** — `scripts/check-discipline.sh` rule 6. [`update`] is a pure
//! state transition, so every behavioural test runs with no terminal and a GTK
//! frontend reuses this file verbatim. That is discipline #4 made real rather
//! than aspirational.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use carrel_core::{BlockIdx, DocByte, Document, LinkId, Matches, NodeKind, search};

use crate::action::{Action, Direction, Edge, SearchKey, Span, Where};
use crate::config;
use crate::home::{Home, HomeMode};
use crate::layout::Layout;
use crate::math_art::{self, MathBox};
use crate::scan::Entry;
use crate::view::ViewState;

/// What the event loop must do after a transition.
///
/// The `App` never draws and never exits; it only reports what is now true.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Idle,
    Redraw,
    Quit,
}

#[derive(Debug)]
pub enum Mode {
    Normal,
    /// The search prompt is open. `saved` is the anchor to restore on cancel.
    Search {
        input: String,
        dir: Direction,
        saved: u32,
    },
}

/// The outline picker's transient state: what has been typed, and which row
/// of the FILTERED list is selected. See [`App::outline_matches`].
#[derive(Debug, Default)]
pub struct Outline {
    pub filter: String,
    pub selected: usize,
}

/// What `za` (or a click on a fold marker) decided to fold: a heading's
/// section, or a `<details>` region.
#[derive(Debug, PartialEq, Eq)]
pub enum FoldTarget {
    Section(carrel_core::NodeId),
    /// Index into `doc.details`.
    Details(u32),
}

/// Which screen is in front of the reader.
///
/// The reader's own state (`doc`, `layout`, `view`) always exists; on the home
/// screen it simply holds an empty document, so no reader invariant ever sees a
/// half-initialised value.
#[derive(Debug)]
pub enum Screen {
    /// Boxed: `Home` carries the entry list, so an unboxed variant would make
    /// every `App` pay for it even while reading.
    Home(Box<Home>),
    Reader,
}

/// Both laid-out forms of one math block.
///
/// Pure functions of the expression and therefore **width-independent**: a
/// resize selects between them, it never recomputes them.
#[derive(Clone, Debug)]
pub struct MathArt {
    pub display: MathBox,
    pub inline: MathBox,
}

/// Which rendering a math block gets at a given width. See [`App::math_form`].
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum MathForm {
    /// Full 2D box art.
    Display,
    /// The single-row form, when the art is wider than the viewport.
    Inline,
    /// The literal LaTeX source: unparseable, too wide even inline, or the
    /// user pressed the rendered-blocks toggle.
    Source,
}

// Four independent on/off facts (rendered blocks, card view, hints,
// streaming) are four bools; an enum would invent states that cannot occur.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug)]
pub struct App {
    pub screen: Screen,
    pub doc: Document,
    pub matches: Option<Matches>,
    pub layout: Layout,
    pub view: ViewState,
    pub mode: Mode,
    pub path: String,
    /// The open file on disk, for resolving relative links. `None` until a
    /// real file is opened — and `None` for a piped document, which is what
    /// keeps the reloader and position persistence inert in pager mode.
    pub file: Option<PathBuf>,
    /// A stdin stream is still arriving. Presentation only: the footer lamp
    /// and the path label read it; nothing else may.
    pub streaming: bool,
    /// The backlinks pane, while it is open. `None` means closed; the rows
    /// stream in from `links.rs` and the event loop appends them.
    pub backlinks: Option<Backlinks>,
    /// The forward-links pane, while it is open. Derived whole at open —
    /// see [`Forward`].
    pub forward: Option<Forward>,
    /// The bookmark list (`"`) while it is up. Only the cursor lives here —
    /// the rows derive from `marks` at every use, so a mark added or cleared
    /// under the pane shows immediately.
    pub mark_list: Option<usize>,
    /// The section tree in the left margin is wanted (config `outline_margin`).
    /// Whether it actually shows also needs [`Self::gutter_w`].
    pub outline_margin: bool,
    /// Bookmarks for the open document, in document order.
    ///
    /// **Doc bytes**, so they survive reflow and resize by construction —
    /// the same currency as a search match and the scroll anchor. They do
    /// NOT survive an edit to the file, which is honest for a reader and is
    /// said so in the help.
    pub marks: Vec<u32>,
    /// May this document be sniffed as a raw diff? Set per document by
    /// whoever opened it — see [`Self::parse_adapting`].
    pub diff_ok: bool,
    /// `--diff` / `--no-diff`: a command-line override that outranks the
    /// per-document rule for the whole run.
    pub diff_forced: Option<bool>,
    /// Pinned to the end of a growing document.
    ///
    /// **Deliberately starts OFF, even for a pipe.** Nothing may move under
    /// the reader unless they asked for it; `F` asks, and so does `G` while
    /// streaming, because going to the end of a document that is still being
    /// written is a statement about wanting the end.
    pub following: bool,
    /// The code block the block cursor sits on, if any. Paint marks it and
    /// `y` copies it.
    pub code_focus: Option<BlockIdx>,
    /// The breadcrumb band is wanted (config/`B`). Whether it actually shows
    /// also needs [`Self::band`] — a document with no headings reserves no
    /// rows for a band that would be permanently blank.
    pub breadcrumb: bool,
    /// Cached at parse like `words`: does the document have any heading at
    /// all? Height must be stable while reading, so this is a per-document
    /// fact, not a per-frame scan.
    has_headings: bool,
    /// The accumulated piped document, retained so `Ctrl-O` can come back to
    /// it after following a link out — a pipe has no path to re-read.
    pub piped: Option<String>,
    /// Folded heading ids. Doc-space and width-independent, so fold state
    /// cannot be invalidated by reflow, by construction. Cleared on reload —
    /// the ids indexed the old parse (the selection precedent).
    pub folded: std::collections::HashSet<carrel_core::NodeId>,
    /// Folded `<details>` regions, by index into `doc.details`. The same
    /// currency and the same clearing rule as `folded`: doc bytes under the
    /// hood, gone when the document is re-parsed.
    pub folded_details: std::collections::BTreeSet<u32>,
    /// Where we came from: `(file, anchor)` pairs. `Ctrl-O` pops.
    ///
    /// Capped, like every other remembered list here — auto-repeat on `%` or
    /// Tab+Enter reaches thousands of entries in seconds, and this was the
    /// only structure in the state layer that grew without a bound or a
    /// sentence saying why it need not.
    pub history: Vec<(PathBuf, u32)>,
    /// The link `Tab` has selected, if any.
    pub selected_link: Option<LinkId>,
    /// Pixel dimensions of decoded images, by block. Plain numbers — the
    /// protocol state lives with the painter, never here.
    pub image_dims: HashMap<BlockIdx, (u32, u32)>,
    /// Wikilink targets resolved to files, rebuilt at every `open_path`.
    /// Only resolved links appear; a missing key means "no note by that
    /// name" as of open time. Plain data — render reads it for `file://`
    /// OSC 8 wrappers.
    pub wiki: HashMap<LinkId, PathBuf>,
    /// Rendered mermaid box art by block, streamed in like image dims. Art
    /// is presentation-shaped but width-INDEPENDENT (fixed-width text), so
    /// it may live in state; a block with no entry paints as source.
    pub diagram_art: HashMap<BlockIdx, Vec<String>>,
    /// Laid-out math art by block, both forms, computed ONCE per document.
    /// A `MathExpr` has no width input, so a resize chooses between the forms
    /// (see [`App::math_form`]) and never recomputes them. A block absent from
    /// this map failed to parse and renders as its literal LaTeX source.
    pub math_art: HashMap<BlockIdx, MathArt>,
    /// `m` flips every RENDERED block -- mermaid diagrams and math alike --
    /// between art and source, like `t` for
    /// tables.
    pub show_rendered: bool,
    /// The terminal's cell size in pixels, set once at startup by the
    /// frontend. `(8, 16)` is the conservative guess when detection fails.
    pub font_px: (u16, u16),
    /// A one-shot status-bar note; cleared by the next action.
    pub note: Option<String>,
    /// Where the picker persists its choice. `None` — the constructor
    /// default — persists nothing, so unit tests can drive `update` freely.
    /// The BINARY sets this at startup; forgetting that shows up in the pty
    /// smoke, not as a stomped developer config. (A home.rs test once wrote
    /// `root = /tmp/.tmpXXXXXX` into the real config on every test run.)
    /// The directory this reading session is rooted at — the home screen's
    /// root, or the directory the opened file came from. Injected by the
    /// binary like [`Self::config_dir`], and fixed for the session: a link
    /// followed out of it does not move it.
    ///
    /// `None` disables containment, which is what the state layer's own
    /// tests want; the binary always sets it.
    pub library_root: Option<std::path::PathBuf>,
    /// A link that leaves the library, waiting for a second Enter. One-shot:
    /// cleared by stepping to another link or by changing document.
    pending_open: Option<(LinkId, std::path::PathBuf)>,
    pub config_dir: Option<std::path::PathBuf>,
    /// The directory `carrel` was run from, as it was at startup. Same
    /// contract as [`Self::config_dir`]: `None` in every constructor, set by
    /// the BINARY, so no test's picker ever reads the developer's real
    /// working directory.
    ///
    /// This is where `d` opens (maintainer report, 2026-09-01), and it is
    /// deliberately NOT the home screen's root: with a saved `root =` in the
    /// config the two differ from the first frame, and the one the reader
    /// means by "here" is the one they typed the command in.
    pub launch_dir: Option<std::path::PathBuf>,
    /// Where reading positions persist. Same contract as `config_dir`:
    /// `None` in every constructor, set by the BINARY at startup, so no test
    /// can ever write the real state file.
    pub state_dir: Option<std::path::PathBuf>,
    /// When `false` (the default), an overflowing table lays out as cards
    /// instead of wrapping in place. `t` flips this. See
    /// [`Action::TableToggle`].
    pub wrap_tables: bool,
    /// The lamplight hint footer is showing. `H` and a click on the lamp
    /// toggle it; persisted through `config_dir` so the choice sticks.
    pub hints: bool,
    /// `Some(scroll)` while the help overlay is up. Presentation-free state:
    /// the sheet's row offset, clamped by the painter against its content.
    pub help: Option<u16>,
    /// `Some` while the outline picker is up. The heading list itself is
    /// DERIVED from `doc` at every use — nothing here can go stale.
    pub outline: Option<Outline>,
    /// The document-info card is showing (`I`). Pure presentation intent —
    /// the rows are derived fresh at every paint.
    pub info: bool,
    /// Spotlight (`S`): dim every block but the one nearest the centre of
    /// the view. Presentation only — the painter reads it, nothing else.
    pub focus: bool,
    /// Auto-read (`A`): the event loop sends [`Action::AutoTick`] on its own
    /// cadence and each one drifts a row. Any deliberate motion turns this
    /// off — nothing keeps moving once the reader takes the wheel.
    pub auto_read: bool,
    /// When the open file last changed on disk. Captured once at open by
    /// whoever did the reading (the binary or [`Self::open_path`]); `None`
    /// for a piped document. State-layer I/O stops there.
    pub mtime: Option<std::time::SystemTime>,
    /// The mouse selection, in doc bytes — the same currency as a search
    /// match, so reflow and resize cannot invalidate it.
    pub selection: Option<std::ops::Range<u32>>,
    /// The cluster the current drag started on. `None` when no drag is live.
    pub sel_anchor: Option<(u32, u32)>,
    /// One-shot clipboard outbox: the state machine fills it, the event loop
    /// drains it into an OSC 52 escape. The state layer never does I/O.
    pub clipboard: Option<String>,
    /// One-shot browser outbox, the twin of [`Self::clipboard`]: the state
    /// machine puts an already-vetted URL here and the event loop hands it to
    /// the desktop. The state layer still does no I/O, and — the part that
    /// matters — **nothing reaches this field that has not passed
    /// [`openable_url`]**, so the allowlist cannot be bypassed by a caller
    /// that forgot about it.
    pub open_url: Option<String>,
    /// The home screen this document was opened from, if any. `q` restores
    /// it — entries, filter, and scroll intact — instead of quitting.
    pub home_stash: Option<Box<Home>>,
    pub cols: u16,
    pub rows: u16,
    /// Words in the document, counted once when it is parsed — a per-frame
    /// count would be O(n) on every keystroke. Feeds [`Self::minutes_left`].
    pub words: usize,
    /// The reading measure in columns; `0` is off (full bleed). Constructors
    /// carry [`config::DEFAULT_MEASURE`] and `main.rs` alone reads the config
    /// file — the same contract as `config_dir` and `state_dir`, and for the
    /// same reason: a test must never be able to reach the real config.
    pub max_width: u16,
}

/// Reader page margins, in cells: the text is inset from the terminal edges
/// so the page reads like a page (field note). These live inside
/// [`App::text_size`] — the ONE function paint geometry and layout width are
/// both derived from — so wrapping and painting cannot disagree about them.
/// The scrollbar keeps the true right edge and the status bar the full width;
/// margins belong to the text alone.
/// Width of the margin outline when it shows.
pub const GUTTER_W: u16 = 22;

pub const PAD_LEFT: u16 = 2;
pub const PAD_RIGHT: u16 = 2;
pub const PAD_TOP: u16 = 1;
pub const PAD_BOTTOM: u16 = 1;

/// Words per minute assumed for the reading estimate.
///
/// 200 is the conservative end of the usual adult prose range (200–250) and is
/// deliberately not tunable: the number is an *estimate offered to a reader*,
/// not a measurement, and a config key would imply a precision it does not
/// have. Code blocks and tables read slower than prose, so a code-heavy
/// document will over-estimate; that is accepted.
pub const READING_WPM: usize = 200;

/// How often auto-read (`A`) asks for the next row. The event loop owns the
/// clock; this only documents the pace. 300 ms is two hundred rows a minute
/// — brisk enough to feel like reading, gentle enough to stop mid-sentence.
pub const AUTO_READ_MS: u64 = 300;

/// Epoch seconds as `YYYY-MM-DD HH:MM`, no calendar library — Howard
/// Hinnant's `civil_from_days` arithmetic, which every state timestamp here
/// already implicitly assumes.
#[must_use]
pub fn format_epoch(secs: u64) -> String {
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        rem / 3600,
        (rem % 3600) / 60
    )
}

/// Whitespace-separated runs in the display text.
///
/// Space 2 is already flattened and decorations are paint-time, so this counts
/// what is actually on screen — no markup, no table separators, no quote bars.
fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

/// The prose budget inside a given bleed budget. `max_width == 0` means the
/// measure is off, which must reproduce the pre-measure geometry exactly.
const fn measure_of(bleed: u16, max_width: u16) -> u16 {
    if max_width == 0 || bleed < max_width {
        bleed
    } else {
        max_width
    }
}

impl App {
    /// The text area: `(prose width, bleed width, height)`. One column is
    /// reserved for the scrollbar and one row for the status line.
    ///
    /// **Two budgets, both derived.** Prose wraps at the first — the reading
    /// measure — while a block whose content has an intrinsic width of its own
    /// (table, code, math, image) may use the second. Neither is stored: both
    /// are pure functions of `(cols, max_width)`, recomputed on every resize,
    /// so the one authoritative coordinate space is undisturbed and a search
    /// hit found at 200 columns is the same byte at 90.
    ///
    /// `render.rs` **must** derive its rects from this, or layout and paint
    /// disagree about how much room the text has and wrapping goes wrong.
    #[must_use]
    pub const fn text_size(
        cols: u16,
        rows: u16,
        footer: bool,
        band: bool,
        max_width: u16,
        gutter: u16,
    ) -> (u16, u16, u16) {
        // Status row always; the lamplight hint row only while showing.
        let chrome = if footer { 2 } else { 1 };
        let bleed = cols.saturating_sub(1 + PAD_LEFT + PAD_RIGHT + gutter);
        (
            measure_of(bleed, max_width),
            bleed,
            rows.saturating_sub(chrome + Self::top_chrome(band) + PAD_BOTTOM),
        )
    }

    /// Rows above the text: the breadcrumb band (crumb + rule) or the plain
    /// pad. The vertical twin of the margin logic in [`Self::text_x`].
    #[must_use]
    pub const fn top_chrome(band: bool) -> u16 {
        if band { 2 } else { PAD_TOP }
    }

    /// The band shows only when wanted AND the document has headings.
    #[must_use]
    pub const fn band(&self) -> bool {
        self.breadcrumb && self.has_headings
    }

    /// The text area's top row, in absolute terminal cells.
    ///
    /// Paint and hit-testing both come through here, exactly like
    /// [`Self::text_x`] — if they ever stop, a click lands one row off and
    /// no frame test will notice.
    #[must_use]
    pub const fn text_y(&self) -> u16 {
        Self::top_chrome(self.band())
    }

    /// The prose column's left edge, in absolute terminal cells.
    ///
    /// [`PAD_LEFT`] is the *minimum* margin, not the left edge: past the
    /// measure the margin grows to absorb the excess, so the text stays
    /// centred on the full area's axis instead of hugging the left wall.
    ///
    /// Hit-testing and paint both come through here. If they ever stop doing
    /// so, a click lands on the wrong byte and no frame test will notice.
    #[must_use]
    pub const fn text_x(cols: u16, max_width: u16, gutter: u16) -> u16 {
        let bleed = cols.saturating_sub(1 + PAD_LEFT + PAD_RIGHT + gutter);
        PAD_LEFT + gutter + (bleed - measure_of(bleed, max_width)) / 2
    }

    /// Columns reserved on the left for the margin outline.
    ///
    /// Zero unless the reader asked for it (`outline_margin`), the document
    /// has headings to show, and the terminal can spare the columns —
    /// **the measure is the point of the reading desk, so the gutter folds
    /// away rather than squeezing it.**
    #[must_use]
    pub fn gutter_w(&self) -> u16 {
        if !self.outline_margin || !self.has_headings {
            return 0;
        }
        let bleed = self.cols.saturating_sub(1 + PAD_LEFT + PAD_RIGHT);
        let measure = measure_of(bleed, self.max_width);
        if bleed.saturating_sub(measure) >= GUTTER_W + 4 {
            GUTTER_W
        } else {
            0
        }
    }

    #[must_use]
    pub fn new(path: String, doc: Document, cols: u16, rows: u16) -> Self {
        let has_headings = doc
            .nodes
            .iter()
            .any(|n| matches!(n.kind, carrel_core::NodeKind::Heading { .. }));
        let (w, bleed, _) =
            Self::text_size(cols, rows, true, has_headings, config::DEFAULT_MEASURE, 0);
        let layout = Layout::with_measure(&doc, bleed.max(1), w.max(1), HashMap::new(), false);
        let mut app = Self {
            screen: Screen::Reader,
            doc,
            matches: None,
            layout,
            view: ViewState::new(),
            mode: Mode::Normal,
            path,
            file: None,
            streaming: false,
            backlinks: None,
            forward: None,
            mark_list: None,
            outline_margin: false,
            marks: Vec::new(),
            diff_ok: false,
            diff_forced: None,
            following: false,
            code_focus: None,
            piped: None,
            breadcrumb: true,
            has_headings,
            folded: std::collections::HashSet::new(),
            folded_details: std::collections::BTreeSet::new(),
            history: Vec::new(),
            selected_link: None,
            image_dims: HashMap::new(),
            wiki: HashMap::new(),
            diagram_art: HashMap::new(),
            math_art: HashMap::new(),
            show_rendered: true,
            font_px: (8, 16),
            note: None,
            library_root: None,
            pending_open: None,
            config_dir: None,
            launch_dir: None,
            state_dir: None,
            wrap_tables: false,
            hints: true,
            help: None,
            outline: None,
            info: false,
            focus: false,
            auto_read: false,
            mtime: None,
            selection: None,
            sel_anchor: None,
            clipboard: None,
            open_url: None,
            home_stash: None,
            cols,
            rows,
            words: 0,
            max_width: config::DEFAULT_MEASURE,
        };
        // Math art is pure computation over the document -- no config, no
        // filesystem -- so the constructor may do it, and every entry point
        // (new, open_path, reload) then has art without a special case.
        app.rebuild_math_art();
        app.words = word_count(&app.doc.text);
        app.relayout();
        app
    }

    /// Start on the home screen. `cached` is painted immediately; the caller
    /// starts the live walk.
    #[must_use]
    pub fn new_home(root: PathBuf, cached: Vec<Entry>, cols: u16, rows: u16) -> Self {
        let mut app = Self::new(String::new(), Document::parse(""), cols, rows);
        app.screen = Screen::Home(Box::new(Home::new(root, cached)));
        app
    }

    #[must_use]
    pub fn is_home(&self) -> bool {
        matches!(self.screen, Screen::Home(_))
    }

    #[must_use]
    pub fn home(&self) -> Option<&Home> {
        match &self.screen {
            Screen::Home(h) => Some(h),
            Screen::Reader => None,
        }
    }

    /// Put a one-shot note where the CURRENT screen's status bar reads it.
    ///
    /// The reader paints `App::note`; the home screen paints `Home::note`.
    /// A note written to the wrong one is invisible — which is exactly how
    /// the theme name failed to appear when cycling on the home screen.
    pub fn set_note(&mut self, note: String) {
        if let Some(h) = self.home_mut() {
            h.note = Some(note);
        } else {
            self.note = Some(note);
        }
    }

    pub fn home_mut(&mut self) -> Option<&mut Home> {
        match &mut self.screen {
            Screen::Home(h) => Some(h),
            Screen::Reader => None,
        }
    }

    /// Persist the current reading position — anchor 0 clears the entry.
    /// Quietly does nothing without a state dir (tests) or a file (home).
    fn save_position(&self) {
        if let (Some(dir), Some(file)) = (self.state_dir.as_deref(), self.file.as_deref()) {
            // The progress numbers are the status bar's own, saved alongside
            // so the home screen can show them without opening the file.
            let _ = crate::state::save_position_in(
                dir,
                file,
                self.view.anchor,
                self.permille(),
                u32::try_from(self.words).unwrap_or(u32::MAX),
            );
        }
    }

    /// How far through the document, in permille — the percentage the status
    /// bar shows, at a resolution worth persisting.
    #[must_use]
    pub fn permille(&self) -> u16 {
        let max = self.layout.max_scroll(self.text_h());
        if max == 0 {
            return 1000;
        }
        let frac = f64::from(self.view.scroll_row.min(max)) / f64::from(max);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let p = (frac * 1000.0).round() as u16;
        p.min(1000)
    }

    /// Persist the bookmark list. Inert for a piped document, which has no
    /// path to key on — the same contract position saving already has.
    fn save_marks(&self) {
        if let (Some(dir), Some(file)) = (self.state_dir.as_deref(), self.file.as_deref()) {
            let _ = crate::state::save_marks_in(dir, file, &self.marks);
        }
    }

    /// Load the bookmark list for the open file. Called wherever a document
    /// is opened, beside the position restore.
    fn load_marks(&mut self) {
        self.marks = match (self.state_dir.as_deref(), self.file.as_deref()) {
            (Some(dir), Some(file)) => crate::state::load_marks_in(dir, file),
            _ => Vec::new(),
        };
    }

    /// Load a document and switch to the reader.
    ///
    /// Neutral about history — the caller decides whether this navigation is
    /// worth remembering, so `Back` itself can reuse it without double-pushing.
    /// A home screen left behind is stashed so `q` can restore it whole.
    /// It saves the outgoing file's reading position and restores the
    /// incoming one — callers that reposition afterwards (history anchors,
    /// fragment jumps) run later and simply win.
    pub fn open_path(&mut self, path: &Path) -> std::io::Result<()> {
        self.save_position();
        let src = read_document(path)?;
        // A `.md` file is never sniffed; `.diff`/`.patch` always are. Set
        // before parsing, because `parse_adapting` reads it.
        self.diff_ok = self.diff_forced.unwrap_or_else(|| {
            matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("diff" | "patch")
            )
        });
        self.mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        self.doc = self.parse_adapting(&src);
        self.load_marks();
        self.matches = None;
        self.forward = None;
        self.mode = Mode::Normal;
        self.view = ViewState::new();
        self.layout = Layout::with_measure(
            &self.doc,
            self.bleed_w(),
            self.text_w(),
            HashMap::new(),
            false,
        );
        self.path = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        self.file = Some(path.to_path_buf());
        self.forget_derived_state();
        // A different document: the panes that answered questions about the
        // one we are leaving would otherwise keep answering them.
        self.backlinks = None;
        self.mark_list = None;
        self.rebuild_math_art();
        self.words = word_count(&self.doc.text);
        self.has_headings = self
            .doc
            .nodes
            .iter()
            .any(|n| matches!(n.kind, carrel_core::NodeKind::Heading { .. }));
        if let Screen::Home(h) = std::mem::replace(&mut self.screen, Screen::Reader) {
            self.home_stash = Some(h);
        }
        self.resolve_wikilinks();
        self.restore_position();
        Ok(())
    }

    /// Does following this link leave the library?
    ///
    /// A markdown file is untrusted input — a shared vault, a downloaded
    /// README — and `Path::join` with an absolute component discards the base
    /// entirely, so `[x](/etc/passwd)` needed no `..` at all to read it. The
    /// README states in bold that carrel "reads only the directory you point
    /// it at"; this is the half of that claim that was not being kept.
    ///
    /// Both paths are canonicalized, so a symlink pointing out of the tree is
    /// caught too. A target that cannot be canonicalized does not exist, and
    /// the open that follows reports that itself.
    #[must_use]
    pub fn escapes_library(&self, target: &Path) -> bool {
        let Some(root) = self.library_root.as_deref() else {
            return false;
        };
        let (Ok(root), Ok(target)) = (root.canonicalize(), target.canonicalize()) else {
            return false;
        };
        !target.starts_with(&root)
    }

    /// The second Enter on the same link: `true` once, then forgotten.
    fn confirmed_open(&mut self, id: LinkId, target: &Path) -> bool {
        matches!(self.pending_open.take(), Some((prev, ref path)) if prev == id && path == target)
    }

    /// Ask for a second Enter before leaving the library.
    fn ask_before_leaving(&mut self, id: LinkId, target: &Path) -> Outcome {
        self.note = Some(format!(
            "outside the library — Enter again to open {}",
            target.display()
        ));
        self.pending_open = Some((id, target.to_path_buf()));
        Outcome::Redraw
    }

    /// Remember where we came from, dropping the oldest when the trail gets
    /// long. Going back a few hundred documents is a trail; going back ten
    /// thousand is a leak.
    fn push_history(&mut self, from: PathBuf, anchor: u32) {
        const HISTORY_CAP: usize = 256;
        // Repeated `%` between the same two points would otherwise push a
        // duplicate per press.
        if self
            .history
            .last()
            .is_some_and(|e| *e == (from.clone(), anchor))
        {
            return;
        }
        if self.history.len() >= HISTORY_CAP {
            self.history.remove(0);
        }
        self.history.push((from, anchor));
    }

    /// Forget every piece of state whose bytes or block indices belong to a
    /// parse that is about to be replaced.
    ///
    /// Both `open_path` and `reload_from` must do this, and having it written
    /// out twice is exactly how they drifted: `reload_from` learned to clear
    /// the selection ("its bytes indexed the old text") and `open_path` never
    /// did, and neither ever cleared `code_focus` — so stepping to a late
    /// code block, following a link to a shorter document and pressing `y`
    /// indexed a block that no longer existed and aborted the process.
    ///
    /// What is NOT here is anything the two disagree about on purpose:
    /// `matches` survives a reload and is re-run, `forward` is re-derived
    /// rather than dropped, and `open_path` alone clears the panes that
    /// answer questions about a file it is leaving.
    fn forget_derived_state(&mut self) {
        self.pending_open = None;
        self.selection = None;
        self.sel_anchor = None;
        self.selected_link = None;
        self.code_focus = None;
        self.folded.clear();
        self.folded_details.clear();
        self.image_dims.clear();
        self.diagram_art.clear();
    }

    /// Re-read the open file in place: same document identity, new content.
    ///
    /// What survives, and why it can: the anchor is a doc byte, so an append
    /// — the agent-writing-a-log case — moves nothing at all, and a rewrite
    /// clamps; the search re-runs from the pattern `Matches` kept for
    /// exactly this; history and the stashed home screen are untouched. The
    /// selection clears — its bytes indexed the old text. On error (a
    /// vanished file) the last good parse stays on screen; the caller notes.
    pub fn reload(&mut self) -> std::io::Result<()> {
        let Some(path) = self.file.clone() else {
            return Ok(());
        };
        let src = read_document(&path)?;
        self.reload_from(&src);
        self.note = Some("reloaded".into());
        Ok(())
    }

    fn parse_adapting(&self, src: &str) -> Document {
        adapt(src, self.diff_ok)
    }

    /// The body of [`Self::reload`], for a document that has no file to
    /// re-read — a piped stream re-parses through here on every append.
    /// Sets no note: one reload is an event, a stream of them is weather.
    pub fn reload_from(&mut self, src: &str) {
        self.doc = self.parse_adapting(src);
        self.layout = Layout::with_measure(
            &self.doc,
            self.bleed_w(),
            self.text_w(),
            HashMap::new(),
            false,
        );
        self.forget_derived_state();
        self.rebuild_math_art();
        self.words = word_count(&self.doc.text);
        self.has_headings = self
            .doc
            .nodes
            .iter()
            .any(|n| matches!(n.kind, carrel_core::NodeKind::Heading { .. }));
        if self.forward.is_some() {
            // The rows indexed the old parse; re-derive rather than lie.
            let rows = forward_rows(self);
            if let Some(f) = self.forward.as_mut() {
                f.rows = rows;
                f.selected = f.selected.min(f.rows.len().saturating_sub(1));
            }
        }
        let last = u32::try_from(self.doc.text.len().saturating_sub(1)).unwrap_or(u32::MAX);
        self.view.anchor = self.view.anchor.min(last);
        self.view.restore(&self.doc, &self.layout, self.text_h());
        if let Some(old) = self.matches.take() {
            let mut m = search(&self.doc, &old.pattern, old.flexible_ws);
            m.current = old
                .current
                .map(|i| i.min(m.ranges.len().saturating_sub(1)))
                .filter(|_| !m.ranges.is_empty());
            self.matches = Some(m);
        }
        if let Some(o) = self.outline.as_mut() {
            o.selected = 0; // re-clamped against the new headings on use
        }
        self.resolve_wikilinks();
    }

    /// The directory links resolve against: the document's own, or the
    /// working directory for a pathless (piped) document — `git show |
    /// carrel` run inside a repo makes its relative links work.
    fn doc_dir(&self) -> PathBuf {
        self.file
            .as_deref()
            .and_then(Path::parent)
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
    }

    /// The blocks a fold hides: everything strictly inside a folded
    /// heading's span, nested headings included, and the body of every
    /// folded `<details>` — its summary stays visible, like a folded
    /// heading's own line does. Derived per call from `folded` × the
    /// section index × `doc.details` — never stored.
    fn hidden_blocks(&self) -> std::collections::HashSet<BlockIdx> {
        if self.folded.is_empty() && self.folded_details.is_empty() {
            return std::collections::HashSet::new();
        }
        let mut spans: Vec<(u32, u32)> = self
            .folded
            .iter()
            .map(|&id| {
                let n = &self.doc.nodes[id.0 as usize];
                (n.doc.end, self.doc.section_end(id))
            })
            .collect();
        spans.extend(self.folded_details.iter().filter_map(|&i| {
            self.doc
                .details
                .get(i as usize)
                .map(|r| (r.summary.end, r.end))
        }));
        (0..self.doc.block_count())
            .map(|i| BlockIdx(i as u32))
            .filter(|&b| {
                let start = self.doc.node_for_block(b).doc.start;
                spans.iter().any(|&(from, to)| start > from && start < to)
            })
            .collect()
    }

    /// Unfold every folded section that hides `byte`. Returns whether
    /// anything changed. A byte inside a folded heading's own text is
    /// visible already, so the heading itself does not count; the same
    /// holds for a `<details>` summary row.
    fn unfold_to(&mut self, byte: u32) -> bool {
        let mut changed = false;
        for id in self.doc.section_path(byte) {
            if byte > self.doc.nodes[id.0 as usize].doc.end && self.folded.remove(&id) {
                changed = true;
            }
        }
        // A details body hides its byte only when the byte sits strictly
        // inside it; a byte at or past `end` belongs to later content.
        let victims: Vec<u32> = self
            .folded_details
            .iter()
            .copied()
            .filter(|&i| {
                self.doc
                    .details
                    .get(i as usize)
                    .is_some_and(|r| byte > r.summary.end && byte < r.end)
            })
            .collect();
        for i in victims {
            self.folded_details.remove(&i);
            changed = true;
        }
        changed
    }

    /// The next code block in direction `n`, from wherever the cursor is.
    ///
    /// `n == 0` means "the one at or after the current position", which is
    /// what `y` uses when nothing is focused yet. Stepping is over code
    /// blocks alone, not every block — `{`/`}` already walk all of them, and
    /// a motion that stops at every paragraph is useless for the thing this
    /// exists to serve: getting the command out of an agent's answer.
    #[must_use]
    pub fn next_code_block(&self, n: i32) -> Option<BlockIdx> {
        let is_code = |b: u32| {
            matches!(
                self.doc.node_for_block(BlockIdx(b)).kind,
                carrel_core::NodeKind::CodeBlock { .. }
            )
        };
        let last = u32::try_from(self.doc.block_count()).ok()?.checked_sub(1)?;
        let here = self
            .code_focus
            .map_or_else(|| self.layout.block_at_row(self.view.scroll_row).0, |b| b.0);
        if n == 0 {
            return (here..=last).find(|&b| is_code(b)).map(BlockIdx);
        }
        let mut at = here;
        for _ in 0..n.unsigned_abs() {
            let found = if n < 0 {
                (0..at).rev().find(|&b| is_code(b))
            } else {
                (at.saturating_add(1)..=last).find(|&b| is_code(b))
            };
            at = found?;
        }
        Some(BlockIdx(at))
    }

    /// The one gate every byte-targeted jump goes through: **anything that
    /// reveals a byte unfolds its way there** — search, outline, links,
    /// fragments, back. A fold must never make a destination unreachable.
    pub fn reveal_byte(&mut self, byte: u32, h: u16, at: crate::action::Where) {
        if self.unfold_to(byte) {
            self.relayout();
        }
        self.view.reveal(&self.doc, &self.layout, byte, h, at);
    }

    /// Re-derive the page after `folded` changed: if the anchor's block is
    /// now hidden, the reader is "at" the folded heading — pull the anchor
    /// to it before restoring, or restore would land on an arbitrary
    /// neighbour of a row that no longer exists.
    fn after_fold_change(&mut self) {
        self.relayout();
        let b = self
            .doc
            .block_at_doc(carrel_core::DocByte(self.view.anchor));
        if self.layout.height(b) == 0 {
            let anchor = self.view.anchor;
            if let Some(&id) = self
                .doc
                .section_path(anchor)
                .iter()
                .rev()
                .find(|id| self.folded.contains(id))
            {
                self.view.anchor = self.doc.nodes[id.0 as usize].doc.start;
            }
            let h = self.text_h();
            self.view.restore(&self.doc, &self.layout, h);
        }
    }

    /// `za`'s target: the innermost section for the top-visible byte — the
    /// breadcrumb's own byte, except a heading at the top of the view
    /// targets itself (the breadcrumb pops it; a fold should grab it).
    ///
    /// A `<details>` region competes by the same rule sections do: whichever
    /// candidate *starts deeper* wins, and a summary at the top of the view
    /// targets its own region outright, exactly as a heading does.
    fn fold_target(&self) -> Option<FoldTarget> {
        let b = self.layout.block_at_row(self.view.scroll_row);
        if b.get() >= self.doc.block_count() {
            return None;
        }
        let node = self.doc.node_for_block(b);
        if matches!(node.kind, carrel_core::NodeKind::Heading { .. }) {
            return Some(FoldTarget::Section(node.id));
        }
        // The top block carries a summary: that region is the target.
        if let Some((i, _)) = self
            .doc
            .details
            .iter()
            .enumerate()
            .find(|(_, r)| node.doc.contains(&r.summary.start))
        {
            return Some(FoldTarget::Details(u32::try_from(i).unwrap_or(u32::MAX)));
        }
        let byte = node.doc.start;
        let sec = self.doc.section_path(byte).last().copied();
        let det = self
            .doc
            .details
            .iter()
            .position(|r| r.summary.end < byte && byte < r.end);
        match (det, sec) {
            (Some(i), Some(h)) => {
                let detail_first =
                    self.doc.details[i].summary.start > self.doc.nodes[h.0 as usize].doc.start;
                Some(if detail_first {
                    FoldTarget::Details(u32::try_from(i).unwrap_or(u32::MAX))
                } else {
                    FoldTarget::Section(h)
                })
            }
            (Some(i), None) => Some(FoldTarget::Details(u32::try_from(i).unwrap_or(u32::MAX))),
            (None, Some(h)) => Some(FoldTarget::Section(h)),
            (None, None) => None,
        }
    }

    /// Resume the saved reading position for the open file, silently, with a
    /// note. Called by `open_path` — and by the binary for a direct
    /// `carrel FILE` open, which builds its `App` without `open_path`.
    pub fn restore_position(&mut self) {
        if let (Some(dir), Some(file)) = (self.state_dir.as_deref(), self.file.as_deref())
            && let Some(saved) = crate::state::load_position_in(dir, file)
            && saved > 0
        {
            let last = u32::try_from(self.doc.text.len().saturating_sub(1)).unwrap_or(u32::MAX);
            self.view.anchor = saved.min(last);
            self.view.restore(&self.doc, &self.layout, self.text_h());
            self.note = Some("resumed — gg for top".into());
        }
    }

    /// Resolve every `[[wikilink]]` target once per document. One directory
    /// listing plus an in-memory index scan per unique target — cheap, and it
    /// gives the painter a synchronous answer for `file://` hyperlinks.
    fn resolve_wikilinks(&mut self) {
        self.wiki.clear();
        let dir = self.doc_dir();
        let dir = dir.as_path();
        let index = self
            .home_stash
            .as_deref()
            .map_or(&[][..], |h| h.entries.as_slice());
        for i in 0..self.doc.links.len() {
            let id = LinkId(u32::try_from(i).unwrap_or(u32::MAX));
            if !self.doc.is_wikilink(id) {
                continue;
            }
            let target = &self.doc.links[i];
            let bare = target.split('#').next().unwrap_or("");
            if bare.is_empty() {
                continue; // [[#Heading]] — in-document, nothing to resolve
            }
            if let Some(p) = crate::wiki::resolve(bare, dir, index) {
                self.wiki.insert(id, p);
            }
        }
    }

    /// Every heading, in reading order, as layout blocks. Derived, never
    /// stored — a heading list cannot go stale because it never exists
    /// between frames.
    #[must_use]
    pub fn headings(&self) -> Vec<BlockIdx> {
        (0..self.doc.block_count())
            .map(|i| BlockIdx(u32::try_from(i).unwrap_or(u32::MAX)))
            .filter(|b| {
                matches!(
                    self.doc.node_for_block(*b).kind,
                    carrel_core::NodeKind::Heading { .. }
                )
            })
            .collect()
    }

    /// The document-info card's rows: `(label, value)`, derived fresh every
    /// call from the parse and the counters already kept. Nothing here is
    /// stored, so nothing here can go stale.
    #[must_use]
    pub fn info_rows(&self) -> Vec<(&'static str, String)> {
        let mut headings = 0usize;
        let mut code = 0usize;
        let mut tables = 0usize;
        let mut images = 0usize;
        let mut math = 0usize;
        for n in &self.doc.nodes {
            match n.kind {
                carrel_core::NodeKind::Heading { .. } => headings += 1,
                carrel_core::NodeKind::CodeBlock { .. } => code += 1,
                carrel_core::NodeKind::Table { .. } => tables += 1,
                carrel_core::NodeKind::Image { .. } => images += 1,
                carrel_core::NodeKind::Math => math += 1,
                _ => {}
            }
        }
        let external = self.doc.links.iter().filter(|l| l.contains("://")).count();
        // Distinct local destinations — the forward pane's own rule, so the
        // two can never disagree about what a link is.
        let internal = crate::app::forward_rows(self)
            .iter()
            .filter(|r| r.target.is_some())
            .count();
        let mut rows = vec![
            (
                "document",
                if self.path.is_empty() {
                    "(stdin)".into()
                } else {
                    self.path.clone()
                },
            ),
            ("words", self.words.to_string()),
            (
                "reading time",
                format!("{} min total", self.words.div_ceil(READING_WPM)),
            ),
            ("headings", headings.to_string()),
            ("code blocks", code.to_string()),
            ("tables", tables.to_string()),
            ("images", images.to_string()),
            ("math blocks", math.to_string()),
            ("links", format!("{internal} local · {external} external")),
            (
                "footnotes",
                format!(
                    "{} refs · {} notes",
                    self.doc.footnote_refs().len(),
                    self.doc.footnote_defs().len()
                ),
            ),
            ("tasks", {
                let ts = self.doc.tasks();
                let done = ts.iter().filter(|t| t.done).count();
                format!("{done} of {} done", ts.len())
            }),
            ("details folds", self.doc.details.len().to_string()),
            ("bookmarks", self.marks.len().to_string()),
        ];
        if let Some(t) = self.mtime {
            // Human date, no chrono: the state file's own epoch-seconds habit,
            // formatted as Y-M-D by hand.
            let secs = t
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| u64::min(d.as_secs(), 253_402_300_799));
            rows.push(("last changed", format_epoch(secs)));
        }
        rows
    }

    /// Headings surviving the outline filter, best match first. The same
    /// fuzzy rule the home screen's `refilter` uses, so the two pickers
    /// feel like one; an empty filter is every heading in reading order.
    #[must_use]
    pub fn outline_matches(&self) -> Vec<BlockIdx> {
        let needle = self
            .outline
            .as_ref()
            .map(|o| o.filter.trim().to_lowercase())
            .unwrap_or_default();
        if needle.is_empty() {
            return self.headings();
        }
        let mut scored: Vec<(i32, BlockIdx)> = self
            .headings()
            .into_iter()
            .filter_map(|b| {
                let n = self.doc.node_for_block(b);
                crate::fuzzy::score(
                    &self.doc.text[n.doc.start as usize..n.doc.end as usize],
                    &needle,
                )
                .map(|s| (s, b))
            })
            .collect();
        scored.sort_by_key(|&(rank, _)| std::cmp::Reverse(rank));
        scored.into_iter().map(|(_, b)| b).collect()
    }

    /// The margin outline's rows: every heading, with its level, and whether
    /// it encloses where the reader is.
    ///
    /// Derived per call from the section index, never stored — the same rule
    /// the breadcrumb and folding follow, and the reason a fold or a reload
    /// cannot leave it stale.
    #[must_use]
    pub fn margin_rows(&self) -> Vec<(BlockIdx, u8, bool)> {
        let here = self.doc.section_path(self.view.anchor);
        self.headings()
            .into_iter()
            .map(|b| {
                let n = self.doc.node_for_block(b);
                let level = match n.kind {
                    carrel_core::NodeKind::Heading { level } => level,
                    _ => 1,
                };
                (b, level, here.contains(&n.id))
            })
            .collect()
    }

    /// Which margin-outline row a click at `row` lands on.
    ///
    /// **The inverse of the paint**, and it takes its window from the same
    /// place: `text_y` for the top edge, `text_h` for the height. A hit-test
    /// that re-derived either would drift, and no frame test would see it.
    #[must_use]
    pub fn margin_row_at(&self, col: u16, row: u16) -> Option<BlockIdx> {
        let g = self.gutter_w();
        if g == 0 || col < PAD_LEFT || col >= PAD_LEFT + g {
            return None;
        }
        let top = self.text_y();
        if row < top {
            return None;
        }
        let rows = self.margin_rows();
        let i = usize::from(row - top);
        // The painted window starts at the same offset the painter uses.
        let first = margin_first(&rows, usize::from(self.text_h()));
        rows.get(first + i).map(|(b, _, _)| *b)
    }

    /// Doc position of a link's first visible run, for revealing it.
    fn link_pos(&self, id: LinkId) -> Option<u32> {
        self.doc
            .nodes
            .iter()
            .flat_map(|n| n.inlines.iter())
            .find(|i| i.link == Some(id))
            .map(|i| i.doc.start)
    }

    /// The prose budget — what a paragraph wraps at.
    #[must_use]
    pub fn text_w(&self) -> u16 {
        Self::text_size(
            self.cols,
            self.rows,
            self.hints,
            self.band(),
            self.max_width,
            self.gutter_w(),
        )
        .0
        .max(1)
    }

    /// The bleed budget — what a table, code block, image or math block may
    /// use. Equal to [`Self::text_w`] whenever the measure is not binding.
    #[must_use]
    pub fn bleed_w(&self) -> u16 {
        Self::text_size(
            self.cols,
            self.rows,
            self.hints,
            self.band(),
            self.max_width,
            self.gutter_w(),
        )
        .1
        .max(1)
    }

    #[must_use]
    pub fn text_h(&self) -> u16 {
        Self::text_size(
            self.cols,
            self.rows,
            self.hints,
            self.band(),
            self.max_width,
            self.gutter_w(),
        )
        .2
        .max(1)
    }

    /// Estimated minutes of reading left, or `None` when saying so is noise.
    ///
    /// Derived from the same scroll fraction the percentage uses, so the two
    /// can never disagree about how far in you are. Suppressed below a minute
    /// and at the end of the document: "0 min left" tells a reader nothing,
    /// and a reading desk should be quiet when it has nothing to add.
    #[must_use]
    pub fn minutes_left(&self) -> Option<usize> {
        let max = self.layout.max_scroll(self.text_h());
        if max == 0 || self.words == 0 {
            return None;
        }
        let read = f64::from(self.view.scroll_row.min(max)) / f64::from(max);
        #[allow(clippy::cast_precision_loss, clippy::cast_sign_loss)]
        let left = ((self.words as f64) * (1.0 - read) / (READING_WPM as f64)).round() as usize;
        (left >= 1).then_some(left)
    }

    /// The prose column's left edge for this app's geometry.
    #[must_use]
    pub fn text_x_now(&self) -> u16 {
        Self::text_x(self.cols, self.max_width, self.gutter_w())
    }

    /// A block's horizontal extent: where it paints, and how wide.
    ///
    /// **The single source for both directions.** `render.rs` builds its Rect
    /// from this and [`Self::doc_span_at`] bounds a click by it. They used to
    /// compute it separately, and agreed only for prose: `block_area` painted
    /// tables, code, math and images against the BLEED column and re-centred a
    /// wide table, while the hit-test bounded every click by the PROSE column.
    /// Whenever bleed exceeded the measure — that is, at 95 columns or wider
    /// with the default 90-column measure, so any maximized terminal — a click
    /// on a wide table resolved thirteen columns to the left of what was
    /// painted, and clicks on its outer thirds were rejected outright.
    /// Selection, double-click-word, triple-click-block and link-follow were
    /// all wrong together.
    ///
    /// Returns columns in screen space, already clamped to the text area.
    #[must_use]
    pub fn block_span_x(&self, block: BlockIdx) -> (u16, u16) {
        let full_x = PAD_LEFT;
        let full_w = self.bleed_w();
        let full_right = full_x.saturating_add(full_w);
        let prose_x = self.text_x_now();
        let node = self.doc.node_for_block(block);
        let budget = self.layout.block_width(&node.kind);
        if budget <= self.text_w() {
            // Bound to the measure: the fixed column.
            return (prose_x, budget.min(full_w));
        }
        // A bleed kind. Centre a table by the width it actually occupies.
        let x = match &node.kind {
            carrel_core::NodeKind::Table { cols, .. } if !cols.is_empty() => {
                let aligned = cols.iter().map(|&c| u32::from(c)).sum::<u32>()
                    + 3 * (cols.len() as u32 - 1)
                    + u32::from(node.indent);
                let aligned = u16::try_from(aligned).unwrap_or(u16::MAX).min(full_w);
                if aligned > self.text_w() {
                    full_x + (full_w - aligned) / 2
                } else {
                    prose_x
                }
            }
            _ => prose_x,
        };
        // Never run off the right edge, and never start left of the text area.
        let x = x.max(full_x).min(full_right.saturating_sub(1));
        (x, full_right - x)
    }

    /// The `(start, end)` doc bytes of the grapheme cluster under a pointer
    /// cell, or `None` outside the text area / off the end of the content.
    ///
    /// **The one place a screen position becomes a byte** — everything
    /// downstream is doc space, which is what lets a selection survive a
    /// resize. It hit-tests against [`Self::block_span_x`], the same function
    /// `render.rs` paints from; if the two ever diverge, every click lands on
    /// the wrong byte and no frame test narrower than the measure can see it.
    #[must_use]
    pub fn doc_span_at(&self, col: u16, row: u16) -> Option<(u32, u32)> {
        let top = self.text_y();
        if row < top || row >= top + self.text_h() {
            return None;
        }
        let vrow = self.view.scroll_row + u32::from(row - top);
        let block = self.layout.block_at_row(vrow);
        if block.get() >= self.doc.block_count() {
            return None;
        }
        // The BLOCK's column, not the prose column: a wide table paints
        // outside the measure, and a click on it must resolve against the
        // geometry it was actually painted with. See `block_span_x`.
        let (block_x, block_w) = self.block_span_x(block);
        if col < block_x || col >= block_x.saturating_add(block_w) {
            return None;
        }
        let mut rows = Vec::new();
        self.layout.rows_for(&self.doc, block, &mut rows);
        let sub = usize::try_from(vrow.saturating_sub(self.layout.row_start(block))).ok()?;
        let r = rows.get(sub)?;
        if r.doc.is_empty() {
            return None;
        }
        let text = self
            .doc
            .text
            .get(r.doc.start as usize..r.doc.end as usize)?;
        Some(carrel_core::cluster_at_col(
            text,
            r.doc.start,
            r.indent,
            col - block_x,
        ))
    }

    #[must_use]
    pub fn searching(&self) -> bool {
        matches!(self.mode, Mode::Search { .. })
    }

    /// §3.5: rebuild the derived layer, then restore position from the anchor.
    pub fn on_resize(&mut self, cols: u16, rows: u16) {
        self.cols = cols;
        self.rows = rows;
        self.relayout();
    }

    /// Rebuild layout — including image row-heights recomputed from stored
    /// dimensions at the current width — and restore the reading position.
    ///
    /// Lay out every math block in the document, both forms, once.
    ///
    /// A block whose LaTeX will not parse is simply absent from the map, which
    /// is the literal-source fallback: a reader must never show a parse error
    /// where the document expected an equation.
    fn rebuild_math_art(&mut self) {
        self.math_art.clear();
        for i in 0..self.doc.block_count() {
            let b = BlockIdx(i as u32);
            let node = self.doc.node_for_block(b);
            if !matches!(node.kind, NodeKind::Math) {
                continue;
            }
            let src = &self.doc.text[node.doc.start as usize..node.doc.end as usize];
            let Some(expr) = carrel_core::math::parse(src) else {
                continue;
            };
            // Take the honest refusal rather than the encoded one: art over
            // the size or depth budget is the same outcome as art that would
            // not parse — absent from the map, rendered as literal source.
            // `lay_out` stays total for callers that want a box regardless,
            // but it has to encode "no fit" as a `u16::MAX`-wide blank, and
            // this is the one caller that can just decline.
            let (Some(display), Some(inline)) = (
                math_art::try_lay_out(&expr, math_art::Mode::Display),
                math_art::try_lay_out(&expr, math_art::Mode::Inline),
            ) else {
                continue;
            };
            self.math_art.insert(b, MathArt { display, inline });
        }
    }

    /// Which form of a math block fits `avail` display columns.
    ///
    /// The ladder, in order: display art -> the inline single-row form -> the
    /// literal LaTeX source. Nothing is ever clipped into nonsense, and the
    /// fallback is a designed outcome rather than an error path.
    #[must_use]
    pub fn math_form(&self, block: BlockIdx, avail: u16) -> MathForm {
        let Some(art) = self.math_art.get(&block) else {
            return MathForm::Source;
        };
        if !self.show_rendered {
            return MathForm::Source;
        }
        if art.display.width <= avail {
            MathForm::Display
        } else if art.inline.width <= avail {
            MathForm::Inline
        } else {
            MathForm::Source
        }
    }

    /// Row count for a math block, or `None` when it renders as source and so
    /// takes its height from ordinary text wrapping.
    ///
    /// Paint MUST read `math_form` too, or height and paint disagree -- the
    /// same reason `with_images` runs the row pass's own functions.
    fn math_rows(&self, block: BlockIdx, avail: u16) -> Option<u32> {
        let art = self.math_art.get(&block)?;
        match self.math_form(block, avail) {
            MathForm::Display => Some(u32::try_from(art.display.rows.len()).unwrap_or(u32::MAX)),
            MathForm::Inline => Some(1),
            MathForm::Source => None,
        }
    }

    /// **Image dimension arrival is just another reflow.** The anchor
    /// machinery that keeps a resize stable keeps this stable too; nothing
    /// about it is a special case.
    pub fn relayout(&mut self) {
        // Images, diagrams and math art are *bleed* kinds — their width is
        // intrinsic to the content, not to the reading measure — so they size
        // against the full budget. Prose alone binds to the measure.
        let w = self.bleed_w();
        let mut block_rows: HashMap<BlockIdx, u32> = self
            .image_dims
            .iter()
            .map(|(b, px)| {
                let indent = self.doc.node_for_block(*b).indent;
                let avail = w.saturating_sub(indent).max(1);
                (*b, crate::images::rows_for_dims(*px, self.font_px, avail))
            })
            .collect();
        // A rendered diagram is, layout-wise, an image whose rows happen to
        // be text: same height-override channel, same reflow machinery.
        if self.show_rendered {
            for (b, art) in &self.diagram_art {
                block_rows.insert(*b, u32::try_from(art.len()).unwrap_or(u32::MAX));
            }
            // Math art rides the same channel. Which FORM is chosen depends on
            // the width, but neither form is recomputed here -- see math_form.
            for b in self.math_art.keys() {
                let indent = self.doc.node_for_block(*b).indent;
                let avail = w.saturating_sub(indent);
                if let Some(rows) = self.math_rows(*b, avail) {
                    block_rows.insert(*b, rows);
                }
            }
        }
        self.layout = Layout::with_hidden(
            &self.doc,
            w,
            self.text_w(),
            block_rows,
            self.wrap_tables,
            &self.hidden_blocks(),
        );
        self.view.restore(&self.doc, &self.layout, self.text_h());
        // Matches and `matches.current` are untouched. That is the whole point.
    }

    fn rerun_search(&mut self) {
        let Mode::Search { input, .. } = &self.mode else {
            return;
        };
        if input.is_empty() {
            self.matches = None;
            return;
        }
        // `flexible_ws` is on: someone searching a phrase they can SEE must
        // match across the author's hard-wrapped source line.
        self.matches = Some(search(&self.doc, input, true));
    }
}

/// Refuse documents whose byte offsets cannot fit the position type.
///
/// Every position in this system is a `u32` byte offset **by design**
/// (architecture.md (private notes repo) §1.3); a ≥ 4 GiB file would silently truncate
/// every offset and corrupt search, layout, and provenance. Refusing loudly
/// beats corrupting quietly — and a 4 GiB "markdown file" is not a document,
/// it is a mistake.
pub fn check_document_size(path: &Path) -> std::io::Result<()> {
    let meta = std::fs::metadata(path)?;
    // A FIFO, /dev/zero and most of /proc all report length zero, so the
    // size guard passed and the unbounded read behind it ran anyway. Reading
    // a FIFO blocked FOREVER with the terminal already in raw mode, the
    // alternate screen up and the event loop stalled — no keystroke could
    // reach it, and killing it from another terminal used to leave the
    // terminal wrecked as well. `/dev/zero` read until the OOM killer came.
    // A markdown link is untrusted input, so `[x](./pipe.md)` was enough.
    if !meta.is_file() {
        return Err(std::io::Error::other(
            "not a regular file — carrel reads documents, and a pipe or \
             device would block the reader with no way out",
        ));
    }
    let len = meta.len();
    if len >= u64::from(u32::MAX) {
        return Err(std::io::Error::other(format!(
            "file is {len} bytes; carrel documents are limited to 4 GiB \
             because positions are 32-bit byte offsets",
        )));
    }
    Ok(())
}

/// Read a document, enforcing the cap with the READ rather than with a stat
/// taken before it.
///
/// `check_document_size` measures and then something else reads, which leaves
/// a window: a file that grows past 4 GiB in between, or one whose length was
/// never meaningful in the first place. `take` closes both — the same shape
/// `read_stdin_capped` in the binary already uses, whose comment claimed it
/// did "exactly as `check_document_size` does for files" before that was true.
pub fn read_document(path: &Path) -> std::io::Result<String> {
    use std::io::Read as _;
    check_document_size(path)?;
    let mut src = String::new();
    std::fs::File::open(path)?
        .take(u64::from(u32::MAX))
        .read_to_string(&mut src)?;
    Ok(src)
}

/// The one transition. Pure state, no drawing and no terminal.
///
/// The first margin-outline row to paint, so the current section stays on
/// screen in a document with more headings than rows.
///
/// Paint and hit-testing both call this — one derivation, both ways.
#[must_use]
pub fn margin_first(rows: &[(BlockIdx, u8, bool)], height: usize) -> usize {
    if rows.len() <= height {
        return 0;
    }
    let cur = rows.iter().rposition(|(_, _, here)| *here).unwrap_or(0);
    // Keep the current section roughly centred once the list scrolls.
    let half = height / 2;
    cur.saturating_sub(half).min(rows.len() - height)
}

/// The backlinks pane's state: what has arrived, and where the cursor is.
#[derive(Debug, Default)]
pub struct Backlinks {
    pub rows: Vec<crate::links::Backlink>,
    pub selected: usize,
    /// The query finished — the difference between "none yet" and "none".
    pub done: bool,
}

/// One row of the forward-links pane: a destination this document points
/// at. `target` is `Some` when it resolves to a local file — wikilinks
/// through the same resolver the reader uses, relative links against the
/// document's own directory — and `None` for anything external, which a
/// reader that never fetches will show but not open.
#[derive(Clone, Debug)]
pub struct ForwardRow {
    pub dest: String,
    /// The link's visible text, when it has one and differs from the dest.
    pub label: Option<String>,
    pub target: Option<PathBuf>,
}

/// The forward-links pane: derived in one pass at open time — every link is
/// already in memory, so unlike backlinks there is nothing to stream.
#[derive(Debug, Default)]
pub struct Forward {
    pub rows: Vec<ForwardRow>,
    pub selected: usize,
}

/// Parse, adapting a raw diff into markdown first when this document is
/// allowed to be one.
///
/// **The one place the diff policy lives.** `diff_ok` is set by whoever
/// opened the document: true for a pipe and for `.diff`/`.patch`, false for a
/// `.md` file — so a markdown document *about* diffs can never be mangled by
/// the sniffer, which is the entire safety argument. `--diff` / `--no-diff`
/// override it for the run.
#[must_use]
pub fn adapt(src: &str, diff_ok: bool) -> Document {
    if diff_ok && carrel_core::looks_like_diff(src) {
        Document::parse(&carrel_core::to_markdown(src))
    } else {
        Document::parse(src)
    }
}

/// The one exception to "no I/O" is [`Action::HomeOpen`], which must read the
/// file it is opening. Everything else is arithmetic over state.
pub fn update(app: &mut App, action: Action) -> Outcome {
    // The lamp switch works from ANY state — hiding the hints must never
    // require first backing out of whatever you were doing.
    if let Action::HintsToggle = action {
        app.hints = !app.hints;
        if let Some(dir) = app.config_dir.as_deref() {
            let _ = crate::config::save_hints_in(dir, app.hints);
        }
        app.relayout(); // the reader's text height changes with the row
        return Outcome::Redraw;
    }
    if let Action::BreadcrumbToggle = action {
        app.breadcrumb = !app.breadcrumb;
        if let Some(dir) = app.config_dir.as_deref() {
            let _ = crate::config::save_breadcrumb_in(dir, app.breadcrumb);
        }
        app.relayout(); // the band's rows come from / return to the text
        return Outcome::Redraw;
    }
    // The info card is passive — it never owns the keyboard — but `I`
    // reaches it from any reader state, the way `H` and `B` do. So does the
    // spotlight, its neighbour in the capital-toggle row.
    if !app.is_home() {
        match action {
            Action::InfoToggle => {
                app.info = !app.info;
                return Outcome::Redraw;
            }
            Action::FocusToggle => {
                app.focus = !app.focus;
                return Outcome::Redraw;
            }
            Action::AutoToggle => {
                app.auto_read = !app.auto_read;
                app.note = Some(if app.auto_read {
                    "auto-read on — any motion stops it".into()
                } else {
                    "auto-read off".into()
                });
                return Outcome::Redraw;
            }
            _ => {}
        }
    }
    // The help overlay owns the keyboard while it is up: scroll scrolls the
    // sheet, dismiss-shaped actions close it, everything else is inert — a
    // stray keystroke must not navigate the document underneath.
    if let Some(scroll) = app.help {
        return match action {
            Action::HelpToggle | Action::Dismiss | Action::CloseFile => {
                app.help = None;
                Outcome::Redraw
            }
            Action::Scroll(_, n) => {
                app.help = Some(if n < 0 {
                    scroll.saturating_sub(u16::try_from(n.unsigned_abs()).unwrap_or(u16::MAX))
                } else {
                    scroll.saturating_add(u16::try_from(n.unsigned_abs()).unwrap_or(u16::MAX))
                });
                Outcome::Redraw
            }
            Action::GoToStart => {
                app.auto_read = false;
                app.help = Some(0);
                Outcome::Redraw
            }
            Action::Quit => Outcome::Quit,
            _ => Outcome::Idle,
        };
    }
    // The outline picker owns the keyboard the same way (help wins when
    // both would apply — it is bound in outline mode's key set as nothing,
    // so the case cannot arise from keys, only from synthetic actions).
    if app.outline.is_some() {
        return outline_update(app, action);
    }
    if app.is_home() {
        return home_update(app, action);
    }
    reader_update(app, action)
}

/// Transitions while the outline picker is up.
fn outline_update(app: &mut App, action: Action) -> Outcome {
    match action {
        Action::OutlineToggle => {
            app.outline = None;
            Outcome::Redraw
        }
        Action::OutlineMove(n) => {
            let last = app.outline_matches().len().saturating_sub(1);
            if let Some(o) = app.outline.as_mut() {
                o.selected = if n < 0 {
                    o.selected.saturating_sub(n.unsigned_abs() as usize)
                } else {
                    o.selected.saturating_add(n.unsigned_abs() as usize)
                }
                .min(last);
            }
            Outcome::Redraw
        }
        Action::OutlineKey(k) => {
            match k {
                SearchKey::Char(c) => {
                    if let Some(o) = app.outline.as_mut() {
                        o.filter.push(c);
                    }
                }
                SearchKey::Backspace => {
                    if let Some(o) = app.outline.as_mut() {
                        o.filter.pop();
                    }
                }
                SearchKey::Accept => return outline_update(app, Action::OutlineJump),
                // Two-stage escape, exactly like the home filter.
                SearchKey::Cancel => {
                    let empty = app.outline.as_ref().is_none_or(|o| o.filter.is_empty());
                    if empty {
                        app.outline = None;
                    } else if let Some(o) = app.outline.as_mut() {
                        o.filter.clear();
                    }
                    return Outcome::Redraw;
                }
            }
            // The list shrank or grew: keep the selection on it.
            let last = app.outline_matches().len().saturating_sub(1);
            if let Some(o) = app.outline.as_mut() {
                o.selected = o.selected.min(last);
            }
            Outcome::Redraw
        }
        // The picker owns the keyboard while it is up, and `update` routes
        // every action here — so the click's action belongs here too, beside
        // the jump it delegates to, not in `reader_update` where it can never
        // arrive.
        Action::OutlineJumpAt(i) => {
            let i = i as usize;
            if i >= app.outline_matches().len() {
                return Outcome::Idle;
            }
            let Some(o) = app.outline.as_mut() else {
                return Outcome::Idle;
            };
            o.selected = i;
            outline_update(app, Action::OutlineJump)
        }
        Action::OutlineJump => {
            let matches = app.outline_matches();
            let Some(block) = app
                .outline
                .as_ref()
                .and_then(|o| matches.get(o.selected).copied())
            else {
                return Outcome::Idle;
            };
            app.outline = None;
            // A jump is a link follow in spirit: Ctrl-O comes back.
            if let Some(here) = app.file.clone() {
                app.push_history(here, app.view.anchor);
            }
            let byte = app.doc.node_for_block(block).doc.start;
            let h = app.text_h();
            app.reveal_byte(byte, h, Where::Top);
            Outcome::Redraw
        }
        Action::Quit => Outcome::Quit,
        _ => Outcome::Idle,
    }
}

/// Every home action, then one scroll clamp.
///
/// The clamp lives here rather than in the arms because an arm that forgot it
/// would leave the selection off screen with nothing to point at — and there
/// are a dozen arms that move a selection.
fn home_update(app: &mut App, action: Action) -> Outcome {
    let resume = app.home().map_or(0, crate::home::Home::resume_shown);
    let (_, list_h) = crate::home::list_geometry(app.cols, app.rows, app.hints, resume);
    let (cols, rows) = (app.cols, app.rows);
    let out = home_action(app, action);
    if let Some(h) = app.home_mut() {
        h.clamp_scroll(usize::from(list_h));
        h.clamp_picker_scroll(cols, rows);
    }
    out
}

#[allow(clippy::too_many_lines)]
fn home_action(app: &mut App, action: Action) -> Outcome {
    match action {
        Action::Quit => return Outcome::Quit,

        Action::HomeOpen => {
            // In content-search mode Enter opens the selected HIT, with the
            // query as the live in-document search, landed on match one.
            let search_query = app.home().and_then(|h| {
                (h.mode == HomeMode::Search).then(|| {
                    (
                        h.hits.get(h.hit_selected).map(|hit| hit.path.clone()),
                        h.query.clone(),
                    )
                })
            });
            if let Some((hit_path, query)) = search_query {
                let Some(path) = hit_path else {
                    return Outcome::Idle;
                };
                return match app.open_path(&path) {
                    Ok(()) => {
                        app.matches = Some(search(&app.doc, &query, true));
                        return update(app, Action::MatchStep(1));
                    }
                    Err(e) => {
                        if let Some(h) = app.home_mut() {
                            h.note = Some(format!("cannot open {}: {e}", path.display()));
                        }
                        Outcome::Redraw
                    }
                };
            }
            let Some(path) = app
                .home()
                .and_then(|h| h.selected_path().map(Path::to_path_buf))
            else {
                return Outcome::Idle;
            };
            return match app.open_path(&path) {
                Ok(()) => Outcome::Redraw,
                // Staying on the home screen with a note beats dropping the
                // user into an empty reader wondering what happened.
                Err(e) => {
                    if let Some(h) = app.home_mut() {
                        h.note = Some(format!("cannot open {}: {e}", path.display()));
                    }
                    Outcome::Redraw
                }
            };
        }

        // A click in the picker. Clamped, so an index from a frame that has
        // since changed can never point past the last match.
        Action::PickerSelect(i) => {
            if let Some(h) = app.home_mut() {
                h.picker.selected = i.min(h.picker.roots.len().saturating_sub(1));
            }
            return Outcome::Redraw;
        }
        Action::PickerChoose => {
            // Enter follows the HIGHLIGHT, and falls back to the typed path
            // only when nothing matched it — so a path typed in full still
            // works even where its parent cannot be listed.
            let Some(root) = app.home().and_then(|h| {
                let p = &h.picker;
                p.roots.get(p.selected).cloned().or_else(|| {
                    Some(p.typed.trim())
                        .filter(|t| !t.is_empty())
                        .map(crate::home::expand_typed)
                })
            }) else {
                return Outcome::Idle;
            };
            if !root.is_dir() {
                if let Some(h) = app.home_mut() {
                    h.note = Some(format!("not a directory: {}", root.display()));
                }
                return Outcome::Redraw;
            }
            // Persisting here is the whole point of the picker: it is how a
            // choice becomes the default — and a place. A failed write is not
            // worth blocking the user over — the choice still applies to this
            // run. Tests run with no `config_dir`, so they can never write the
            // real file.
            let saved = match app.config_dir.as_deref() {
                Some(dir) => crate::config::save_root_in(dir, &root)
                    .and_then(|()| crate::config::add_place_in(dir, &root)),
                None => Ok(()),
            };
            if let Some(h) = app.home_mut() {
                // The menu reflects the new favourite immediately.
                h.places.retain(|p| p != &root);
                h.places.insert(0, root.clone());
                h.places.truncate(crate::config::PLACE_CAP);
            }
            let cached = crate::scan::load_cache(&root);
            if let Some(h) = app.home_mut() {
                h.set_root(root, cached);
                if let Err(e) = saved {
                    h.note = Some(format!("could not save the default: {e}"));
                }
            }
            return Outcome::Redraw;
        }

        Action::HomeResume(i) => {
            let Some(path) = app
                .home()
                .and_then(|h| h.resume.get(i))
                .map(|r| r.path.clone())
            else {
                return Outcome::Idle;
            };
            return match app.open_path(&path) {
                Ok(()) => Outcome::Redraw,
                Err(e) => {
                    if let Some(h) = app.home_mut() {
                        h.note = Some(format!("cannot open {}: {e}", path.display()));
                    }
                    Outcome::Redraw
                }
            };
        }

        Action::HelpToggle => {
            app.help = Some(0);
            return Outcome::Redraw;
        }

        Action::PickerOpen => {
            // Opens on the directory `carrel` was run from — `App::launch_dir`
            // — so what you type continues from here instead of from `/`, and
            // the highlight is already on "here" before a single keystroke.
            //
            // It used to open on the home screen's ROOT with the remembered
            // places leading the list, which meant that with a saved `root =`
            // in the config the first thing `d` offered was the last directory
            // you read in, not the one you had just `cd`-ed to (maintainer
            // report, 2026-09-01). Places are the EMPTY menu's job now, the
            // rule the typing arm already followed: one Esc clears the prefill
            // and brings them back.
            //
            // The launch directory leads its own children rather than being
            // merely typed above them, because `directory_matches` reads a
            // trailing slash as "list what is INSIDE" — without this row the
            // path in the input would be the one thing Enter could not choose.
            let start = app
                .launch_dir
                .clone()
                .or_else(|| app.home().map(|h| h.root.clone()));
            let prefill = start
                .as_deref()
                .map(crate::home::picker_prefill)
                .unwrap_or_default();
            let lead = if prefill.is_empty() {
                // Nowhere to continue from: fall back to the remembered places
                // ahead of the default menu, which is what `d` always did.
                app.home().map(|h| h.places.clone()).unwrap_or_default()
            } else {
                start.into_iter().collect()
            };
            let mut roots = merge_places(lead, Home::matches_for(&prefill));
            roots.dedup();
            if let Some(h) = app.home_mut() {
                h.picker.roots = roots;
                h.picker.selected = 0;
                h.picker.top = 0;
                h.picker.typed = prefill;
                h.mode = HomeMode::Picker;
            }
            return Outcome::Redraw;
        }

        _ => {}
    }

    let Some(h) = app.home_mut() else {
        return Outcome::Idle;
    };

    match action {
        Action::HomeSearchMode => {
            h.mode = HomeMode::Search;
            h.query.clear();
            h.hits.clear();
            h.hit_selected = 0;
            h.grep_done = false;
            Outcome::Redraw
        }
        // A click. `Home::select` clamps and routes by mode, so an index from
        // a frame that has since changed cannot put the selection out of range.
        Action::HomeSelect(i) => {
            h.select(i);
            Outcome::Redraw
        }
        Action::HomeMove(n) if h.mode == HomeMode::Search => {
            let last = h.hits.len().saturating_sub(1);
            h.hit_selected = if n < 0 {
                h.hit_selected.saturating_sub(n.unsigned_abs() as usize)
            } else {
                h.hit_selected.saturating_add(n.unsigned_abs() as usize)
            }
            .min(last);
            Outcome::Redraw
        }
        Action::HomeKey(k) if h.mode == HomeMode::Search => {
            match k {
                SearchKey::Char(c) => {
                    h.query.push(c);
                    h.hits.clear();
                    h.hit_selected = 0;
                    h.grep_done = false;
                }
                SearchKey::Backspace => {
                    h.query.pop();
                    h.hits.clear();
                    h.hit_selected = 0;
                    h.grep_done = false;
                }
                SearchKey::Accept => return Outcome::Idle,
                // Two-stage escape, same as the filter.
                SearchKey::Cancel => {
                    if h.query.is_empty() {
                        h.mode = HomeMode::Normal;
                        h.hits.clear();
                    } else {
                        h.query.clear();
                        h.hits.clear();
                        h.hit_selected = 0;
                    }
                }
            }
            Outcome::Redraw
        }
        Action::HomeMove(n) => {
            if h.mode == HomeMode::Picker {
                let last = h.picker.roots.len().saturating_sub(1);
                h.picker.selected = if n < 0 {
                    h.picker.selected.saturating_sub(n.unsigned_abs() as usize)
                } else {
                    h.picker
                        .selected
                        .saturating_add(n.unsigned_abs() as usize)
                        .min(last)
                };
            } else {
                h.move_by(n);
            }
            Outcome::Redraw
        }
        Action::HomeGo(Edge::First) => {
            h.go_first();
            Outcome::Redraw
        }
        Action::HomeGo(Edge::Last) => {
            h.go_last();
            Outcome::Redraw
        }
        // Cancelling the picker also lands here: it opens from the menu
        // (`d` is a normal-mode key), so Esc returns to the menu — genuinely
        // the same transition.
        Action::HomeNormalMode | Action::PickerCancel => {
            h.mode = HomeMode::Normal;
            Outcome::Redraw
        }
        Action::HomeFilterMode => {
            h.mode = HomeMode::Filter;
            Outcome::Redraw
        }
        // In the picker, keystrokes belong to the picker alone — NOTHING may
        // fall through to the filter hidden behind the overlay, a corruption
        // that used to stay invisible until Esc. Typing edits the path and
        // re-lists the directories that match it.
        Action::HomeKey(k) if h.mode == HomeMode::Picker => {
            match k {
                SearchKey::Char(c) => h.picker.typed.push(c),
                SearchKey::Backspace => {
                    h.picker.typed.pop();
                }
                SearchKey::Accept => return Outcome::Idle,
                // Two-stage escape, the same shape as the filter's: clear the
                // path first, close the picker only when there is nothing
                // left to clear.
                SearchKey::Cancel => {
                    if h.picker.typed.is_empty() {
                        h.mode = HomeMode::Normal;
                        return Outcome::Redraw;
                    }
                    h.picker.typed.clear();
                }
            }
            let typed = h.picker.typed.clone();
            // Places lead the empty menu; once something is typed the
            // filesystem's own completions take over.
            let places = if typed.is_empty() {
                h.places.clone()
            } else {
                Vec::new()
            };
            let mut roots = merge_places(places, Home::matches_for(&typed));
            roots.dedup();
            let h = app.home_mut().expect("home screen");
            h.picker.roots = roots;
            h.picker.selected = 0;
            h.picker.top = 0;
            Outcome::Redraw
        }
        Action::HomeKey(k) => {
            match k {
                SearchKey::Char(c) => {
                    h.filter.push(c);
                    h.refilter();
                }
                SearchKey::Backspace => {
                    h.filter.pop();
                    h.refilter();
                }
                SearchKey::Accept => return Outcome::Idle,
                SearchKey::Cancel => {
                    // Two-stage escape: clear the filter first, change mode only
                    // when there is nothing left to clear.
                    if h.filter.is_empty() {
                        h.mode = HomeMode::Normal;
                    } else {
                        h.filter.clear();
                        h.refilter();
                    }
                }
            }
            Outcome::Redraw
        }
        // Reader-only actions cannot corrupt home state.
        _ => Outcome::Idle,
    }
}

#[allow(clippy::too_many_lines)]
fn reader_update(app: &mut App, action: Action) -> Outcome {
    let h = app.text_h();
    // Notes are one-shot: whatever happens next replaces or clears them.
    app.note = None;
    match action {
        Action::Quit => {
            app.save_position();
            Outcome::Quit
        }

        Action::CloseFile => close_file(app),

        Action::ScrollTo(row) => {
            app.view.scroll_to(&app.doc, &app.layout, row, h);
            Outcome::Redraw
        }

        Action::FoldToggle => {
            let Some(target) = app.fold_target() else {
                app.note = Some("no section here to fold".into());
                return Outcome::Redraw;
            };
            match target {
                FoldTarget::Section(id) => {
                    if !app.folded.remove(&id) {
                        app.folded.insert(id);
                    }
                }
                FoldTarget::Details(i) => {
                    if !app.folded_details.remove(&i) {
                        app.folded_details.insert(i);
                    }
                }
            }
            app.after_fold_change();
            Outcome::Redraw
        }
        Action::FoldAll => {
            app.folded = app
                .doc
                .nodes
                .iter()
                .filter(|n| matches!(n.kind, carrel_core::NodeKind::Heading { .. }))
                .map(|n| n.id)
                .collect();
            app.folded_details =
                (0..u32::try_from(app.doc.details.len()).unwrap_or(u32::MAX)).collect();
            app.after_fold_change();
            Outcome::Redraw
        }
        Action::UnfoldAll => {
            app.folded.clear();
            app.folded_details.clear();
            app.after_fold_change();
            Outcome::Redraw
        }

        Action::Dismiss => {
            app.selected_link = None;
            app.selection = None;
            app.sel_anchor = None;
            // …and the highlights an ACCEPTED search left behind. Cancelling
            // mid-typing goes through `SearchKey::Cancel`, which also restores
            // the pre-search position; this does not, and must not — accepting
            // a search moved the reader somewhere on purpose, so clearing the
            // highlights afterwards is not an undo.
            app.matches = None;
            Outcome::Redraw
        }

        Action::SelectAnchor(cluster) => {
            app.selection = None;
            app.sel_anchor = Some(cluster);
            Outcome::Redraw
        }
        Action::SelectDrag(cluster) => {
            let Some(anchor) = app.sel_anchor else {
                return Outcome::Idle;
            };
            let start = anchor.0.min(cluster.0);
            let end = anchor.1.max(cluster.1);
            app.selection = Some(start..end);
            Outcome::Redraw
        }
        Action::SelectRelease => {
            let pressed = app.sel_anchor.take();
            // A completed click — press and release with no drag, so no
            // selection formed — on a heading toggles its fold. A fold
            // marker looks clickable; the home list taught us what happens
            // when things look clickable and are not.
            if app.selection.is_none()
                && let Some((byte, _)) = pressed
                && fold_at(app, byte)
            {
                return Outcome::Redraw;
            }
            copy_selection(app);
            Outcome::Redraw
        }
        // The marker itself, which paints two columns left of the text and so
        // was never inside any clickable span. Same resolution as a click on
        // the row: the two must agree, or the glyph and the words beside it
        // would fold different things.
        Action::FoldAt(byte) => {
            if fold_at(app, byte) {
                Outcome::Redraw
            } else {
                Outcome::Idle
            }
        }
        Action::SelectWord(byte) => {
            app.selection = word_range_at(&app.doc.text, byte);
            Outcome::Redraw
        }
        Action::SelectBlock(byte) => {
            let node = app.doc.node_for_block(app.doc.block_at_doc(DocByte(byte)));
            app.selection = (!node.doc.is_empty()).then(|| node.doc.clone());
            Outcome::Redraw
        }

        Action::LinkStep(n) => link_step(app, n, h),

        Action::LinkFollow => {
            app.following = false;
            link_follow(app)
        }
        // A click does in one intent what Tab-then-Enter does in two: the
        // pointer already said which link it meant, so selecting it and
        // following it is one gesture.
        Action::LinkOpen(i) => {
            if app.doc.links.get(i as usize).is_none() {
                return Outcome::Idle;
            }
            app.following = false;
            app.selected_link = Some(LinkId(i));
            link_follow(app)
        }

        Action::Back => go_back(app, h),

        // A bookmark lands on the block the reader is looking at, not on a
        // raw scroll offset: `zz` and a resize both move the offset, and a
        // mark that drifted off the thing it marked would be useless.
        Action::MarkToggle => {
            let block = app.layout.block_at_row(app.view.scroll_row);
            let at = app.doc.node_for_block(block).doc.start;
            if let Some(i) = app.marks.iter().position(|&m| m == at) {
                app.marks.remove(i);
                app.note = Some("bookmark cleared".into());
            } else {
                app.marks.push(at);
                app.marks.sort_unstable();
                let n = app.marks.iter().position(|&m| m == at).unwrap_or(0) + 1;
                app.note = Some(format!("bookmark {n} of {}", app.marks.len()));
            }
            app.save_marks();
            Outcome::Redraw
        }

        // The bookmark list: `'` walks blind, `"` shows all. Rows are
        // derived from `marks` at every use — nothing to go stale.
        Action::MarkListToggle => {
            if app.mark_list.is_some() {
                app.mark_list = None;
            } else if app.marks.is_empty() {
                app.note = Some("no bookmarks — press m to set one".into());
                return Outcome::Redraw;
            } else {
                // Land on the first mark at or after the reader, the way the
                // outline picker pre-selects the current section.
                let here = app
                    .doc
                    .node_for_block(app.layout.block_at_row(app.view.scroll_row))
                    .doc
                    .start;
                let sel = app
                    .marks
                    .iter()
                    .position(|&m| m >= here)
                    .unwrap_or(app.marks.len().saturating_sub(1));
                app.mark_list = Some(sel);
            }
            Outcome::Redraw
        }
        Action::MarkListMove(n) => {
            if let Some(sel) = app.mark_list
                && !app.marks.is_empty()
            {
                let last = app.marks.len().saturating_sub(1);
                app.mark_list = Some(if n < 0 {
                    sel.saturating_sub(n.unsigned_abs() as usize)
                } else {
                    sel.saturating_add(n.unsigned_abs() as usize).min(last)
                });
            }
            Outcome::Redraw
        }
        Action::MarkListJump => {
            let Some(&at) = app.mark_list.and_then(|sel| app.marks.get(sel)) else {
                return Outcome::Idle;
            };
            app.mark_list = None;
            if let Some(from) = app.file.clone() {
                app.push_history(from, app.view.anchor);
            }
            app.reveal_byte(at, h, crate::action::Where::Top);
            Outcome::Redraw
        }

        Action::MarkNext => {
            if app.marks.is_empty() {
                app.note = Some("no bookmarks — press m to set one".into());
                return Outcome::Redraw;
            }
            let here = app
                .doc
                .node_for_block(app.layout.block_at_row(app.view.scroll_row))
                .doc
                .start;
            // The next one strictly after where we are, wrapping — the only
            // behaviour that lets `'` walk a list of any length.
            let i = app.marks.iter().position(|&m| m > here).unwrap_or(0);
            let at = app.marks[i];
            app.reveal_byte(at, h, Where::Top);
            app.note = Some(format!("bookmark {} of {}", i + 1, app.marks.len()));
            Outcome::Redraw
        }

        // The margin outline's click. Same destination as the outline
        // picker's jump, through the same reveal gate.
        // Backlinks: who points here. Opening starts a query; the event
        // loop streams rows in. Toggling closes it.
        Action::FootnoteJump => footnote_jump(app, h),

        Action::BacklinksToggle => {
            if app.backlinks.is_some() {
                app.backlinks = None;
            } else if app.file.is_none() {
                app.note = Some("a piped document has no path to link to".into());
            } else {
                app.backlinks = Some(Backlinks::default());
            }
            Outcome::Redraw
        }
        Action::BacklinksMove(n) => {
            if let Some(b) = app.backlinks.as_mut() {
                let last = b.rows.len().saturating_sub(1);
                b.selected = if n < 0 {
                    b.selected.saturating_sub(n.unsigned_abs() as usize)
                } else {
                    b.selected
                        .saturating_add(n.unsigned_abs() as usize)
                        .min(last)
                };
            }
            Outcome::Redraw
        }
        // A click already said which row; selecting it and opening it is one
        // gesture, and the open itself is the handler that is already tested.
        Action::BacklinksOpenAt(i) => {
            let i = i as usize;
            let Some(b) = app.backlinks.as_mut() else {
                return Outcome::Idle;
            };
            if i >= b.rows.len() {
                return Outcome::Idle;
            }
            b.selected = i;
            update(app, Action::BacklinksOpen)
        }
        Action::ForwardOpenAt(i) => {
            let i = i as usize;
            let Some(f) = app.forward.as_mut() else {
                return Outcome::Idle;
            };
            if i >= f.rows.len() {
                return Outcome::Idle;
            }
            f.selected = i;
            update(app, Action::ForwardOpen)
        }
        Action::MarkListJumpAt(i) => {
            let i = i as usize;
            if app.mark_list.is_none() || i >= app.marks.len() {
                return Outcome::Idle;
            }
            app.mark_list = Some(i);
            update(app, Action::MarkListJump)
        }
        Action::BacklinksOpen => {
            let Some(path) = app
                .backlinks
                .as_ref()
                .and_then(|b| b.rows.get(b.selected))
                .map(|r| r.path.clone())
            else {
                return Outcome::Idle;
            };
            app.backlinks = None;
            let here = app.view.anchor;
            if let Some(from) = app.file.clone() {
                app.push_history(from, here);
            }
            match app.open_path(&path) {
                Ok(()) => Outcome::Redraw,
                Err(e) => {
                    app.note = Some(format!("cannot open {}: {e}", path.display()));
                    Outcome::Redraw
                }
            }
        }

        // Forward links: who this note points at. Derived whole at open —
        // every destination is already in memory — so there is no query to
        // drive and nothing to stream.
        Action::ForwardToggle => {
            app.forward = if app.forward.is_some() {
                None
            } else {
                Some(Forward {
                    rows: forward_rows(app),
                    selected: 0,
                })
            };
            Outcome::Redraw
        }
        Action::ForwardMove(n) => {
            if let Some(f) = app.forward.as_mut() {
                let last = f.rows.len().saturating_sub(1);
                f.selected = if n < 0 {
                    f.selected.saturating_sub(n.unsigned_abs() as usize)
                } else {
                    f.selected
                        .saturating_add(n.unsigned_abs() as usize)
                        .min(last)
                };
            }
            Outcome::Redraw
        }
        Action::ForwardOpen => {
            let Some(row) = app
                .forward
                .as_ref()
                .and_then(|f| f.rows.get(f.selected))
                .cloned()
            else {
                return Outcome::Idle;
            };
            let Some(path) = row.target else {
                // Not fetchable, but openable — the same door `Enter` on a
                // link in the text now opens.
                let dest = row.dest.clone();
                return open_external(app, &dest);
            };
            app.forward = None;
            let here = app.view.anchor;
            if let Some(from) = app.file.clone() {
                app.push_history(from, here);
            }
            match app.open_path(&path) {
                Ok(()) => Outcome::Redraw,
                Err(e) => {
                    app.note = Some(format!("cannot open {}: {e}", path.display()));
                    Outcome::Redraw
                }
            }
        }

        Action::OutlineJumpTo(b) => {
            let here = app.view.anchor;
            if app.file.is_some() || app.piped.is_some() {
                let from = app.file.clone().unwrap_or_default();
                app.push_history(from, here);
            }
            let byte = app.doc.node_for_block(b).doc.start;
            app.reveal_byte(byte, h, Where::Top);
            Outcome::Redraw
        }

        Action::FollowToggle => {
            app.following = !app.following;
            if app.following {
                app.view.scroll_to(&app.doc, &app.layout, u32::MAX, h);
                app.note = Some("following the end".into());
            } else {
                app.note = Some("stopped following".into());
            }
            Outcome::Redraw
        }

        // Stepping the block cursor is a byte-targeted jump like any other,
        // so it goes through `reveal_byte` — a code block inside a fold must
        // unfold its way into view rather than being unreachable.
        Action::CodeStep(n) => {
            let Some(target) = app.next_code_block(n) else {
                app.note = Some("no code block that way".into());
                return Outcome::Redraw;
            };
            app.code_focus = Some(target);
            let byte = app.doc.node_for_block(target).doc.start;
            app.reveal_byte(byte, h, Where::Top);
            Outcome::Redraw
        }

        Action::TaskStep(n) => {
            let tasks = app.doc.tasks();
            if tasks.is_empty() {
                app.note = Some("no task lists in this document".into());
                return Outcome::Redraw;
            }
            let here = app
                .doc
                .node_for_block(app.layout.block_at_row(app.view.scroll_row))
                .doc
                .start;
            let len = tasks.len();
            // `partition_point` already yields "the next task after the
            // cursor", so adding the whole step count on top of it landed one
            // too far — in BOTH directions. From the top of a three-task
            // document, five presses of `X` walked 2, 1, 3, 2, 1 instead of
            // 1, 2, 3, 1, 2. `MarkNext` twenty lines up is the model: it takes
            // the first mark strictly after the cursor and adds nothing.
            let count = usize::try_from(n.unsigned_abs()).unwrap_or(1).max(1);
            let i = if n >= 0 {
                (tasks.partition_point(|t| t.at <= here) + count - 1) % len
            } else {
                // The last task strictly before the cursor — `len - 1` when
                // there is none, which is the wrap — then back the rest.
                let last_before = (tasks.partition_point(|t| t.at < here) + len - 1) % len;
                (last_before + len - ((count - 1) % len)) % len
            };
            let t = &tasks[i];
            app.reveal_byte(t.at, h, crate::action::Where::Top);
            let state = if t.done { "done" } else { "open" };
            app.note = Some(format!("task {} of {len} ({state})", i + 1));
            Outcome::Redraw
        }

        Action::YankBlock => {
            let Some(b) = app.code_focus.or_else(|| app.next_code_block(0)) else {
                app.note = Some("no code block here to copy".into());
                return Outcome::Redraw;
            };
            app.code_focus = Some(b);
            let node = app.doc.node_for_block(b);
            let text = app.doc.text[node.doc.start as usize..node.doc.end as usize].to_string();
            // The same ceiling `copy_selection` applies to a drag, for the
            // same reason: terminals cap OSC 52 string length and truncate or
            // print the tail as literal text, so a 2 MB embedded blob was
            // never going to arrive — and writing it synchronously to the tty
            // from the UI thread made the failure slow as well as silent.
            if text.len() > CLIPBOARD_MAX {
                app.note = Some("code block too large to copy".into());
                return Outcome::Redraw;
            }
            let lines = text.lines().count();
            app.clipboard = Some(text);
            app.note = Some(format!(
                "copied {lines} line{}",
                if lines == 1 { "" } else { "s" }
            ));
            Outcome::Redraw
        }

        // The heartbeat. Only ever acts while auto-read is on, so a tick
        // that arrives after the reader took over is inert by construction.
        Action::AutoTick => {
            if !app.auto_read {
                return Outcome::Idle;
            }
            let before = app.view.scroll_row;
            app.view.scroll_by(&app.doc, &app.layout, 1, h);
            if app.view.scroll_row == before {
                app.auto_read = false;
                app.note = Some("the end — auto-read stopped".into());
            }
            Outcome::Redraw
        }

        Action::Scroll(span, n) => {
            // A deliberate move: the reader has the wheel again.
            app.auto_read = false;
            // Scrolling away from the end is a deliberate move: stop following.
            if n < 0 {
                app.following = false;
            }
            let step = match span {
                Span::Line => 1,
                Span::HalfPage => i32::from(h / 2).max(1),
                Span::Page => i32::from(h).max(1),
            };
            app.view
                .scroll_by(&app.doc, &app.layout, n.saturating_mul(step), h);
            Outcome::Redraw
        }

        Action::GoToStart => {
            app.following = false;
            app.view.scroll_to(&app.doc, &app.layout, 0, h);
            Outcome::Redraw
        }
        Action::GoToEnd => {
            app.auto_read = false;
            // `G` on a document that is still being written means "and keep
            // me there" — the one place following turns itself on.
            if app.streaming {
                app.following = true;
            }
            app.view.scroll_to(&app.doc, &app.layout, u32::MAX, h);
            Outcome::Redraw
        }
        Action::GoToRow(r) => {
            app.view.scroll_to(&app.doc, &app.layout, r, h);
            Outcome::Redraw
        }

        Action::BlockStep(n) => {
            let cur = app.layout.block_at_row(app.view.scroll_row).0;
            let last = u32::try_from(app.doc.block_count().saturating_sub(1)).unwrap_or(u32::MAX);
            let target = if n < 0 {
                cur.saturating_sub(n.unsigned_abs())
            } else {
                cur.saturating_add(n.unsigned_abs()).min(last)
            };
            let row = app.layout.row_start(BlockIdx(target));
            app.view.scroll_to(&app.doc, &app.layout, row, h);
            Outcome::Redraw
        }

        Action::Recenter(at) => {
            // Positions the CURRENT MATCH, not a cursor — a reader has none.
            let Some(byte) = app
                .matches
                .as_ref()
                .and_then(|m| m.current.and_then(|i| m.ranges.get(i)).map(|r| r.start))
            else {
                return Outcome::Idle;
            };
            app.reveal_byte(byte, h, at);
            Outcome::Redraw
        }

        Action::SearchOpen(dir) => {
            app.mode = Mode::Search {
                input: String::new(),
                dir,
                saved: app.view.anchor,
            };
            Outcome::Redraw
        }

        Action::SearchKey(k) => search_key(app, k, h),

        Action::MatchStep(n) => {
            app.following = false;
            let Some(m) = app.matches.as_mut() else {
                return Outcome::Idle;
            };
            if m.ranges.is_empty() {
                return Outcome::Idle;
            }
            let len = m.ranges.len();
            let cur = match m.current {
                Some(i) => {
                    // rem_euclid keeps the step non-negative, so `N` at match 0
                    // wraps to the end rather than underflowing.
                    let span = i32::try_from(len).unwrap_or(i32::MAX);
                    let step = usize::try_from(n.rem_euclid(span)).unwrap_or(0);
                    // Say so when the cycle passes an end — vim's
                    // "search hit BOTTOM", at reader volume.
                    let wrapped = if n >= 0 {
                        step != 0 && i + step >= len
                    } else {
                        let back = usize::try_from((-n).rem_euclid(span)).unwrap_or(0);
                        back != 0 && i < back
                    };
                    if wrapped {
                        app.note = Some("search wrapped".into());
                    }
                    (i + step) % len
                }
                // The first step lands ON the first match going forward and the
                // last going backward, rather than stepping past one.
                None => {
                    if n >= 0 {
                        0
                    } else {
                        len - 1
                    }
                }
            };
            m.current = Some(cur);
            let byte = m.ranges[cur].start;
            app.reveal_byte(byte, h, Where::Middle);
            Outcome::Redraw
        }

        Action::HelpToggle => {
            app.help = Some(0);
            Outcome::Redraw
        }

        Action::OutlineToggle => {
            let heads = app.headings();
            if heads.is_empty() {
                app.note = Some("no headings in this document".into());
                return Outcome::Redraw;
            }
            // Pre-select the section being read: the last heading at or
            // before the block containing the anchor.
            let at = app.doc.block_at_doc(DocByte(app.view.anchor));
            let selected = heads.iter().rposition(|b| *b <= at).unwrap_or(0);
            app.outline = Some(Outline {
                filter: String::new(),
                selected,
            });
            Outcome::Redraw
        }

        Action::RenderedToggle => {
            app.show_rendered = !app.show_rendered;
            app.note = Some(
                if app.show_rendered {
                    "rendered: art"
                } else {
                    "rendered: source"
                }
                .to_string(),
            );
            app.relayout();
            Outcome::Redraw
        }

        Action::TableToggle => {
            app.wrap_tables = !app.wrap_tables;
            app.note = Some(
                if app.wrap_tables {
                    "tables: wrapped"
                } else {
                    "tables: cards"
                }
                .to_string(),
            );
            app.relayout();
            Outcome::Redraw
        }

        // Home-only actions never reach the reader.
        _ => Outcome::Idle,
    }
}

/// `%`: the footnote's other end.
///
/// The last reference-or-definition at or above the view decides where you
/// are: under a reference, `%` takes you to its definition; inside a
/// definition, back to its first reference; above everything, to the first
/// reference in the document. Either way a history entry is pushed, so
/// `Ctrl-O` returns — a jump is a link follow in spirit.
fn footnote_jump(app: &mut App, h: u16) -> Outcome {
    let byte = app
        .doc
        .node_for_block(app.layout.block_at_row(app.view.scroll_row))
        .doc
        .start;
    let refs = app.doc.footnote_refs();
    let defs = app.doc.footnote_defs();
    if refs.is_empty() {
        app.note = Some("no footnotes in this document".into());
        return Outcome::Redraw;
    }
    let mut marks: Vec<(u32, bool, &str)> = refs
        .iter()
        .map(|(n, at)| (*at, false, n.as_ref()))
        .chain(defs.iter().map(|(n, at)| (*at, true, n.as_ref())))
        .collect();
    marks.sort_unstable_by_key(|m| m.0);
    let governing = marks.iter().rev().find(|m| m.0 <= byte).copied();
    let target = if let Some((_, true, name)) = governing {
        // Inside a definition: back to its first reference.
        refs.iter()
            .find(|(n, _)| n.as_ref() == name)
            .map(|(_, at)| *at)
    } else {
        // Under a reference (or past every mark): to its definition. Past
        // the last mark wraps to the first pair, like `/` wrapping.
        let name = if let Some((_, _, name)) = governing {
            name
        } else {
            refs[0].0.as_ref()
        };
        defs.iter()
            .find(|(d, _)| d.as_ref() == name)
            .map(|(_, at)| *at)
    };
    let Some(target) = target else {
        return Outcome::Idle;
    };
    if app.file.is_some() || app.piped.is_some() {
        let from = app.file.clone().unwrap_or_default();
        app.push_history(from, app.view.anchor);
    }
    app.reveal_byte(target, h, crate::action::Where::Top);
    Outcome::Redraw
}

/// Places ahead of the picker's own matches; a place that also matched the
/// filesystem listing keeps its front-row seat once.
fn merge_places(places: Vec<PathBuf>, matched: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut out = places;
    for m in matched {
        if !out.contains(&m) {
            out.push(m);
        }
    }
    out
}

/// Every distinct destination the open document points at, in reading order.
///
/// Resolution is exactly the reader's own rule — wikilinks through
/// `wiki::resolve`, relative links against the document's directory,
/// fragments stripped — so a row that says it opens really opens. Bare
/// `#fragment` links are internal navigation, not destinations, and are
/// skipped; duplicates collapse onto their first occurrence.
fn forward_rows(app: &App) -> Vec<ForwardRow> {
    let dir = app.doc_dir();
    let index = app
        .home_stash
        .as_deref()
        .map_or(&[][..], |h| h.entries.as_slice());
    let mut rows: Vec<ForwardRow> = Vec::new();
    for (i, link) in app.doc.links.iter().enumerate() {
        let id = LinkId(u32::try_from(i).unwrap_or(u32::MAX));
        if link.starts_with('#') {
            continue;
        }
        let target = if app.doc.is_wikilink(id) {
            app.wiki
                .get(&id)
                .cloned()
                .or_else(|| crate::wiki::resolve(link, &dir, index))
        } else if link.contains("://") {
            None
        } else {
            let bare = link.split(['#', '?']).next().unwrap_or(link);
            (!bare.is_empty()).then(|| dir.join(bare))
        };
        if rows
            .iter()
            .any(|r| r.dest.as_str() == link.as_ref() && r.target == target)
        {
            continue;
        }
        // The link's first visible run is its label, when that differs from
        // the destination itself.
        let label = app
            .doc
            .nodes
            .iter()
            .flat_map(|n| n.inlines.iter())
            .find(|inl| inl.link == Some(id))
            .map(|inl| app.doc.text[inl.doc.start as usize..inl.doc.end as usize].to_string())
            .filter(|t| !t.is_empty() && t.as_str() != &**link);
        rows.push(ForwardRow {
            dest: link.to_string(),
            label,
            target,
        });
    }
    rows
}

/// Back to the library, if we came from it. Entries, filter, and selection
/// are exactly as they were left; opened directly, `q` quits like a pager.
fn close_file(app: &mut App) -> Outcome {
    app.save_position();
    let Some(h) = app.home_stash.take() else {
        return Outcome::Quit;
    };
    app.screen = Screen::Home(h);
    app.matches = None;
    app.selected_link = None;
    app.history.clear();
    Outcome::Redraw
}

fn go_back(app: &mut App, h: u16) -> Outcome {
    let Some((prev, anchor)) = app.history.pop() else {
        return Outcome::Idle;
    };
    // A same-document entry (a fragment jump) restores the position without
    // re-reading the file — nothing about the document changed.
    if app.file.as_deref() == Some(prev.as_path()) {
        if app.unfold_to(anchor) {
            app.relayout();
        }
        app.view.anchor = anchor;
        app.view.restore(&app.doc, &app.layout, h);
        return Outcome::Redraw;
    }
    // Back to a piped document: no file to re-read, but the text was
    // retained for exactly this. Re-parse from memory and become pathless
    // again; the label follows whether the stream is still arriving.
    if prev == Path::new("(stdin)")
        && let Some(src) = app.piped.take()
    {
        app.save_position();
        app.file = None;
        app.reload_from(&src);
        app.piped = Some(src);
        app.path = if app.streaming {
            "(stdin — streaming…)".into()
        } else {
            "(stdin)".into()
        };
        app.view.anchor = anchor;
        app.view.restore(&app.doc, &app.layout, h);
        return Outcome::Redraw;
    }
    match app.open_path(&prev) {
        Ok(()) => {
            // The anchor is a doc byte, so the reading position returns
            // through the same StableViewport path as a resize.
            app.view.anchor = anchor;
            app.view.restore(&app.doc, &app.layout, h);
        }
        Err(e) => {
            app.note = Some(format!("cannot go back to {}: {e}", prev.display()));
        }
    }
    Outcome::Redraw
}

fn link_step(app: &mut App, n: i32, h: u16) -> Outcome {
    // Moving off the link abandons any pending confirmation.
    app.pending_open = None;
    let Ok(len) = i64::try_from(app.doc.links.len()) else {
        return Outcome::Idle;
    };
    if len == 0 {
        return Outcome::Idle;
    }
    let cur = match app.selected_link {
        Some(LinkId(i)) => (i64::from(i) + i64::from(n)).rem_euclid(len),
        // The first step lands ON the first link forward, the last backward.
        None => {
            if n >= 0 {
                0
            } else {
                len - 1
            }
        }
    };
    let id = LinkId(u32::try_from(cur).unwrap_or(0));
    app.selected_link = Some(id);
    if let Some(pos) = app.link_pos(id) {
        app.reveal_byte(pos, h, Where::Middle);
    }
    Outcome::Redraw
}

/// The longest URL carrel will hand to the desktop.
///
/// The same cap the OSC 8 pass uses, for the same reason: terminals and
/// desktop openers both give up somewhere near 2 KiB, and a document that
/// wants more than this is not describing a link a person meant to follow.
const MAX_URL: usize = 2048;

/// Does this destination carry a URI scheme at all?
///
/// RFC 3986: `ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )` before the colon.
/// Two characters minimum, so a Windows-shaped `C:\path` stays a path.
#[must_use]
fn has_scheme(dest: &str) -> bool {
    let Some((scheme, _)) = dest.split_once(':') else {
        return false;
    };
    scheme.len() >= 2
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// The URL, if it is one carrel is willing to open — otherwise `None`.
///
/// **This is the security boundary, and it is an allowlist on purpose.**
/// Carrel's audience arrives holding a markdown file an AI agent wrote out of
/// web pages they never read, and following a link in it is the most ordinary
/// thing they will do. So: `http`, `https` and `mailto` open, and everything
/// else — `file:` reading the disk, `javascript:` and `data:` where the opener
/// is a browser, `ssh:`, and every scheme nobody has thought of yet — does
/// not, whatever the desktop's handler table would have made of it.
///
/// Whitespace and control characters are refused outright rather than
/// escaped. A URL containing either is not one a document meant to offer, and
/// argument splitting is not a place to be clever.
#[must_use]
pub fn openable_url(dest: &str) -> Option<&str> {
    if dest.len() > MAX_URL || dest.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return None;
    }
    let (scheme, rest) = dest.split_once(':')?;
    let ok = if scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https") {
        // A bare `https:` or `https://` names no host.
        rest.strip_prefix("//").is_some_and(|host| !host.is_empty())
    } else if scheme.eq_ignore_ascii_case("mailto") {
        !rest.is_empty()
    } else {
        false
    };
    ok.then_some(dest)
}

/// Fill the browser outbox, or say why not.
///
/// The refusal deliberately does not echo the destination back: the reader is
/// already showing the link it is talking about, and a note is not the place
/// to reprint a string this function has just declined to trust.
fn open_external(app: &mut App, dest: &str) -> Outcome {
    match openable_url(dest) {
        Some(url) => {
            app.open_url = Some(url.to_string());
            app.note = Some("opened in your browser".to_string());
        }
        None => {
            app.note = Some("carrel opens http, https and mailto links only".to_string());
        }
    }
    Outcome::Redraw
}

/// Fold or unfold whatever `byte` is inside, if it is inside anything
/// foldable. `true` when something changed.
///
/// A `<details>` summary folds its region exactly as a heading folds its
/// section — the two markers look alike, so they must behave alike — and a
/// summary wins, because it is the more specific of the two.
fn fold_at(app: &mut App, byte: u32) -> bool {
    if let Some((i, _)) = app
        .doc
        .details
        .iter()
        .enumerate()
        .find(|(_, r)| r.summary.contains(&byte))
    {
        let i = u32::try_from(i).unwrap_or(u32::MAX);
        if !app.folded_details.remove(&i) {
            app.folded_details.insert(i);
        }
        app.after_fold_change();
        return true;
    }
    let node = app.doc.node_for_block(app.doc.block_at_doc(DocByte(byte)));
    if matches!(node.kind, carrel_core::NodeKind::Heading { .. }) {
        let id = node.id;
        if !app.folded.remove(&id) {
            app.folded.insert(id);
        }
        app.after_fold_change();
        return true;
    }
    false
}

fn link_follow(app: &mut App) -> Outcome {
    let Some(id) = app.selected_link else {
        return Outcome::Idle;
    };
    let url = app.doc.links[id.0 as usize].to_string();
    // A wikilink's destination is a note name, resolved at open time — not a
    // path relative to this file and never a URI.
    if app.doc.is_wikilink(id) {
        return wiki_follow(app, id, &url);
    }
    // Anything with a scheme is external, and external now OPENS.
    //
    // This reverses "a reader does not spawn programs", which was right when
    // the answer was OSC 8 and the audience already knew what OSC 8 was. It
    // told everyone else to click something carrel was not making clickable,
    // and offered "copy it" as the fallback. Handing the URL to the desktop is
    // still not fetching: no socket is opened here, and carrel depends on no
    // HTTP client and no TLS library — the README's claim is unchanged and
    // stays checkable with `cargo tree`.
    if has_scheme(&url) {
        return open_external(app, &url);
    }
    // A piped document has no path; the sentinel's parent is the empty
    // path, so relative targets resolve against the working directory, and
    // `go_back` recognises it to return to the retained text.
    let here = app.file.clone().unwrap_or_else(|| PathBuf::from("(stdin)"));
    // `#section` jumps within this document; `notes.md#section` opens the
    // file, then jumps. Fragments resolve through the core's GitHub-style
    // slugs, so both frontends agree on where a link lands.
    let (bare, frag) = match url.split_once('#') {
        Some((b, f)) => (b, Some(f)),
        None => (url.as_str(), None),
    };
    if bare.is_empty() {
        let Some(frag) = frag.filter(|f| !f.is_empty()) else {
            app.note = Some("nothing to follow".into());
            return Outcome::Redraw;
        };
        let anchor = app.view.anchor;
        if jump_to_fragment(app, frag) {
            app.push_history(here, anchor);
        }
        return Outcome::Redraw;
    }
    let target = here.parent().unwrap_or(Path::new(".")).join(bare);
    if app.escapes_library(&target) && !app.confirmed_open(id, &target) {
        return app.ask_before_leaving(id, &target);
    }
    let anchor = app.view.anchor;
    match app.open_path(&target) {
        Ok(()) => {
            app.push_history(here, anchor);
            // A missing fragment in an opened file is not an error: you are
            // in the right document, at the top, and the note says why.
            if let Some(f) = frag.filter(|f| !f.is_empty()) {
                jump_to_fragment(app, f);
            }
            Outcome::Redraw
        }
        Err(e) => {
            app.note = Some(format!("cannot open {}: {e}", target.display()));
            Outcome::Redraw
        }
    }
}

/// What a single OSC 52 write may carry.
///
/// Terminals cap the length of an OSC string; past it they truncate, or drop
/// back to printing the tail as literal text on the user's screen. A copy
/// this large is a mis-drag or a generated blob, not something anyone is
/// pasting.
pub const CLIPBOARD_MAX: usize = 100_000;

/// Fill the clipboard outbox from the current selection — or say why not.
fn copy_selection(app: &mut App) {
    let Some(sel) = app.selection.clone() else {
        return;
    };
    if sel.is_empty() {
        return;
    }
    if (sel.end - sel.start) as usize > CLIPBOARD_MAX {
        app.note = Some("selection too large to copy".into());
        return;
    }
    let Some(text) = app.doc.text.get(sel.start as usize..sel.end as usize) else {
        return;
    };
    app.note = Some(format!("copied {} chars", text.chars().count()));
    app.clipboard = Some(text.to_string());
}

/// The alphanumeric/`_`/`-` run around `byte`, or `None` on a non-word cell.
fn word_range_at(text: &str, byte: u32) -> Option<std::ops::Range<u32>> {
    let mut at = byte as usize;
    if at >= text.len() {
        return None;
    }
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '-';
    if !text[at..].chars().next().is_some_and(is_word) {
        return None;
    }
    let end = at + text[at..].find(|c| !is_word(c)).unwrap_or(text.len() - at);
    let start = text[..at].rfind(|c| !is_word(c)).map_or(0, |i| {
        i + text[i..].chars().next().map_or(1, char::len_utf8)
    });
    Some(u32::try_from(start).unwrap_or(0)..u32::try_from(end).unwrap_or(u32::MAX))
}

/// Follow a `[[wikilink]]`: the file was resolved at open time, the fragment
/// goes through the same slug jump as ordinary links.
fn wiki_follow(app: &mut App, id: LinkId, url: &str) -> Outcome {
    let (bare, frag) = match url.split_once('#') {
        Some((b, f)) => (b, Some(f)),
        None => (url, None),
    };
    let Some(here) = app.file.clone() else {
        app.note = Some("no file context to resolve a wikilink".into());
        return Outcome::Redraw;
    };
    if bare.is_empty() {
        // [[#Heading]] — an in-document jump.
        let Some(frag) = frag.filter(|f| !f.is_empty()) else {
            app.note = Some("nothing to follow".into());
            return Outcome::Redraw;
        };
        let anchor = app.view.anchor;
        if jump_to_wiki_fragment(app, frag) {
            app.push_history(here, anchor);
        }
        return Outcome::Redraw;
    }
    let Some(target) = app.wiki.get(&id).cloned() else {
        app.note = Some(format!("no note named '{bare}' here"));
        return Outcome::Redraw;
    };
    // A wikilink's first resolution rule is `here_dir.join(name)`, so
    // `[[../../etc/passwd]]` escapes exactly as a markdown link does.
    if app.escapes_library(&target) && !app.confirmed_open(id, &target) {
        return app.ask_before_leaving(id, &target);
    }
    let anchor = app.view.anchor;
    match app.open_path(&target) {
        Ok(()) => {
            app.push_history(here, anchor);
            if let Some(f) = frag.filter(|f| !f.is_empty()) {
                jump_to_wiki_fragment(app, f);
            }
        }
        Err(e) => {
            app.note = Some(format!("cannot open {}: {e}", target.display()));
        }
    }
    Outcome::Redraw
}

/// `[[Note#Some Heading]]` carries heading TEXT, not a slug (Obsidian's
/// convention). Try it as written first — someone may write the slug — then
/// slugged the same way the core slugs headings.
fn jump_to_wiki_fragment(app: &mut App, frag: &str) -> bool {
    if app.doc.fragment_target(frag).is_some() {
        return jump_to_fragment(app, frag);
    }
    let slug: String = frag
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
            _ => None,
        })
        .collect();
    jump_to_fragment(app, &slug)
}

/// Scroll the heading `#frag` names to the top of the view. `false` (with a
/// note) when no heading matches.
fn jump_to_fragment(app: &mut App, frag: &str) -> bool {
    let Some(at) = app.doc.fragment_target(frag) else {
        app.note = Some(format!("no such section: #{frag}"));
        return false;
    };
    let h = app.text_h();
    app.reveal_byte(at, h, Where::Top);
    true
}

fn search_key(app: &mut App, k: SearchKey, h: u16) -> Outcome {
    match k {
        SearchKey::Char(c) => {
            if let Mode::Search { input, .. } = &mut app.mode {
                input.push(c);
            }
            app.rerun_search();
        }
        SearchKey::Backspace => {
            if let Mode::Search { input, .. } = &mut app.mode {
                input.pop();
            }
            app.rerun_search();
        }
        SearchKey::Accept => {
            let Mode::Search { dir, .. } = &app.mode else {
                return Outcome::Idle;
            };
            let dir = *dir;
            app.mode = Mode::Normal;
            if app.matches.as_ref().is_some_and(|m| !m.ranges.is_empty()) {
                let step = if dir == Direction::Forward { 1 } else { -1 };
                return update(app, Action::MatchStep(step));
            }
        }
        SearchKey::Cancel => {
            let Mode::Search { saved, .. } = &app.mode else {
                return Outcome::Idle;
            };
            let saved = *saved;
            app.mode = Mode::Normal;
            app.matches = None;
            app.view.anchor = saved;
            app.view.restore(&app.doc, &app.layout, h);
        }
    }
    Outcome::Redraw
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write as _;

    #[test]
    fn hints_toggle_flips_and_persists_only_into_the_injected_dir() {
        let cfg = tempfile::tempdir().unwrap();
        let mut a = App::new("t.md".into(), Document::parse("# T"), 40, 10);
        assert!(a.hints, "on by default");
        update(&mut a, Action::HintsToggle);
        assert!(!a.hints);
        // No config_dir — nothing persisted anywhere.
        a.config_dir = Some(cfg.path().into());
        update(&mut a, Action::HintsToggle);
        assert!(a.hints);
        assert_eq!(crate::config::load_hints_in(cfg.path()), Some(true));
    }

    const SRC: &str =
        "# T\n\nalpha beta gamma delta epsilon zeta eta theta iota kappa\n\n- one\n- two\n";

    /// 20x6 leaves a 19x5 text area — narrow and short enough that this
    /// document actually scrolls, which several tests below depend on.
    fn app() -> App {
        App::new("t.md".into(), Document::parse(SRC), 20, 6)
    }

    #[test]
    fn scrolling_moves_the_view_and_asks_for_a_redraw() {
        let mut a = app();
        assert_eq!(
            update(&mut a, Action::Scroll(Span::Line, 2)),
            Outcome::Redraw
        );
        assert_eq!(a.view.scroll_row, 2);
    }

    #[test]
    fn go_to_end_lands_on_the_last_screenful() {
        let mut a = app();
        update(&mut a, Action::GoToEnd);
        assert_eq!(a.view.scroll_row, a.layout.max_scroll(a.text_h()));
    }

    #[test]
    fn quit_reports_quit() {
        let mut a = app();
        assert_eq!(update(&mut a, Action::Quit), Outcome::Quit);
    }

    #[test]
    fn opening_search_enters_search_mode_and_remembers_where_we_were() {
        let mut a = app();
        update(&mut a, Action::Scroll(Span::Line, 2));
        let before = a.view.anchor;
        update(&mut a, Action::SearchOpen(Direction::Forward));
        assert!(a.searching());
        match &a.mode {
            Mode::Search { saved, .. } => assert_eq!(*saved, before),
            Mode::Normal => panic!("expected search mode"),
        }
    }

    #[test]
    fn search_runs_incrementally_on_every_keystroke() {
        let mut a = app();
        update(&mut a, Action::SearchOpen(Direction::Forward));
        update(&mut a, Action::SearchKey(SearchKey::Char('a')));
        let after_a = a.matches.as_ref().expect("matches after one char").len();
        assert!(after_a > 0);
        update(&mut a, Action::SearchKey(SearchKey::Char('l')));
        let after_al = a.matches.as_ref().unwrap().len();
        assert!(after_al < after_a, "a longer needle must not match more");
    }

    #[test]
    fn cancelling_a_search_discards_matches_and_restores_the_position() {
        let mut a = app();
        update(&mut a, Action::Scroll(Span::Line, 2));
        let before = a.view.anchor;
        update(&mut a, Action::SearchOpen(Direction::Forward));
        update(&mut a, Action::SearchKey(SearchKey::Char('t')));
        update(&mut a, Action::SearchKey(SearchKey::Cancel));
        assert!(a.matches.is_none());
        assert!(!a.searching());
        assert_eq!(a.view.anchor, before);
    }

    #[test]
    fn accepting_a_search_keeps_the_matches() {
        let mut a = app();
        update(&mut a, Action::SearchOpen(Direction::Forward));
        update(&mut a, Action::SearchKey(SearchKey::Char('a')));
        update(&mut a, Action::SearchKey(SearchKey::Accept));
        assert!(!a.searching());
        assert!(a.matches.is_some());
    }

    #[test]
    fn a_short_document_says_nothing_about_time() {
        let a = app();
        assert_eq!(
            a.minutes_left(),
            None,
            "under a minute, or nothing to scroll: stay quiet"
        );
    }

    #[test]
    fn a_long_document_estimates_and_counts_down_as_you_read() {
        // 4,000 words at 200 wpm is about twenty minutes.
        let body = "word ".repeat(4_000);
        let mut a = App::new("t.md".into(), Document::parse(&body), 80, 24);
        a.on_resize(80, 24);
        assert_eq!(a.words, 4_000);
        let start = a.minutes_left().expect("a long document has time left");
        assert!(
            (18..=22).contains(&start),
            "expected about 20 minutes, got {start}"
        );

        update(&mut a, Action::GoToEnd);
        assert_eq!(a.minutes_left(), None, "at the end there is nothing left");
    }

    #[test]
    fn the_estimate_is_derived_from_the_same_scroll_the_percentage_uses() {
        let body = "word ".repeat(4_000);
        let mut a = App::new("t.md".into(), Document::parse(&body), 80, 24);
        a.on_resize(80, 24);
        let before = a.minutes_left().unwrap();
        update(&mut a, Action::Scroll(Span::Page, 5));
        let after = a.minutes_left().unwrap();
        assert!(after < before, "scrolling forward must reduce the estimate");
    }

    #[test]
    fn esc_clears_the_highlights_left_by_an_accepted_search() {
        // Reported 2026-08-16: search, press Enter, then Esc — and the
        // highlights stay on screen with no way to clear them but running
        // another search. `Dismiss` cleared the selection and the link and
        // never touched `matches`.
        let mut a = app();
        update(&mut a, Action::SearchOpen(Direction::Forward));
        update(&mut a, Action::SearchKey(SearchKey::Char('a')));
        update(&mut a, Action::SearchKey(SearchKey::Accept));
        assert!(a.matches.is_some(), "precondition: a search landed");

        update(&mut a, Action::Dismiss);
        assert!(a.matches.is_none(), "Esc must clear the search highlights");
    }

    #[test]
    fn esc_after_a_search_does_not_yank_the_view_back() {
        // The difference from cancelling mid-typing: accepting a search MOVED
        // you somewhere on purpose. Clearing the highlights afterwards must
        // not undo that navigation — only `SearchKey::Cancel` restores.
        let mut a = app();
        update(&mut a, Action::SearchOpen(Direction::Forward));
        update(&mut a, Action::SearchKey(SearchKey::Char('a')));
        update(&mut a, Action::SearchKey(SearchKey::Accept));
        let landed = a.view.anchor;
        let row = a.view.scroll_row;

        update(&mut a, Action::Dismiss);
        assert_eq!(a.view.anchor, landed, "Esc must not move the reader");
        assert_eq!(a.view.scroll_row, row);
    }

    #[test]
    fn esc_with_nothing_to_clear_is_harmless() {
        let mut a = app();
        let row = a.view.scroll_row;
        assert_eq!(update(&mut a, Action::Dismiss), Outcome::Redraw);
        assert!(a.matches.is_none());
        assert_eq!(a.view.scroll_row, row);
    }

    #[test]
    fn match_step_wraps_around_and_sets_the_current_index() {
        let mut a = app();
        update(&mut a, Action::SearchOpen(Direction::Forward));
        update(&mut a, Action::SearchKey(SearchKey::Char('a')));
        update(&mut a, Action::SearchKey(SearchKey::Accept));
        let n = a.matches.as_ref().unwrap().len();
        assert_eq!(
            a.matches.as_ref().unwrap().current,
            Some(0),
            "accept lands on the first"
        );
        update(&mut a, Action::MatchStep(i32::try_from(n).unwrap()));
        assert_eq!(a.matches.as_ref().unwrap().current, Some(0), "wraps");
    }

    #[test]
    fn recenter_is_a_no_op_without_a_current_match() {
        let mut a = app();
        let before = a.view.scroll_row;
        assert_eq!(
            update(&mut a, Action::Recenter(Where::Middle)),
            Outcome::Idle
        );
        assert_eq!(a.view.scroll_row, before);
    }

    #[test]
    fn resize_rebuilds_layout_and_keeps_the_anchor() {
        let mut a = app();
        update(&mut a, Action::Scroll(Span::Line, 3));
        let before = a.view.anchor;
        a.on_resize(40, 6);
        assert_eq!(a.layout.width(), a.text_w());
        assert_eq!(a.view.anchor, before);
    }

    #[test]
    fn t_toggles_tables_between_cards_and_wrapped_and_says_so() {
        let wide = "| name | description |\n|---|---|\n\
                    | alpha | a value easily long enough to overflow |\n";
        let mut a = App::new("t.md".into(), Document::parse(wide), 30, 10);
        let h_cards = a.layout.height(BlockIdx(0));
        update(&mut a, Action::TableToggle);
        assert!(a.wrap_tables);
        assert_eq!(a.note.as_deref(), Some("tables: wrapped"));
        assert_ne!(a.layout.height(BlockIdx(0)), h_cards, "relayout happened");
        update(&mut a, Action::TableToggle);
        assert!(!a.wrap_tables);
        assert_eq!(a.note.as_deref(), Some("tables: cards"));
        assert_eq!(a.layout.height(BlockIdx(0)), h_cards);
    }

    #[test]
    fn a_resize_across_the_card_threshold_keeps_the_anchor_row_visible() {
        // The teeth of the property: after flipping modes via resize, the top
        // visible row still contains the anchor (StableViewport, spec §3).
        // Mirrors `top_visible_row` in `tests/resize.rs`, adapted to the real
        // accessors (`on_resize`, `view.scroll_row` as a field, `Action::Scroll`
        // taking a `Span`).
        let wide = "before paragraph\n\n| name | description |\n|---|---|\n\
                    | alpha | long enough value to overflow narrow widths |\n\
                    | beta | second row value also fairly long here |\n\n\
                    after paragraph\n";
        let mut a = App::new("t.md".into(), Document::parse(wide), 80, 8);
        update(&mut a, Action::Scroll(Span::Line, 3)); // land inside the table
        let anchor = a.view.anchor;
        a.on_resize(34, 8); // crosses the threshold: aligned -> cards
        let b = a.layout.block_at_row(a.view.scroll_row);
        let mut rows = Vec::new();
        a.layout.rows_for(&a.doc, b, &mut rows);
        let sub = (a.view.scroll_row - a.layout.row_start(b)) as usize;
        let r = &rows[sub];
        assert!(
            r.doc.start <= anchor && (anchor < r.doc.end || r.doc.is_empty()),
            "top row {r:?} must contain anchor {anchor}"
        );
    }

    #[test]
    fn the_home_screen_opens_the_selected_file_into_the_reader() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("x.md");
        std::fs::write(&f, "# hello\n\nbody text").unwrap();

        let cached = vec![Entry {
            path: f.clone(),
            mtime: std::time::SystemTime::now(),
        }];
        let mut a = App::new_home(d.path().into(), cached, 40, 10);
        assert!(a.is_home());

        assert_eq!(update(&mut a, Action::HomeOpen), Outcome::Redraw);
        assert!(!a.is_home(), "must be in the reader now");
        assert!(a.doc.text.contains("hello"), "{}", a.doc.text);
        assert_eq!(a.path, "x.md");
    }

    #[test]
    fn opening_with_nothing_selected_stays_on_the_home_screen() {
        let d = tempfile::tempdir().unwrap();
        let mut a = App::new_home(d.path().into(), vec![], 40, 10);
        assert_eq!(update(&mut a, Action::HomeOpen), Outcome::Idle);
        assert!(a.is_home());
    }

    #[test]
    fn opening_a_file_that_vanished_keeps_you_on_the_home_screen_with_a_note() {
        let mut a = App::new_home(
            "/tmp".into(),
            vec![Entry {
                path: PathBuf::from("/nonexistent/gone.md"),
                mtime: std::time::SystemTime::UNIX_EPOCH,
            }],
            40,
            10,
        );
        assert_eq!(update(&mut a, Action::HomeOpen), Outcome::Redraw);
        assert!(a.is_home(), "must not drop into an empty reader");
        assert!(a.home().unwrap().note.is_some());
    }

    #[test]
    fn escape_clears_a_filter_before_it_changes_mode() {
        let d = tempfile::tempdir().unwrap();
        let mut a = App::new_home(d.path().into(), vec![], 40, 10);
        update(&mut a, Action::HomeFilterMode);
        update(&mut a, Action::HomeKey(SearchKey::Char('x')));
        update(&mut a, Action::HomeKey(SearchKey::Cancel));
        let h = a.home().unwrap();
        assert!(h.filter.is_empty(), "first Esc clears the filter");
        assert_eq!(h.mode, HomeMode::Filter, "and stays in filter mode");

        update(&mut a, Action::HomeKey(SearchKey::Cancel));
        assert_eq!(
            a.home().unwrap().mode,
            HomeMode::Normal,
            "second Esc leaves"
        );
    }

    #[test]
    fn a_note_lands_on_the_status_bar_of_the_screen_the_user_is_looking_at() {
        // The theme name was invisible when cycling on the home screen: the
        // note went to App::note, but the home status bar paints Home::note.
        let d = tempfile::tempdir().unwrap();
        let mut a = App::new_home(d.path().into(), vec![], 40, 10);
        a.set_note("theme: nord".into());
        assert_eq!(a.home().unwrap().note.as_deref(), Some("theme: nord"));
        assert_eq!(a.note, None, "not on the reader's bar");

        let mut r = App::new("t.md".into(), Document::parse("x"), 40, 10);
        r.set_note("theme: nord".into());
        assert_eq!(r.note.as_deref(), Some("theme: nord"));
    }

    #[test]
    fn cycling_past_the_last_match_says_the_search_wrapped() {
        let mut a = App::new("t.md".into(), Document::parse("x a x a x"), 40, 6);
        update(&mut a, Action::SearchOpen(Direction::Forward));
        update(&mut a, Action::SearchKey(SearchKey::Char('a')));
        update(&mut a, Action::SearchKey(SearchKey::Accept));
        update(&mut a, Action::MatchStep(1)); // -> 2 of 2
        assert_eq!(a.note, None, "no note mid-cycle");
        update(&mut a, Action::MatchStep(1)); // -> 1 of 2, wrapped
        assert_eq!(a.note.as_deref(), Some("search wrapped"));
        update(&mut a, Action::MatchStep(-1)); // back to 2 of 2, wrapped again
        assert_eq!(a.note.as_deref(), Some("search wrapped"));
    }

    #[test]
    fn following_a_fragment_link_jumps_to_its_heading_and_back_returns() {
        let src = "[go](#target-section)\n\n# Filler\n\npara\n\npara\n\npara\n\n# Target Section\n\nthe destination\n";
        let mut a = App::new("t.md".into(), Document::parse(src), 40, 6);
        a.file = Some(PathBuf::from("/tmp/t.md"));
        update(&mut a, Action::LinkStep(1));
        assert!(a.selected_link.is_some());
        update(&mut a, Action::LinkFollow);

        let at = a.doc.fragment_target("target-section").unwrap();
        let block = a.doc.block_at_doc(carrel_core::DocByte(at));
        assert_eq!(
            a.view.scroll_row,
            a.layout.row_start(block),
            "the heading's row is the top of the view"
        );
        update(&mut a, Action::Back);
        assert_eq!(a.view.scroll_row, 0, "Back returns to where you were");
    }

    #[test]
    fn an_unknown_fragment_says_so_instead_of_jumping() {
        let src = "[go](#nowhere)\n\nbody\n";
        let mut a = App::new("t.md".into(), Document::parse(src), 40, 6);
        a.file = Some(PathBuf::from("/tmp/t.md"));
        update(&mut a, Action::LinkStep(1));
        update(&mut a, Action::LinkFollow);
        assert_eq!(a.view.scroll_row, 0);
        assert!(
            a.note.as_deref().is_some_and(|n| n.contains("nowhere")),
            "{:?}",
            a.note
        );
    }

    #[test]
    fn reader_actions_are_ignored_on_the_home_screen() {
        let d = tempfile::tempdir().unwrap();
        let mut a = App::new_home(d.path().into(), vec![], 40, 10);
        assert_eq!(update(&mut a, Action::Scroll(Span::Line, 1)), Outcome::Idle);
        assert!(a.is_home());
    }

    #[test]
    fn image_dimensions_arriving_is_just_another_reflow() {
        let src = "alpha beta gamma delta epsilon zeta\n\n![pic](p.png)\n\nomega psi chi phi\n";
        let mut a = App::new("t.md".into(), Document::parse(src), 20, 6);
        update(&mut a, Action::Scroll(Span::Line, 2));
        let anchor = a.view.anchor;
        let before = a.layout.total_rows();

        // Pixels arrive: 100×200 px at the default (8,16) font.
        a.image_dims.insert(BlockIdx(1), (100, 200));
        a.relayout();

        assert_eq!(a.view.anchor, anchor, "the anchor is the authority");
        assert_ne!(a.layout.total_rows(), before, "the image grew the document");
        assert!(a.layout.height(BlockIdx(1)) > 1);
    }

    #[test]
    fn reopening_a_file_resumes_where_you_left_off() {
        let d = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let f = d.path().join("long.md");
        std::fs::write(&f, SRC).unwrap();

        let cached = vec![Entry {
            path: f.clone(),
            mtime: std::time::SystemTime::now(),
        }];
        let mut a = App::new_home(d.path().into(), cached.clone(), 20, 6);
        a.state_dir = Some(state.path().to_path_buf());
        update(&mut a, Action::HomeOpen);
        update(&mut a, Action::Scroll(Span::Line, 3));
        let anchor = a.view.anchor;
        assert!(anchor > 0, "the doc must actually scroll");
        assert_eq!(update(&mut a, Action::Quit), Outcome::Quit);

        let mut b = App::new_home(d.path().into(), cached, 20, 6);
        b.state_dir = Some(state.path().to_path_buf());
        update(&mut b, Action::HomeOpen);
        assert_eq!(b.view.anchor, anchor, "silent resume");
        assert_eq!(b.note.as_deref(), Some("resumed — gg for top"));
    }

    #[test]
    fn a_position_at_the_top_is_not_saved_and_clears_a_stale_one() {
        let d = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let f = d.path().join("doc.md");
        std::fs::write(&f, SRC).unwrap();
        crate::state::save_position_in(state.path(), &f, 7, 0, 0).unwrap();

        let cached = vec![Entry {
            path: f.clone(),
            mtime: std::time::SystemTime::now(),
        }];
        let mut a = App::new_home(d.path().into(), cached, 20, 6);
        a.state_dir = Some(state.path().to_path_buf());
        update(&mut a, Action::HomeOpen);
        update(&mut a, Action::GoToStart); // read it, go back to the top
        update(&mut a, Action::Quit);
        assert_eq!(
            crate::state::load_position_in(state.path(), &f),
            None,
            "top of file means no entry"
        );
    }

    #[test]
    fn a_saved_position_past_a_shrunken_file_clamps_instead_of_panicking() {
        let d = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let f = d.path().join("doc.md");
        std::fs::write(&f, "tiny\n").unwrap();
        crate::state::save_position_in(state.path(), &f, 10_000, 0, 0).unwrap();

        let cached = vec![Entry {
            path: f.clone(),
            mtime: std::time::SystemTime::now(),
        }];
        let mut a = App::new_home(d.path().into(), cached, 20, 6);
        a.state_dir = Some(state.path().to_path_buf());
        update(&mut a, Action::HomeOpen);
        assert!((a.view.anchor as usize) < a.doc.text.len().max(1));
    }

    #[test]
    fn without_a_state_dir_nothing_is_persisted() {
        // Every constructor defaults to None — the tests/home.rs lesson.
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("doc.md");
        std::fs::write(&f, SRC).unwrap();
        let cached = vec![Entry {
            path: f.clone(),
            mtime: std::time::SystemTime::now(),
        }];
        let mut a = App::new_home(d.path().into(), cached, 20, 6);
        assert_eq!(a.state_dir, None);
        update(&mut a, Action::HomeOpen);
        update(&mut a, Action::Scroll(Span::Line, 3));
        update(&mut a, Action::Quit); // must not create any file anywhere
    }

    #[test]
    fn following_a_link_away_saves_the_position_of_the_file_you_left() {
        let d = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let target = d.path().join("target.md");
        std::fs::write(&target, "# target\n\nbody\n").unwrap();
        // The link sits at the BOTTOM: revealing it via Tab re-anchors the
        // view deep into the document, so the position saved on follow is
        // meaningfully nonzero. (A link at the top would re-anchor to 0 and
        // the save would — correctly — clear the entry.)
        let here = d.path().join("here.md");
        std::fs::write(&here, format!("{SRC}\n\n[go](target.md)\n")).unwrap();

        let cached = vec![Entry {
            path: here.clone(),
            mtime: std::time::SystemTime::now(),
        }];
        let mut a = App::new_home(d.path().into(), cached, 20, 6);
        a.state_dir = Some(state.path().to_path_buf());
        update(&mut a, Action::HomeOpen);
        update(&mut a, Action::LinkStep(1)); // reveal re-anchors mid-document
        let anchor = a.view.anchor;
        assert!(anchor > 0, "revealing the bottom link must scroll");
        update(&mut a, Action::LinkFollow);
        assert_eq!(a.path, "target.md");
        assert_eq!(
            crate::state::load_position_in(state.path(), &here),
            Some(anchor),
        );
    }

    #[test]
    fn following_a_wikilink_opens_the_sibling_note() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("Reflow Layer.md"), "# reflow\n\ncontent\n").unwrap();
        let here = d.path().join("here.md");
        std::fs::write(&here, "see [[Reflow Layer]] for details\n").unwrap();

        let cached = vec![Entry {
            path: here.clone(),
            mtime: std::time::SystemTime::now(),
        }];
        let mut a = App::new_home(d.path().into(), cached, 40, 10);
        update(&mut a, Action::HomeOpen);
        update(&mut a, Action::LinkStep(1));
        update(&mut a, Action::LinkFollow);
        assert_eq!(a.path, "Reflow Layer.md");
        assert!(a.doc.text.contains("content"));
        update(&mut a, Action::Back);
        assert_eq!(a.path, "here.md", "history works for wikilinks too");
    }

    #[test]
    fn an_unresolved_wikilink_notes_and_stays_put() {
        let d = tempfile::tempdir().unwrap();
        let here = d.path().join("here.md");
        std::fs::write(&here, "see [[No Such Note]]\n").unwrap();
        let cached = vec![Entry {
            path: here.clone(),
            mtime: std::time::SystemTime::now(),
        }];
        let mut a = App::new_home(d.path().into(), cached, 40, 10);
        update(&mut a, Action::HomeOpen);
        update(&mut a, Action::LinkStep(1));
        update(&mut a, Action::LinkFollow);
        assert_eq!(a.path, "here.md", "did not navigate");
        assert_eq!(a.note.as_deref(), Some("no note named 'No Such Note' here"));
    }

    #[test]
    fn a_wikilink_fragment_jumps_after_opening() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(
            d.path().join("Note.md"),
            "# Filler\n\npara\n\npara\n\npara\n\n# Target Section\n\ndest\n",
        )
        .unwrap();
        let here = d.path().join("here.md");
        std::fs::write(&here, "see [[Note#Target Section]]\n").unwrap();
        let cached = vec![Entry {
            path: here.clone(),
            mtime: std::time::SystemTime::now(),
        }];
        let mut a = App::new_home(d.path().into(), cached, 40, 6);
        update(&mut a, Action::HomeOpen);
        update(&mut a, Action::LinkStep(1));
        update(&mut a, Action::LinkFollow);
        assert_eq!(a.path, "Note.md");
        let at = a.doc.fragment_target("target-section").unwrap();
        let block = a.doc.block_at_doc(carrel_core::DocByte(at));
        assert_eq!(a.view.scroll_row, a.layout.row_start(block));
    }

    #[test]
    fn help_opens_scrolls_and_closes_without_touching_the_document() {
        let mut a = app();
        update(&mut a, Action::Scroll(Span::Line, 2));
        let (row, anchor) = (a.view.scroll_row, a.view.anchor);

        assert_eq!(update(&mut a, Action::HelpToggle), Outcome::Redraw);
        assert_eq!(a.help, Some(0));
        update(&mut a, Action::Scroll(Span::Line, 3));
        assert_eq!(a.help, Some(3), "j scrolls the sheet");
        assert_eq!(a.view.scroll_row, row, "the document did not move");

        update(&mut a, Action::HelpToggle);
        assert_eq!(a.help, None);
        assert_eq!((a.view.scroll_row, a.view.anchor), (row, anchor));
    }

    #[test]
    fn q_and_esc_close_help_rather_than_acting_on_the_document() {
        let mut a = app();
        update(&mut a, Action::HelpToggle);
        assert_eq!(
            update(&mut a, Action::CloseFile),
            Outcome::Redraw,
            "q closes help, not the file"
        );
        assert_eq!(a.help, None);
        assert!(!a.is_home(), "still reading");

        update(&mut a, Action::HelpToggle);
        update(&mut a, Action::Dismiss); // Esc
        assert_eq!(a.help, None);
    }

    #[test]
    fn other_keys_are_inert_while_help_is_up() {
        let mut a = app();
        update(&mut a, Action::HelpToggle);
        assert_eq!(
            update(&mut a, Action::SearchOpen(Direction::Forward)),
            Outcome::Idle
        );
        assert!(!a.searching());
        assert_eq!(update(&mut a, Action::LinkStep(1)), Outcome::Idle);
        assert_eq!(a.selected_link, None);
    }

    #[test]
    fn help_works_on_the_home_screen_too() {
        let d = tempfile::tempdir().unwrap();
        let mut a = App::new_home(d.path().into(), vec![], 40, 10);
        assert_eq!(update(&mut a, Action::HelpToggle), Outcome::Redraw);
        assert_eq!(a.help, Some(0));
        update(&mut a, Action::HelpToggle);
        assert_eq!(a.help, None);
        assert!(a.is_home());
    }

    #[test]
    fn drag_selection_grows_copies_on_release_and_notes_the_size() {
        let mut a = app();
        // SRC starts "# T\n\nalpha beta…"; heading text "T" is doc 0..1.
        let alpha = a.doc.text.find("alpha").unwrap() as u32;
        update(&mut a, Action::SelectAnchor((alpha, alpha + 1)));
        assert_eq!(a.selection, None, "a press alone selects nothing");
        update(&mut a, Action::SelectDrag((alpha + 4, alpha + 5)));
        assert_eq!(a.selection, Some(alpha..alpha + 5), "grown to the pointer");
        update(&mut a, Action::SelectRelease);
        assert_eq!(a.clipboard.as_deref(), Some("alpha"));
        assert_eq!(a.note.as_deref(), Some("copied 5 chars"));
        assert_eq!(
            a.selection,
            Some(alpha..alpha + 5),
            "still painted after release"
        );
    }

    #[test]
    fn dragging_backwards_selects_the_same_range() {
        let mut a = app();
        let alpha = a.doc.text.find("alpha").unwrap() as u32;
        update(&mut a, Action::SelectAnchor((alpha + 4, alpha + 5)));
        update(&mut a, Action::SelectDrag((alpha, alpha + 1)));
        assert_eq!(a.selection, Some(alpha..alpha + 5));
    }

    #[test]
    fn a_new_press_replaces_the_selection_and_esc_clears_it() {
        let mut a = app();
        let alpha = a.doc.text.find("alpha").unwrap() as u32;
        update(&mut a, Action::SelectAnchor((alpha, alpha + 1)));
        update(&mut a, Action::SelectDrag((alpha + 4, alpha + 5)));
        update(&mut a, Action::SelectRelease);
        update(&mut a, Action::SelectAnchor((alpha, alpha + 1)));
        assert_eq!(a.selection, None, "a new press replaces");
        update(&mut a, Action::Dismiss);
        update(&mut a, Action::SelectAnchor((alpha, alpha + 1)));
        update(&mut a, Action::SelectDrag((alpha + 1, alpha + 2)));
        update(&mut a, Action::Dismiss);
        assert_eq!(a.selection, None, "Esc clears");
    }

    #[test]
    fn double_click_selects_the_word_under_the_pointer() {
        let mut a = app();
        let beta = a.doc.text.find("beta").unwrap() as u32;
        update(&mut a, Action::SelectWord(beta + 2));
        assert_eq!(a.selection, Some(beta..beta + 4));
        update(&mut a, Action::SelectRelease);
        assert_eq!(a.clipboard.as_deref(), Some("beta"));
    }

    #[test]
    fn triple_click_selects_the_whole_block() {
        let code = "intro\n\n```rust\nfn main() {\n    body();\n}\n```\n";
        let mut a = App::new("t.md".into(), Document::parse(code), 60, 20);
        let body = a.doc.text.find("body").unwrap() as u32;
        update(&mut a, Action::SelectBlock(body));
        update(&mut a, Action::SelectRelease);
        let copied = a.clipboard.as_deref().unwrap();
        assert!(copied.contains("fn main()"), "{copied:?}");
        assert!(copied.contains("body();"));
        assert!(!copied.contains("intro"), "only the code block");
    }

    #[test]
    fn the_selection_survives_a_resize_bit_for_bit() {
        let mut a = app();
        let alpha = a.doc.text.find("alpha").unwrap() as u32;
        update(&mut a, Action::SelectAnchor((alpha, alpha + 1)));
        update(&mut a, Action::SelectDrag((alpha + 4, alpha + 5)));
        let sel = a.selection.clone();
        a.on_resize(11, 6);
        assert_eq!(a.selection, sel, "byte ranges do not reflow");
    }

    #[test]
    fn an_oversize_selection_notes_instead_of_copying() {
        let big = "x".repeat(200_000);
        let mut a = App::new("t.md".into(), Document::parse(&big), 60, 20);
        update(&mut a, Action::SelectBlock(5));
        update(&mut a, Action::SelectRelease);
        assert_eq!(a.clipboard, None);
        assert_eq!(a.note.as_deref(), Some("selection too large to copy"));
    }

    #[test]
    fn selection_actions_are_ignored_on_the_home_screen() {
        let d = tempfile::tempdir().unwrap();
        let mut a = App::new_home(d.path().into(), vec![], 40, 10);
        assert_eq!(update(&mut a, Action::SelectAnchor((0, 1))), Outcome::Idle);
        assert_eq!(update(&mut a, Action::SelectRelease), Outcome::Idle);
        assert_eq!(a.selection, None);
    }

    const HEADINGS: &str = "# One\n\nalpha beta gamma delta epsilon zeta\n\n\
                            ## Two\n\ntext under two here\n\n# Three\n\nmore\n";

    #[test]
    fn o_opens_the_outline_preselecting_the_current_section() {
        let mut a = App::new("t.md".into(), Document::parse(HEADINGS), 30, 6);
        // Scroll until the viewport top sits under "Two".
        let two = a
            .doc
            .text
            .find("## Two")
            .map_or_else(|| a.doc.text.find("Two").unwrap() as u32, |b| b as u32);
        let block = a.doc.block_at_doc(carrel_core::DocByte(two));
        let row = a.layout.row_start(block) + 1;
        update(&mut a, Action::GoToRow(row));
        update(&mut a, Action::OutlineToggle);
        let o = a.outline.as_ref().expect("outline open");
        let heads = a.headings();
        assert_eq!(heads.len(), 3);
        assert_eq!(
            heads[o.selected], block,
            "pre-selected the section being read"
        );
    }

    #[test]
    fn the_outline_filter_narrows_and_enter_jumps_with_history() {
        let mut a = App::new("t.md".into(), Document::parse(HEADINGS), 30, 6);
        update(&mut a, Action::OutlineToggle);
        update(&mut a, Action::OutlineKey(SearchKey::Char('t')));
        update(&mut a, Action::OutlineKey(SearchKey::Char('h')));
        update(&mut a, Action::OutlineKey(SearchKey::Char('r')));
        assert_eq!(a.outline_matches().len(), 1, "only Three matches 'thr'");
        a.file = Some(PathBuf::from("/tmp/t.md"));
        update(&mut a, Action::OutlineJump);
        assert!(a.outline.is_none(), "jump closes the outline");
        let three = a.doc.text.find("Three").unwrap() as u32;
        let block = a.doc.block_at_doc(carrel_core::DocByte(three));
        assert_eq!(a.view.scroll_row, a.layout.row_start(block));
        update(&mut a, Action::Back);
        assert_eq!(a.view.scroll_row, 0, "Ctrl-O returns");
    }

    #[test]
    fn outline_escape_clears_the_filter_before_closing() {
        let mut a = App::new("t.md".into(), Document::parse(HEADINGS), 30, 6);
        update(&mut a, Action::OutlineToggle);
        update(&mut a, Action::OutlineKey(SearchKey::Char('x')));
        update(&mut a, Action::OutlineKey(SearchKey::Cancel));
        assert!(a.outline.as_ref().is_some_and(|o| o.filter.is_empty()));
        update(&mut a, Action::OutlineKey(SearchKey::Cancel));
        assert!(a.outline.is_none(), "second Esc closes");
    }

    #[test]
    fn a_document_without_headings_notes_instead_of_opening() {
        let mut a = App::new("t.md".into(), Document::parse("just prose\n"), 30, 6);
        update(&mut a, Action::OutlineToggle);
        assert!(a.outline.is_none());
        assert_eq!(a.note.as_deref(), Some("no headings in this document"));
    }

    #[test]
    fn the_overlays_are_mutually_exclusive() {
        let mut a = App::new("t.md".into(), Document::parse(HEADINGS), 30, 6);
        update(&mut a, Action::HelpToggle);
        assert_eq!(update(&mut a, Action::OutlineToggle), Outcome::Idle);
        assert!(a.outline.is_none(), "help owns the keys");
        update(&mut a, Action::HelpToggle); // close help
        update(&mut a, Action::OutlineToggle);
        assert_eq!(
            update(&mut a, Action::SearchOpen(Direction::Forward)),
            Outcome::Idle,
            "outline owns the keys"
        );
        assert!(!a.searching());
        assert_eq!(update(&mut a, Action::LinkStep(1)), Outcome::Idle);
    }

    #[test]
    fn outline_move_clamps_to_the_filtered_list() {
        let mut a = App::new("t.md".into(), Document::parse(HEADINGS), 30, 6);
        update(&mut a, Action::OutlineToggle);
        update(&mut a, Action::OutlineMove(100));
        assert_eq!(a.outline.as_ref().unwrap().selected, 2, "clamped to last");
        update(&mut a, Action::OutlineMove(-100));
        assert_eq!(a.outline.as_ref().unwrap().selected, 0);
    }

    #[test]
    fn reload_after_an_append_keeps_the_anchor_bit_for_bit() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("live.md");
        std::fs::write(&f, SRC).unwrap();
        let mut a = App::new("live.md".into(), Document::parse(SRC), 20, 6);
        a.file = Some(f.clone());
        update(&mut a, Action::Scroll(Span::Line, 3));
        let anchor = a.view.anchor;
        let rows = a.layout.total_rows();

        std::fs::write(&f, format!("{SRC}\nappended tail paragraph here\n")).unwrap();
        a.reload().unwrap();
        assert_eq!(a.view.anchor, anchor, "an append moves nothing");
        assert!(a.layout.total_rows() > rows, "the document grew");
        assert_eq!(a.note.as_deref(), Some("reloaded"));
    }

    const FOLD_SRC: &str = "\
# Alpha\n\nalpha one\n\nalpha two\n\n## Nested\n\nnested body\n\n# Beta\n\nbeta body with needle\n";

    fn heading_id(a: &App, name: &str) -> carrel_core::NodeId {
        a.doc
            .nodes
            .iter()
            .find(|n| {
                matches!(n.kind, carrel_core::NodeKind::Heading { .. })
                    && &a.doc.text[n.doc.start as usize..n.doc.end as usize] == name
            })
            .map(|n| n.id)
            .expect("heading")
    }

    #[test]
    fn folding_a_section_hides_its_rows_but_keeps_the_heading() {
        let mut a = App::new("f.md".into(), Document::parse(FOLD_SRC), 40, 10);
        let all = a.layout.total_rows();
        let alpha = heading_id(&a, "Alpha");
        a.folded.insert(alpha);
        a.relayout();
        assert!(a.layout.total_rows() < all, "the section's rows are gone");
        let hb = a.doc.block_at_doc(carrel_core::DocByte(
            a.doc.nodes[alpha.0 as usize].doc.start,
        ));
        assert!(a.layout.height(hb) > 0, "the folded heading stays visible");
        // Nested's heading is inside Alpha's span: hidden along with it.
        let nested = heading_id(&a, "Nested");
        let nb = a.doc.block_at_doc(carrel_core::DocByte(
            a.doc.nodes[nested.0 as usize].doc.start,
        ));
        assert_eq!(a.layout.height(nb), 0, "a nested heading folds away too");
    }

    #[test]
    fn folding_the_current_section_pulls_the_anchor_to_the_heading() {
        let mut a = App::new("f.md".into(), Document::parse(FOLD_SRC), 40, 6);
        let at = u32::try_from(a.doc.text.find("alpha two").unwrap()).unwrap();
        a.reveal_byte(at, 6, Where::Top);
        update(&mut a, Action::FoldToggle);
        let alpha = heading_id(&a, "Alpha");
        assert!(a.folded.contains(&alpha), "za folded the enclosing section");
        assert_eq!(
            a.view.anchor, a.doc.nodes[alpha.0 as usize].doc.start,
            "the anchor now sits on the heading"
        );
    }

    #[test]
    fn a_search_jump_unfolds_its_way_into_a_folded_section() {
        let mut a = App::new("f.md".into(), Document::parse(FOLD_SRC), 40, 10);
        update(&mut a, Action::FoldAll);
        assert!(!a.folded.is_empty());
        update(&mut a, Action::SearchOpen(Direction::Forward));
        for c in "needle".chars() {
            update(&mut a, Action::SearchKey(SearchKey::Char(c)));
        }
        update(&mut a, Action::SearchKey(SearchKey::Accept));
        let m = a.matches.as_ref().expect("matches live");
        let byte = m.ranges[m.current.unwrap()].start;
        let b = a.doc.block_at_doc(carrel_core::DocByte(byte));
        assert!(
            a.layout.height(b) > 0,
            "the jump unfolded the match's section"
        );
        let beta = heading_id(&a, "Beta");
        assert!(!a.folded.contains(&beta), "Beta opened");
        let alpha = heading_id(&a, "Alpha");
        assert!(
            a.folded.contains(&alpha),
            "Alpha stays folded — not its path"
        );
    }

    #[test]
    fn unfold_all_restores_every_row_and_reload_clears_folds() {
        let mut a = App::new("f.md".into(), Document::parse(FOLD_SRC), 40, 10);
        let all = a.layout.total_rows();
        update(&mut a, Action::FoldAll);
        assert!(a.layout.total_rows() < all);
        update(&mut a, Action::UnfoldAll);
        assert_eq!(a.layout.total_rows(), all);

        update(&mut a, Action::FoldAll);
        a.reload_from(FOLD_SRC);
        assert!(a.folded.is_empty(), "a reload's ids indexed the old parse");
        assert_eq!(a.layout.total_rows(), all);
    }

    #[test]
    fn a_completed_click_on_a_heading_toggles_its_fold() {
        let mut a = App::new("f.md".into(), Document::parse(FOLD_SRC), 40, 10);
        let alpha = heading_id(&a, "Alpha");
        let at = a.doc.nodes[alpha.0 as usize].doc.start;
        update(&mut a, Action::SelectAnchor((at, at + 1)));
        update(&mut a, Action::SelectRelease);
        assert!(a.folded.contains(&alpha), "click folds");
        update(&mut a, Action::SelectAnchor((at, at + 1)));
        update(&mut a, Action::SelectRelease);
        assert!(!a.folded.contains(&alpha), "click again unfolds");
    }

    #[test]
    fn a_drag_from_a_heading_still_selects_and_never_folds() {
        let mut a = App::new("f.md".into(), Document::parse(FOLD_SRC), 40, 10);
        let alpha = heading_id(&a, "Alpha");
        let at = a.doc.nodes[alpha.0 as usize].doc.start;
        update(&mut a, Action::SelectAnchor((at, at + 1)));
        update(&mut a, Action::SelectDrag((at + 3, at + 4)));
        update(&mut a, Action::SelectRelease);
        assert!(a.selection.is_none() || a.folded.is_empty());
        assert!(a.folded.is_empty(), "a drag is a selection, not a fold");
    }

    #[test]
    fn a_click_on_plain_text_still_folds_nothing() {
        let mut a = App::new("f.md".into(), Document::parse(FOLD_SRC), 40, 10);
        let at = u32::try_from(a.doc.text.find("alpha one").unwrap()).unwrap();
        update(&mut a, Action::SelectAnchor((at, at + 1)));
        update(&mut a, Action::SelectRelease);
        assert!(a.folded.is_empty());
    }

    // --- <details> fold like a section ---

    const DETAILS_SRC: &str = concat!(
        "# Top\n\n",
        "<details>\n<summary>Click me</summary>\n\n",
        "hidden body prose\n\n",
        "</details>\n\n",
        "after the details\n"
    );

    #[test]
    fn folding_a_details_region_hides_its_body_and_keeps_its_summary() {
        let mut a = App::new("d.md".into(), Document::parse(DETAILS_SRC), 40, 10);
        assert_eq!(a.doc.details.len(), 1, "{:?}", a.doc.details);
        update(&mut a, Action::FoldToggle); // top block is the H1 → its section
        assert!(a.folded.contains(&a.doc.nodes[0].id), "za folded Top");
        update(&mut a, Action::UnfoldAll);

        // Scroll so the summary row is at the top; za must target the region.
        let sum = a.doc.details[0].summary.start;
        a.reveal_byte(sum, a.text_h(), crate::action::Where::Top);
        update(&mut a, Action::FoldToggle);
        assert!(a.folded_details.contains(&0), "za folded the region");
        // The summary text stays on the page; the body does not.
        let sum_block = a.doc.block_at_doc(carrel_core::DocByte(sum));
        assert!(a.layout.height(sum_block) > 0, "summary visible");
        let body_at = u32::try_from(a.doc.text.find("hidden body").unwrap()).unwrap();
        let body_block = a.doc.block_at_doc(carrel_core::DocByte(body_at));
        assert_eq!(a.layout.height(body_block), 0, "body folded away");
    }

    #[test]
    fn fold_all_folds_details_too_and_a_search_unfolds_them() {
        let mut a = App::new("d.md".into(), Document::parse(DETAILS_SRC), 40, 10);
        update(&mut a, Action::FoldAll);
        assert!(a.folded_details.contains(&0), "zM folds regions");
        update(&mut a, Action::SearchOpen(Direction::Forward));
        for c in "prose".chars() {
            update(&mut a, Action::SearchKey(SearchKey::Char(c)));
        }
        update(&mut a, Action::SearchKey(SearchKey::Accept));
        let m = a.matches.as_ref().expect("matches live");
        let byte = m.ranges[m.current.unwrap()].start;
        let b = a.doc.block_at_doc(carrel_core::DocByte(byte));
        assert!(a.layout.height(b) > 0, "the jump unfolded its region");
        assert!(!a.folded_details.contains(&0));
    }

    #[test]
    fn a_click_on_a_summary_toggles_the_region() {
        let mut a = App::new("d.md".into(), Document::parse(DETAILS_SRC), 40, 10);
        let at = a.doc.details[0].summary.start + 2; // inside "Click me"
        update(&mut a, Action::SelectAnchor((at, at + 1)));
        update(&mut a, Action::SelectRelease);
        assert!(a.folded_details.contains(&0), "click folds");
        update(&mut a, Action::SelectAnchor((at, at + 1)));
        update(&mut a, Action::SelectRelease);
        assert!(!a.folded_details.contains(&0), "click again unfolds");
    }

    #[test]
    fn a_reload_clears_details_folds_with_the_rest() {
        let mut a = App::new("d.md".into(), Document::parse(DETAILS_SRC), 40, 10);
        let all = a.layout.total_rows();
        update(&mut a, Action::FoldAll);
        assert!(a.layout.total_rows() < all, "details bodies went away");
        a.reload_from(DETAILS_SRC);
        assert!(a.folded_details.is_empty(), "indices indexed the old parse");
        assert_eq!(a.layout.total_rows(), all);
    }

    // --- footnote % jumps ---

    /// Long enough to scroll: `Where::Top` must be able to land without
    /// clamping, which a five-row fixture never exercises.
    fn foot_src() -> String {
        // Definitions mid-document, with room after: `Where::Top` must land
        // without end-clamping, which a target in the last screenful never
        // allows.
        let mut s = String::new();
        for i in 0..60 {
            let _ = write!(s, "filler paragraph number {i}\n\n");
        }
        s.push_str("intro[^1] more[^2]\n\ntail\n\n[^1]: first note\n\n[^2]: second note\n\n");
        for i in 0..60 {
            let _ = write!(s, "afterward paragraph number {i}\n\n");
        }
        s
    }

    #[test]
    fn percent_jumps_from_a_reference_to_its_definition() {
        let src = foot_src();
        let mut a = App::new("f.md".into(), Document::parse(&src), 40, 40);
        a.file = Some(std::path::PathBuf::from("f.md"));
        // Stand on the first reference itself.
        let ref1 = a.doc.footnote_refs()[0].1;
        a.reveal_byte(ref1, a.text_h(), crate::action::Where::Middle);
        update(&mut a, Action::FootnoteJump);
        let def1 = a.doc.footnote_defs()[0].1;
        assert_eq!(a.view.anchor, def1, "landed at the definition");
        assert_eq!(a.history.len(), 1, "the jump pushed an undo entry");
        let top = a
            .doc
            .node_for_block(a.layout.block_at_row(a.view.scroll_row));
        assert_eq!(&*top.prefix.as_ref().expect("def label").text, "[^1]: ");
    }

    #[test]
    fn percent_round_trips_and_ctrl_o_returns() {
        let src = foot_src();
        let mut a = App::new("f.md".into(), Document::parse(&src), 40, 40);
        a.file = Some(std::path::PathBuf::from("f.md"));
        let def1 = a.doc.footnote_defs()[0].1;
        update(&mut a, Action::FootnoteJump); // top of doc wraps to pair one
        assert_eq!(a.view.anchor, def1);
        update(&mut a, Action::FootnoteJump); // inside the definition: back
        let ref1 = a.doc.footnote_refs()[0].1;
        assert!(
            a.view.anchor <= ref1,
            "returned above the reference, got {}",
            a.view.anchor
        );
        // Ctrl-O undoes each leg exactly.
        update(&mut a, Action::Back);
        assert_eq!(a.view.anchor, def1, "back at the definition");
        update(&mut a, Action::Back);
        assert_eq!(a.view.anchor, 0, "back at the very start");
    }

    #[test]
    fn percent_on_a_document_without_footnotes_says_so() {
        let mut a = App::new("f.md".into(), Document::parse("plain prose only\n"), 40, 10);
        assert_eq!(update(&mut a, Action::FootnoteJump), Outcome::Redraw);
        assert!(
            a.note
                .as_deref()
                .is_some_and(|n| n.contains("no footnotes"))
        );
    }

    // --- forward links (l) ---

    #[test]
    fn the_forward_pane_lists_local_and_external_and_skips_fragments() {
        let src = concat!(
            "a [local](neighbour.md), a [wiki]([[Neighbour]]), ",
            "an [outside](https://example.com/x), and [here](#section).\n"
        );
        let mut a = App::new("f.md".into(), Document::parse(src), 40, 10);
        update(&mut a, Action::ForwardToggle);
        let f = a.forward.as_ref().expect("pane open");
        assert_eq!(f.rows.len(), 3, "fragments are not destinations");
        assert_eq!(
            f.rows[0].target.as_deref(),
            Some(std::path::Path::new("./neighbour.md"))
        );
        assert!(f.rows[1].dest.contains("Neighbour"), "{}", f.rows[1].dest);
        assert!(f.rows[2].target.is_none(), "external is never fetchable");
        assert_eq!(f.rows[0].label.as_deref(), Some("local"));
    }

    /// Was `opening_an_external_forward_link_says_so_and_fetches_nothing`,
    /// which asserted the note "external — carrel does not fetch". The
    /// destination now opens; what has NOT changed, and is what the old name
    /// was really guarding, is that carrel still fetches nothing — the URL
    /// leaves through the outbox and no byte of it is ever read back.
    #[test]
    fn opening_an_external_forward_link_hands_it_to_the_browser_and_fetches_nothing() {
        let src = "see [that](https://example.com/elsewhere)\n";
        let mut a = App::new("f.md".into(), Document::parse(src), 40, 10);
        update(&mut a, Action::ForwardToggle);
        assert_eq!(update(&mut a, Action::ForwardOpen), Outcome::Redraw);
        assert_eq!(
            a.open_url.take().as_deref(),
            Some("https://example.com/elsewhere")
        );
        assert_eq!(a.note.as_deref(), Some("opened in your browser"));
        assert!(a.forward.is_some(), "the pane stays open");
    }

    /// The allowlist, item by item.
    ///
    /// Written before the opener existed and watched to fail: this is the
    /// only thing standing between a document carrel did not write and the
    /// desktop's handler table, so it is tested as a table rather than by
    /// example.
    #[test]
    fn only_http_https_and_mailto_are_openable() {
        for ok in [
            "http://example.com",
            "https://example.com/a/b?c=d#e",
            "HTTPS://EXAMPLE.COM",
            "mailto:someone@example.com",
        ] {
            assert_eq!(openable_url(ok), Some(ok), "{ok} should open");
        }
        for no in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:text/html,<script>x</script>",
            "ssh://host",
            "vscode://file/etc/passwd",
            "//example.com",             // scheme-relative: no scheme at all
            "https:",                    // no host
            "https://",                  // still no host
            "mailto:",                   // no address
            "notes/architecture.md",     // a local path is not a URL
            "http://exa mple.com",       // whitespace never survives argv
            "https://example.com/\u{7}", // a control character, ever
        ] {
            assert_eq!(openable_url(no), None, "{no:?} must not open");
        }
        let long = format!("https://example.com/{}", "a".repeat(MAX_URL));
        assert_eq!(openable_url(&long), None, "past the cap");
    }

    #[test]
    fn following_an_external_link_fills_the_browser_outbox() {
        let src = "see [that](https://example.com/x)\n";
        let mut a = App::new("f.md".into(), Document::parse(src), 40, 10);
        update(&mut a, Action::LinkStep(1));
        assert_eq!(update(&mut a, Action::LinkFollow), Outcome::Redraw);
        assert_eq!(a.open_url.take().as_deref(), Some("https://example.com/x"));
        assert_eq!(a.note.as_deref(), Some("opened in your browser"));
    }

    /// A refusal must be a refusal, not a fallback to "treat it as a path".
    #[test]
    fn a_refused_scheme_opens_nothing_and_says_so() {
        let src = "see [that](javascript:alert(1))\n";
        let mut a = App::new("f.md".into(), Document::parse(src), 40, 10);
        update(&mut a, Action::LinkStep(1));
        update(&mut a, Action::LinkFollow);
        assert!(a.open_url.is_none(), "nothing may reach the outbox");
        assert_eq!(
            a.note.as_deref(),
            Some("carrel opens http, https and mailto links only")
        );
    }

    /// The click does in one intent what Tab-then-Enter does in two.
    #[test]
    fn clicking_a_link_opens_it_without_selecting_it_first() {
        let src = "one [a](https://example.com/a) two [b](https://example.com/b)\n";
        let mut a = App::new("f.md".into(), Document::parse(src), 60, 10);
        assert!(a.selected_link.is_none(), "nothing selected yet");
        assert_eq!(update(&mut a, Action::LinkOpen(1)), Outcome::Redraw);
        assert_eq!(a.open_url.take().as_deref(), Some("https://example.com/b"));
    }

    /// A target from a frame that no longer describes the document — the
    /// reload race — must be inert rather than panicking on an index.
    #[test]
    fn link_open_past_the_end_is_inert() {
        let src = "one [a](https://example.com/a)\n";
        let mut a = App::new("f.md".into(), Document::parse(src), 40, 10);
        assert_eq!(update(&mut a, Action::LinkOpen(99)), Outcome::Idle);
        assert!(a.open_url.is_none());
    }

    /// The marker and the words beside it must fold the same thing, or the
    /// glyph is lying about what it does.
    #[test]
    fn the_fold_marker_folds_exactly_what_the_heading_row_folds() {
        let src = "# One\n\nbody\n\n# Two\n\nmore\n";
        let mut a = App::new("f.md".into(), Document::parse(src), 40, 20);
        let head = a.doc.node_for_block(carrel_core::BlockIdx(0)).doc.start;

        assert_eq!(update(&mut a, Action::FoldAt(head)), Outcome::Redraw);
        let folded_by_marker: Vec<_> = a.folded.iter().copied().collect();
        assert_eq!(folded_by_marker.len(), 1, "one section folded");

        // The same byte through the click-on-the-row path.
        update(&mut a, Action::FoldAt(head));
        assert!(a.folded.is_empty(), "and the same byte unfolds it again");
    }

    #[test]
    fn a_fold_marker_click_on_nothing_foldable_is_inert() {
        let src = "just a paragraph\n";
        let mut a = App::new("f.md".into(), Document::parse(src), 40, 10);
        assert_eq!(update(&mut a, Action::FoldAt(2)), Outcome::Idle);
    }

    /// A click in a pane already said which row it meant, so it selects and
    /// opens in one gesture — and it must open the row the pointer was on,
    /// not the row the keyboard cursor happened to be resting on.
    #[test]
    fn clicking_an_outline_row_jumps_to_that_row_not_the_selected_one() {
        // Tall sections, and a short window: on a document that fits the
        // screen the view cannot move, and "did it jump?" has no answer.
        let filler = "para\n\n".repeat(20);
        let src = format!("# Alpha\n\n{filler}# Beta\n\n{filler}# Gamma\n\n{filler}");
        let mut a = App::new("f.md".into(), Document::parse(&src), 60, 8);
        update(&mut a, Action::OutlineToggle);
        assert_eq!(a.outline.as_ref().unwrap().selected, 0, "cursor on Alpha");

        assert_eq!(update(&mut a, Action::OutlineJumpAt(2)), Outcome::Redraw);
        assert!(a.outline.is_none(), "the picker closes on a jump");
        let landed = a.doc.block_at_doc(carrel_core::DocByte(a.view.anchor));
        let text = a.doc.block_text(landed);
        assert!(text.contains("Gamma"), "landed on {text:?}, not Gamma");
    }

    #[test]
    fn clicking_a_bookmark_row_jumps_to_that_bookmark() {
        // Tall enough that `GoToEnd` actually moves: on a document that fits
        // the window the second toggle would land on the first mark and
        // REMOVE it, leaving nothing to click.
        let body = "para\n\n".repeat(40);
        let src = format!("# One\n\n{body}# Two\n\nlast\n");
        let mut a = App::new("f.md".into(), Document::parse(&src), 60, 8);
        update(&mut a, Action::MarkToggle); // mark the top
        update(&mut a, Action::GoToEnd);
        update(&mut a, Action::MarkToggle); // and the end
        update(&mut a, Action::MarkListToggle);
        assert_eq!(a.marks.len(), 2);

        assert_eq!(update(&mut a, Action::MarkListJumpAt(1)), Outcome::Redraw);
        assert_eq!(a.view.anchor, a.marks[1], "landed on the row clicked");
        assert!(a.mark_list.is_none(), "the list closes behind it");
    }

    /// A target left over from a frame that no longer describes the pane —
    /// the row vanished, the list refiltered — must be inert, never a panic
    /// and never the wrong row.
    #[test]
    fn a_pane_row_click_past_the_end_is_inert() {
        let src = "# One\n\nbody\n";
        let mut a = App::new("f.md".into(), Document::parse(src), 60, 20);
        update(&mut a, Action::OutlineToggle);
        assert_eq!(update(&mut a, Action::OutlineJumpAt(99)), Outcome::Idle);
        assert!(a.outline.is_some(), "and the picker stays open");

        assert_eq!(update(&mut a, Action::MarkListJumpAt(0)), Outcome::Idle);
        assert_eq!(update(&mut a, Action::BacklinksOpenAt(0)), Outcome::Idle);
        assert_eq!(update(&mut a, Action::ForwardOpenAt(0)), Outcome::Idle);
    }

    #[test]
    fn clicking_a_forward_link_row_opens_that_row() {
        let src = "see [a](https://example.com/a) and [b](https://example.com/b)\n";
        let mut a = App::new("f.md".into(), Document::parse(src), 60, 20);
        update(&mut a, Action::ForwardToggle);
        assert_eq!(a.forward.as_ref().unwrap().rows.len(), 2);

        assert_eq!(update(&mut a, Action::ForwardOpenAt(1)), Outcome::Redraw);
        assert_eq!(a.open_url.take().as_deref(), Some("https://example.com/b"));
    }

    /// A click a pane swallowed changes nothing at all.
    #[test]
    fn an_absorbed_click_does_nothing() {
        let src = "# One\n\nbody\n";
        let mut a = App::new("f.md".into(), Document::parse(src), 60, 20);
        let before = a.view.anchor;
        assert_eq!(update(&mut a, Action::Absorb), Outcome::Idle);
        assert_eq!(a.view.anchor, before);
        assert!(a.selection.is_none(), "and starts no selection");
    }

    #[test]
    fn duplicate_destinations_collapse_to_one_row() {
        let src = "one [x](dup.md), two [y](dup.md)\n";
        let mut a = App::new("f.md".into(), Document::parse(src), 40, 10);
        update(&mut a, Action::ForwardToggle);
        assert_eq!(a.forward.as_ref().unwrap().rows.len(), 1);
    }

    // --- document info card (I) ---

    #[test]
    fn the_info_card_toggles_and_counts_the_document() {
        let src = "# Title\n\npara one[^a]\n\n```rust\nlet x;\n```\n\n| t |\n|---|\n| c |\n\n[^a]: note\n";
        let mut a = App::new("stats.md".into(), Document::parse(src), 40, 10);
        assert!(!a.info);
        update(&mut a, Action::InfoToggle);
        assert!(a.info);
        let rows = a.info_rows();
        let get = |k: &str| {
            rows.iter()
                .find(|(l, _)| *l == k)
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        };
        assert_eq!(get("document"), "stats.md");
        assert_eq!(get("headings"), "1");
        assert_eq!(get("code blocks"), "1");
        assert_eq!(get("tables"), "1");
        assert_eq!(get("footnotes"), "1 refs · 1 notes");
        assert_eq!(get("bookmarks"), "0");
        update(&mut a, Action::InfoToggle);
        assert!(!a.info);
    }

    #[test]
    fn a_piped_document_reports_itself_as_stdin_with_no_mtime() {
        let a = App::new(String::new(), Document::parse("hello world\n"), 40, 10);
        assert_eq!(
            a.info_rows()
                .iter()
                .find(|(l, _)| *l == "document")
                .map(|(_, v)| v.clone()),
            Some("(stdin)".into())
        );
        assert!(a.mtime.is_none());
    }

    #[test]
    fn epoch_formats_as_a_civil_date() {
        assert_eq!(format_epoch(0), "1970-01-01 00:00");
        assert_eq!(format_epoch(59), "1970-01-01 00:00", "seconds never shown");
        assert_eq!(format_epoch(86_400 * 19_000 + 3_600), "2022-01-08 01:00");
    }

    // --- task jumping (X) ---

    #[test]
    fn x_jumps_through_tasks_and_wraps() {
        let mut src = String::new();
        for i in 0..30 {
            let _ = writeln!(src, "filler {i}");
            src.push_str("\n\n");
        }
        src.push_str("- [ ] one\n\n- [ ] two\n\n- [x] three done\n\n");
        for i in 0..30 {
            let _ = writeln!(src, "tail {i}");
            src.push_str("\n\n");
        }
        let mut a = App::new("t.md".into(), Document::parse(&src), 40, 40);
        // Four presses over three tasks: every one lands on A task, says
        // which and how many, and by press four the cycle has wrapped.
        let mut seen = Vec::new();
        for _ in 0..4 {
            update(&mut a, Action::TaskStep(1));
            let note = a.note.clone().expect("a note per jump");
            let idx: usize = note
                .split_whitespace()
                .nth(1)
                .and_then(|w| w.parse().ok())
                .expect("task N of M");
            seen.push(idx);
            let top = a.layout.block_at_row(a.view.scroll_row);
            assert!(
                a.doc
                    .node_for_block(top)
                    .prefix
                    .as_ref()
                    .is_some_and(|p| p.task.is_some()),
                "landed on a task: {note}"
            );
        }
        // The ORDER, not just the distinctness — a set of three passed while
        // `X` was walking 2, 1, 3, 2 and skipping the first task entirely.
        assert_eq!(seen, vec![1, 2, 3, 1], "X must walk tasks in order");

        // And backwards, from the top: the last task, then back through them.
        let mut a = App::new("t.md".into(), Document::parse(&src), 40, 40);
        let mut back = Vec::new();
        for _ in 0..4 {
            update(&mut a, Action::TaskStep(-1));
            let note = a.note.clone().expect("a note per jump");
            back.push(
                note.split_whitespace()
                    .nth(1)
                    .and_then(|w| w.parse::<usize>().ok())
                    .expect("task N of M"),
            );
        }
        assert_eq!(back, vec![3, 2, 1, 3], "N-style stepping walks back");
    }

    #[test]
    fn x_on_a_document_without_tasks_says_so() {
        let mut a = App::new("t.md".into(), Document::parse("nothing here\n"), 40, 10);
        assert_eq!(update(&mut a, Action::TaskStep(1)), Outcome::Redraw);
        assert!(
            a.note
                .as_deref()
                .is_some_and(|n| n.contains("no task lists")),
            "{:?}",
            a.note
        );
    }

    // --- auto-read (A) ---

    #[test]
    fn auto_read_ticks_drift_a_row_and_any_motion_stops_them() {
        let mut src = String::new();
        for i in 0..40 {
            let _ = writeln!(src, "filler {i}");
            src.push('\n');
        }
        let mut a = App::new("a.md".into(), Document::parse(&src), 40, 40);
        update(&mut a, Action::AutoToggle);
        assert!(a.auto_read);

        let before = a.view.scroll_row;
        assert_eq!(update(&mut a, Action::AutoTick), Outcome::Redraw);
        assert_eq!(a.view.scroll_row, before + 1, "one row per heartbeat");

        // The reader takes the wheel: any deliberate scroll detaches.
        update(&mut a, Action::Scroll(crate::action::Span::Line, -2));
        assert!(!a.auto_read);
        let frozen = a.view.scroll_row;
        assert_eq!(update(&mut a, Action::AutoTick), Outcome::Idle, "inert now");
        assert_eq!(a.view.scroll_row, frozen);
    }

    #[test]
    fn auto_read_stops_itself_at_the_end_of_the_document() {
        let mut a = App::new("a.md".into(), Document::parse("top\n\nbottom\n"), 40, 10);
        update(&mut a, Action::GoToEnd);
        update(&mut a, Action::AutoToggle);
        assert!(a.auto_read);
        // Already pinned to the end: the first tick has nowhere to go.
        update(&mut a, Action::AutoTick);
        assert!(!a.auto_read, "the end stops it");
        assert!(
            a.note.as_deref().is_some_and(|n| n.contains("the end")),
            "{:?}",
            a.note
        );
    }

    // --- the bookmark list (") ---

    #[test]
    fn the_outline_picker_ranks_fuzzy_matches_best_first() {
        // Both headings carry r-s-t in order; the tight one ranks first.
        let src = "# Results\n\nx\n\n# Rust\n\ny\n";
        let mut a = App::new("o.md".into(), Document::parse(src), 40, 10);
        update(&mut a, Action::OutlineToggle);
        for c in "rst".chars() {
            update(&mut a, Action::OutlineKey(SearchKey::Char(c)));
        }
        let matches = a.outline_matches();
        assert_eq!(matches.len(), 2);
        let first = a.doc.node_for_block(matches[0]);
        assert_eq!(
            &a.doc.text[first.doc.start as usize..first.doc.end as usize],
            "Rust",
            "the tighter run outranks the gappy one"
        );
    }

    #[test]
    fn the_mark_list_opens_preselected_and_jumps() {
        let src = "# One\n\nfirst body\n\n# Two\n\nsecond body\n";
        let mut a = App::new("m.md".into(), Document::parse(src), 40, 10);
        a.file = Some(std::path::PathBuf::from("m.md"));
        // Seed two marks at real block starts; a fixture this small cannot
        // scroll, so driving them through the view would double-mark one.
        let b0 = a.doc.node_for_block(BlockIdx(0)).doc.start;
        let b2 = a.doc.node_for_block(BlockIdx(2)).doc.start;
        // With no marks, the list declines to open and says why.
        update(&mut a, Action::MarkListToggle);
        assert!(
            a.note
                .as_deref()
                .is_some_and(|n| n.contains("no bookmarks"))
        );
        a.marks = vec![b0, b2];

        update(&mut a, Action::MarkListToggle);
        let sel = a.mark_list.expect("list open");
        // The view tops out on a fixture this small, so it sits above both
        // marks — the first one is "at or after" it.
        assert_eq!(a.marks[sel], b0, "pre-selected at or after the view");

        update(&mut a, Action::MarkListMove(-1));
        update(&mut a, Action::MarkListJump);
        assert!(a.mark_list.is_none(), "jump closes the list");
        assert_eq!(a.history.len(), 1, "Ctrl-O can undo the jump");
        let top = a.layout.block_at_row(a.view.scroll_row);
        assert_eq!(
            a.doc.node_for_block(top).doc.start,
            a.marks[0],
            "landed on the marked block"
        );
    }

    #[test]
    fn the_mark_list_cursor_saturates_and_clamps() {
        let mut a = App::new("m.md".into(), Document::parse("one\ntwo\n"), 40, 10);
        a.marks = vec![0, 8];
        update(&mut a, Action::MarkListToggle);
        update(&mut a, Action::MarkListMove(-9));
        assert_eq!(a.mark_list, Some(0), "saturates at the first");
        update(&mut a, Action::MarkListMove(9));
        assert_eq!(a.mark_list, Some(1), "clamps at the last");
        update(&mut a, Action::MarkListToggle);
        assert!(a.mark_list.is_none());
    }

    #[test]
    fn clearing_marks_under_the_open_list_keeps_the_cursor_honest() {
        let mut a = App::new("m.md".into(), Document::parse("one\ntwo\nthree\n"), 40, 10);
        a.marks = vec![0, 4, 10];
        update(&mut a, Action::MarkListToggle);
        update(&mut a, Action::MarkListMove(2));
        assert_eq!(a.mark_list, Some(2));
        // A mark cleared under the pane shrinks the rows; the cursor
        // re-clamps against the live list rather than pointing past it.
        a.marks.remove(2);
        update(&mut a, Action::MarkListMove(1));
        assert_eq!(a.mark_list, Some(1), "re-clamped to the new last");
    }

    #[test]
    fn za_outside_any_section_notes_instead_of_folding() {
        let mut a = App::new(
            "f.md".into(),
            Document::parse("plain text, no headings anywhere\n"),
            40,
            6,
        );
        update(&mut a, Action::FoldToggle);
        assert!(a.folded.is_empty());
        assert!(a.note.is_some(), "the note says why nothing happened");
    }

    #[test]
    fn toggling_the_breadcrumb_relayouts_and_persists_only_when_injected() {
        let cfg = tempfile::tempdir().unwrap();
        let mut a = App::new("x.md".into(), Document::parse("# H\n\nbody\n"), 40, 12);
        a.config_dir = Some(cfg.path().into());
        let tall = a.text_h();
        update(&mut a, Action::BreadcrumbToggle);
        assert!(!a.breadcrumb);
        assert_eq!(a.text_h(), tall + 1, "the band's row comes back to text");
        assert_eq!(
            crate::config::load_breadcrumb_in(cfg.path()),
            Some(false),
            "the choice persists into the injected dir"
        );
        update(&mut a, Action::BreadcrumbToggle);
        assert_eq!(a.text_h(), tall);
        assert_eq!(crate::config::load_breadcrumb_in(cfg.path()), Some(true));
    }

    #[test]
    fn the_band_costs_one_text_row_and_moves_the_top_edge() {
        let with_headings = Document::parse("# H\n\nbody\n");
        let a = App::new("x.md".into(), with_headings, 40, 12);
        assert!(a.band(), "breadcrumb defaults on and headings exist");
        assert_eq!(a.text_y(), 2, "crumb row + rule row");

        let plain = Document::parse("no headings here\n");
        let b = App::new("x.md".into(), plain, 40, 12);
        assert!(!b.band(), "no headings, no band");
        assert_eq!(b.text_y(), PAD_TOP);
        assert_eq!(
            a.text_h() + 1,
            b.text_h(),
            "the band replaces the pad row and adds one more"
        );
    }

    #[test]
    fn clicks_map_through_the_top_edge_in_both_band_states() {
        // Same document, band toggled: the first text row must hit the same
        // byte both ways. A one-row offset here is invisible to frame tests.
        let mut a = App::new("x.md".into(), Document::parse("# H\n\nabcdef\n"), 40, 12);
        let x = a.text_x_now();
        let with_band = a.doc_span_at(x, a.text_y());
        assert!(with_band.is_some(), "first text row hits content");
        a.breadcrumb = false;
        let without = a.doc_span_at(x, a.text_y());
        assert_eq!(
            with_band, without,
            "the same first visible cluster, either chrome"
        );
        assert_eq!(
            a.doc_span_at(x, 0),
            None,
            "the crumb row itself is not text"
        );
    }

    #[test]
    fn reload_from_appends_without_touching_the_anchor() {
        let mut a = App::new("(stdin)".into(), Document::parse(SRC), 20, 6);
        update(&mut a, Action::Scroll(Span::Line, 3));
        let anchor = a.view.anchor;
        let rows = a.layout.total_rows();
        a.reload_from(&format!("{SRC}\nappended tail paragraph here\n"));
        assert_eq!(a.view.anchor, anchor, "an append moves nothing");
        assert!(a.layout.total_rows() > rows, "the document grew");
        assert_eq!(a.note, None, "a streamed chunk is not a 'reloaded' event");
    }

    // --- bookmarks (2026-08-21) ---

    fn marked_app() -> App {
        let body: String = (0..40).fold(String::new(), |mut b, i| {
            use std::fmt::Write as _;
            let _ = write!(b, "Para {i} here.\n\n");
            b
        });
        App::new("t.md".into(), Document::parse(&body), 40, 8)
    }

    #[test]
    fn a_bookmark_toggles_and_lands_on_a_block_not_a_scroll_offset() {
        let mut a = marked_app();
        update(&mut a, Action::Scroll(Span::Line, 6));
        update(&mut a, Action::MarkToggle);
        assert_eq!(a.marks.len(), 1);
        let at = a.marks[0];
        // It sits on a block start, so `zz` and a resize cannot drift it off
        // the thing it marks.
        assert!(
            a.doc.nodes.iter().any(|n| n.doc.start == at),
            "the mark is not on a block boundary"
        );

        update(&mut a, Action::MarkToggle);
        assert!(a.marks.is_empty(), "the same key clears it");
    }

    #[test]
    fn the_quote_key_walks_the_bookmarks_and_wraps() {
        let mut a = marked_app();
        update(&mut a, Action::Scroll(Span::Line, 4));
        update(&mut a, Action::MarkToggle);
        update(&mut a, Action::Scroll(Span::Line, 20));
        update(&mut a, Action::MarkToggle);
        assert_eq!(a.marks.len(), 2);
        let (first, second) = (a.marks[0], a.marks[1]);

        update(&mut a, Action::GoToStart);
        update(&mut a, Action::MarkNext);
        let here = |a: &App| {
            a.doc
                .node_for_block(a.layout.block_at_row(a.view.scroll_row))
                .doc
                .start
        };
        assert_eq!(here(&a), first);
        update(&mut a, Action::MarkNext);
        assert_eq!(here(&a), second);
        update(&mut a, Action::MarkNext);
        assert_eq!(here(&a), first, "it wraps rather than stopping");
    }

    #[test]
    fn a_bookmark_survives_a_resize_because_it_is_a_doc_byte() {
        let mut a = marked_app();
        update(&mut a, Action::Scroll(Span::Line, 8));
        update(&mut a, Action::MarkToggle);
        let at = a.marks[0];
        a.on_resize(24, 8);
        assert_eq!(a.marks, vec![at], "reflow cannot move a doc byte");
        update(&mut a, Action::GoToStart);
        update(&mut a, Action::MarkNext);
        assert_eq!(
            a.doc
                .node_for_block(a.layout.block_at_row(a.view.scroll_row))
                .doc
                .start,
            at,
            "and it still lands on the same block at the new width"
        );
    }

    #[test]
    fn the_quote_key_says_so_when_there_is_nothing_to_go_to() {
        let mut a = marked_app();
        update(&mut a, Action::MarkNext);
        assert!(a.note.is_some());
    }

    // --- the diff adapter's policy (2026-08-21) ---

    const RAW_DIFF: &str = "\
diff --git a/x.rs b/x.rs
--- a/x.rs
+++ b/x.rs
@@ -1,2 +1,2 @@
-old
+new
";

    #[test]
    fn a_pipe_is_adapted_and_a_markdown_file_is_never_sniffed() {
        // The safety argument in one test: the SAME bytes are a diff on a
        // pipe and prose in a `.md` file.
        let piped = adapt(RAW_DIFF, true);
        assert!(
            piped
                .nodes
                .iter()
                .any(|n| matches!(n.kind, carrel_core::NodeKind::Heading { .. })),
            "a pipe should have become sections: {:?}",
            piped.text
        );

        // Not adapted: no sections, and the diff stays one run of prose.
        // (`--` becomes an en-dash here — smart punctuation, which is what
        // markdown parsing of a diff looks like and exactly why `.md` files
        // are never sniffed.)
        let as_markdown = adapt(RAW_DIFF, false);
        assert!(
            !as_markdown
                .nodes
                .iter()
                .any(|n| matches!(n.kind, carrel_core::NodeKind::Heading { .. })),
            "a .md file must not be restructured: {:?}",
            as_markdown.text
        );
        assert!(
            !as_markdown
                .nodes
                .iter()
                .any(|n| matches!(n.kind, carrel_core::NodeKind::CodeBlock { .. })),
            "nor fenced: {:?}",
            as_markdown.text
        );
    }

    #[test]
    fn a_markdown_document_about_diffs_survives_the_reader() {
        // The false-positive that the never-sniff-a-.md rule exists to stop.
        let doc = adapt(
            "# On diffs\n\nRun `git show`. Output starts `diff --git a/x b/x`.\n",
            false,
        );
        assert!(doc.text.contains("diff --git a/x b/x"), "{:?}", doc.text);
    }

    #[test]
    fn a_streamed_diff_is_adapted_on_every_append() {
        let mut a = App::new("(stdin)".into(), Document::parse(""), 60, 20);
        a.file = None;
        a.diff_ok = true;
        // Half a diff arrives…
        a.reload_from("diff --git a/x.rs b/x.rs\n@@ -1 +1 @@\n-old\n");
        let first = a.doc.block_count();
        // …then the rest.
        a.reload_from(RAW_DIFF);
        assert!(a.doc.block_count() >= first, "it re-parsed");
        assert!(
            a.doc
                .nodes
                .iter()
                .any(|n| matches!(n.kind, carrel_core::NodeKind::Heading { .. })),
            "still adapted after the append: {:?}",
            a.doc.text
        );
    }

    // --- follow mode and the block cursor (2026-08-21) ---

    fn streaming_app() -> App {
        let mut a = App::new(
            "(stdin)".into(),
            Document::parse(&"a line\n\n".repeat(60)),
            40,
            10,
        );
        a.file = None;
        a.streaming = true;
        a
    }

    #[test]
    fn following_pins_the_view_to_the_end_as_the_document_grows() {
        let mut a = streaming_app();
        update(&mut a, Action::FollowToggle);
        assert!(a.following);
        let end = a.layout.max_scroll(a.text_h());
        assert_eq!(a.view.scroll_row, end, "F goes to the end at once");

        // Appending must keep it there. `poll_stream` applies this in the
        // event loop; the state half is what a test can reach.
        a.reload_from(&"a line\n\n".repeat(120));
        let h = a.text_h();
        a.view.scroll_to(&a.doc, &a.layout, u32::MAX, h);
        assert_eq!(a.view.scroll_row, a.layout.max_scroll(h));
        assert!(a.view.scroll_row > end, "the document really grew");
    }

    #[test]
    fn a_deliberate_move_detaches_but_an_incidental_one_does_not() {
        let mut a = streaming_app();
        update(&mut a, Action::FollowToggle);
        assert!(a.following);
        update(&mut a, Action::Scroll(Span::Line, -1));
        assert!(!a.following, "scrolling up detaches");

        update(&mut a, Action::FollowToggle);
        update(&mut a, Action::ThemeCycle);
        assert!(a.following, "cycling a theme is not a move");
        update(&mut a, Action::HintsToggle);
        assert!(a.following, "nor is folding the hints");

        // Scrolling DOWN is not a detach either — it is going the same way.
        update(&mut a, Action::Scroll(Span::Line, 1));
        assert!(a.following);
    }

    #[test]
    fn g_follows_only_while_the_document_is_still_growing() {
        let mut a = streaming_app();
        update(&mut a, Action::GoToEnd);
        assert!(a.following, "G on a growing document means keep me here");

        let mut b = streaming_app();
        b.streaming = false;
        update(&mut b, Action::GoToEnd);
        assert!(!b.following, "G on a finished document is just a jump");
    }

    #[test]
    fn the_code_cursor_steps_code_blocks_only_and_yank_copies_one() {
        let src = "intro\n\n```sh\nfirst cmd\n```\n\nprose between\n\n```sh\nsecond cmd\n```\n";
        let mut a = App::new("t.md".into(), Document::parse(src), 40, 20);
        update(&mut a, Action::CodeStep(1));
        let one = a.code_focus.expect("a first code block");
        assert!(matches!(
            a.doc.node_for_block(one).kind,
            carrel_core::NodeKind::CodeBlock { .. }
        ));

        update(&mut a, Action::CodeStep(1));
        let two = a.code_focus.expect("a second code block");
        assert_ne!(one, two, "it stepped past the prose, not into it");
        assert!(matches!(
            a.doc.node_for_block(two).kind,
            carrel_core::NodeKind::CodeBlock { .. }
        ));

        update(&mut a, Action::YankBlock);
        assert_eq!(
            a.clipboard.take().as_deref(),
            Some("second cmd\n"),
            "the block's text, no fence"
        );
        assert_eq!(a.note.as_deref(), Some("copied 1 line"));

        // Past the last one, the cursor says so rather than moving.
        update(&mut a, Action::CodeStep(1));
        assert_eq!(a.code_focus, Some(two));
        assert!(a.note.is_some());
    }

    #[test]
    fn a_document_change_forgets_the_block_cursor_and_the_selection() {
        // Both of these index the OLD parse. `code_focus` surviving an open
        // was a reachable panic: step to a late code block, follow a link to
        // a shorter document, press `y`, and `node_for_block` indexed a block
        // that no longer existed. The selection surviving was quieter — the
        // copy took bytes from the new document at the old offsets.
        let src =
            "intro\n\n```sh\none\n```\n\nmid\n\n```sh\ntwo\n```\n\nmore\n\n```sh\nthree\n```\n";
        let d = tempfile::tempdir().unwrap();
        let small = d.path().join("small.md");
        std::fs::write(&small, "# Small\n").unwrap();

        for step in 1..=3 {
            let mut a = App::new("t.md".into(), Document::parse(src), 40, 20);
            for _ in 0..step {
                update(&mut a, Action::CodeStep(1));
            }
            let focused = a.code_focus.expect("a code block is focused");
            a.selection = Some(0..5);
            a.sel_anchor = Some((0, 5));

            a.open_path(&small).expect("the small document opens");

            assert_eq!(a.code_focus, None, "the block cursor indexed the old parse");
            assert_eq!(a.selection, None, "the selection indexed the old text");
            assert_eq!(a.sel_anchor, None);

            // The reported crash, exactly: `y` with a stale cursor.
            update(&mut a, Action::YankBlock);
            assert!(
                focused.get() < 12,
                "sanity: the old index was a real block in the old document"
            );
        }
    }

    #[test]
    fn only_regular_files_are_opened_as_documents() {
        // A FIFO, /dev/zero and most of /proc report length zero, so the size
        // guard passed and the unbounded read behind it ran anyway: reading a
        // FIFO blocked forever with the terminal in raw mode and the event
        // loop stalled, and /dev/zero read until the OOM killer arrived.
        // `[x](./pipe.md)` in any markdown file was enough to reach it.
        #[cfg(unix)]
        {
            let e = check_document_size(Path::new("/dev/null"))
                .expect_err("a character device is not a document");
            assert!(e.to_string().contains("not a regular file"), "{e}");
            assert!(read_document(Path::new("/dev/zero")).is_err());
        }
        let d = tempfile::tempdir().unwrap();
        assert!(
            check_document_size(d.path()).is_err(),
            "a directory is not a document either"
        );

        // And a real file still opens.
        let f = d.path().join("real.md");
        std::fs::write(&f, "# Real\n").unwrap();
        assert_eq!(read_document(&f).unwrap(), "# Real\n");
    }

    #[test]
    fn a_link_out_of_the_library_asks_before_it_opens() {
        // `Path::join` with an absolute component discards the base, so this
        // needed no `..` at all to read any file on the system.
        let inside = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("secret.md");
        std::fs::write(&secret, "# Secret\n\nnot yours\n").unwrap();
        let neighbour = inside.path().join("neighbour.md");
        std::fs::write(&neighbour, "# Neighbour\n\nfine\n").unwrap();

        let src = format!(
            "# Doc\n\n[out]({})\n\n[in](neighbour.md)\n",
            secret.display()
        );
        let mut a = App::new("doc.md".into(), Document::parse(&src), 60, 20);
        a.file = Some(inside.path().join("doc.md"));
        a.library_root = Some(inside.path().to_path_buf());

        // First Enter on the escaping link: a note, and nothing opened.
        update(&mut a, Action::LinkStep(1));
        update(&mut a, Action::LinkFollow);
        assert!(
            a.note
                .as_deref()
                .unwrap_or_default()
                .contains("outside the library"),
            "expected a confirmation note, got {:?}",
            a.note
        );
        assert!(
            !a.doc.text.contains("not yours"),
            "the document must NOT have opened on the first Enter"
        );

        // Second Enter on the same link opens it: containment is a speed
        // bump for a deliberate reader, not a wall.
        update(&mut a, Action::LinkFollow);
        assert!(
            a.doc.text.contains("not yours"),
            "a confirmed open must go through"
        );

        // A link that stays inside never asks.
        let mut a = App::new("doc.md".into(), Document::parse(&src), 60, 20);
        a.file = Some(inside.path().join("doc.md"));
        a.library_root = Some(inside.path().to_path_buf());
        update(&mut a, Action::LinkStep(1)); // lands on link 0
        update(&mut a, Action::LinkStep(1)); // …then link 1, the in-library one
        update(&mut a, Action::LinkFollow);
        assert!(
            a.doc.text.contains("fine"),
            "an in-library link opens on the first Enter: {:?}",
            a.note
        );
    }

    #[test]
    fn stepping_to_another_link_abandons_a_pending_escape() {
        let inside = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("secret.md");
        std::fs::write(&secret, "# Secret\n").unwrap();
        let src = format!("# D\n\n[a]({})\n\n[b](x.md)\n", secret.display());
        let mut a = App::new("doc.md".into(), Document::parse(&src), 60, 20);
        a.file = Some(inside.path().join("doc.md"));
        a.library_root = Some(inside.path().to_path_buf());

        update(&mut a, Action::LinkStep(1));
        update(&mut a, Action::LinkFollow); // arms the confirmation
        update(&mut a, Action::LinkStep(1)); // …and moves off it
        update(&mut a, Action::LinkStep(-1)); // back again
        update(&mut a, Action::LinkFollow);
        assert!(
            a.note
                .as_deref()
                .unwrap_or_default()
                .contains("outside the library"),
            "the confirmation must not survive a trip through another link"
        );
    }

    #[test]
    fn a_reload_forgets_the_block_cursor_too() {
        let src = "a\n\n```sh\none\n```\n\nb\n\n```sh\ntwo\n```\n";
        let mut a = App::new("t.md".into(), Document::parse(src), 40, 20);
        update(&mut a, Action::CodeStep(1));
        update(&mut a, Action::CodeStep(1));
        assert!(a.code_focus.is_some());

        a.reload_from("# tiny\n");
        assert_eq!(a.code_focus, None);
        update(&mut a, Action::YankBlock); // must not panic
    }

    #[test]
    fn yank_with_no_cursor_takes_the_block_in_front_of_you() {
        let src = "intro\n\n```sh\nthe one\n```\n";
        let mut a = App::new("t.md".into(), Document::parse(src), 40, 20);
        assert!(a.code_focus.is_none());
        update(&mut a, Action::YankBlock);
        assert_eq!(a.clipboard.take().as_deref(), Some("the one\n"));
    }

    #[test]
    fn a_document_with_no_code_says_so_instead_of_copying_prose() {
        let mut a = App::new("t.md".into(), Document::parse("just prose\n"), 40, 10);
        update(&mut a, Action::YankBlock);
        assert!(a.clipboard.is_none(), "prose is not a code block");
        assert!(a.note.is_some());
    }

    /// A folded section must not make a code block unreachable — the standing
    /// rule for every byte-targeted jump.
    #[test]
    fn stepping_to_a_code_block_inside_a_fold_unfolds_it() {
        let src = "# Head\n\n```sh\nhidden cmd\n```\n";
        let mut a = App::new("t.md".into(), Document::parse(src), 40, 20);
        update(&mut a, Action::FoldAll);
        assert!(!a.folded.is_empty());
        update(&mut a, Action::CodeStep(1));
        let b = a.code_focus.expect("found it through the fold");
        assert!(
            a.layout.height(b) > 0,
            "the block is still hidden after stepping to it"
        );
    }

    #[test]
    fn reload_from_keeps_matches_live_across_an_append() {
        let mut a = App::new("(stdin)".into(), Document::parse("needle one\n"), 40, 8);
        update(&mut a, Action::SearchOpen(Direction::Forward));
        for c in "needle".chars() {
            update(&mut a, Action::SearchKey(SearchKey::Char(c)));
        }
        update(&mut a, Action::SearchKey(SearchKey::Accept));
        assert_eq!(a.matches.as_ref().unwrap().len(), 1);
        a.reload_from("needle one\n\nand a second needle\n");
        assert_eq!(
            a.matches.as_ref().unwrap().len(),
            2,
            "the appended hit is found"
        );
    }

    #[test]
    fn a_pathless_document_resolves_links_from_the_working_directory() {
        let mut a = App::new("(stdin)".into(), Document::parse("x"), 20, 6);
        assert_eq!(a.doc_dir(), Path::new("."), "no file means the cwd");
        a.file = Some(PathBuf::from("/tmp/notes/n.md"));
        assert_eq!(a.doc_dir(), Path::new("/tmp/notes"));
    }

    #[test]
    fn following_a_link_out_of_a_piped_document_and_back_again() {
        let d = tempfile::tempdir().unwrap();
        let t = d.path().join("t.md");
        std::fs::write(&t, "# Target doc\n").unwrap();
        let src = format!("intro\n\n[go]({})\n", t.display());
        let mut a = App::new("(stdin)".into(), Document::parse(&src), 40, 8);
        a.piped = Some(src.clone());

        update(&mut a, Action::LinkStep(1)); // select the only link
        assert!(a.selected_link.is_some());
        update(&mut a, Action::LinkFollow);
        assert!(a.doc.text.contains("Target doc"), "the link opened");
        assert_eq!(a.file.as_deref(), Some(t.as_path()));

        update(&mut a, Action::Back);
        assert!(a.doc.text.contains("intro"), "the piped text came back");
        assert_eq!(a.file, None, "and it is pathless again");
        assert_eq!(a.path, "(stdin)");
    }

    #[test]
    fn reload_after_a_truncation_clamps_instead_of_panicking() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("live.md");
        std::fs::write(&f, SRC).unwrap();
        let mut a = App::new("live.md".into(), Document::parse(SRC), 20, 7);
        a.file = Some(f.clone());
        update(&mut a, Action::GoToEnd);

        std::fs::write(&f, "tiny\n").unwrap();
        a.reload().unwrap();
        assert!((a.view.anchor as usize) < a.doc.text.len().max(1));
        assert_eq!(a.view.scroll_row, 0, "a tiny doc has one screenful");
    }

    #[test]
    fn reload_reruns_the_search_and_clamps_the_current_index() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("live.md");
        std::fs::write(&f, "needle one\n\nneedle two\n").unwrap();
        let mut a = App::new(
            "live.md".into(),
            Document::parse("needle one\n\nneedle two\n"),
            40,
            8,
        );
        a.file = Some(f.clone());
        update(&mut a, Action::SearchOpen(Direction::Forward));
        for c in "needle".chars() {
            update(&mut a, Action::SearchKey(SearchKey::Char(c)));
        }
        update(&mut a, Action::SearchKey(SearchKey::Accept));
        update(&mut a, Action::MatchStep(1)); // current = 1 of 2
        assert_eq!(a.matches.as_ref().unwrap().current, Some(1));

        std::fs::write(&f, "needle alone\n").unwrap();
        a.reload().unwrap();
        let m = a.matches.as_ref().unwrap();
        assert_eq!(m.len(), 1, "the count follows the new text");
        assert_eq!(m.current, Some(0), "current clamped into range");
    }

    #[test]
    fn reload_clears_the_selection_and_a_vanished_file_keeps_the_doc() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("live.md");
        std::fs::write(&f, SRC).unwrap();
        let mut a = App::new("live.md".into(), Document::parse(SRC), 20, 6);
        a.file = Some(f.clone());
        update(&mut a, Action::SelectAnchor((0, 1)));
        update(&mut a, Action::SelectDrag((4, 5)));
        assert!(a.selection.is_some());

        std::fs::write(&f, format!("{SRC}\nmore\n")).unwrap();
        a.reload().unwrap();
        assert_eq!(a.selection, None, "old bytes, old meaning");

        std::fs::remove_file(&f).unwrap();
        assert!(a.reload().is_err(), "vanished file reports the error");
        assert!(!a.doc.text.is_empty(), "the last good copy stays");
    }

    #[test]
    fn slash_enters_content_search_and_esc_backs_out_in_two_stages() {
        let d = tempfile::tempdir().unwrap();
        let mut a = App::new_home(d.path().into(), vec![], 40, 10);
        update(&mut a, Action::HomeSearchMode);
        assert_eq!(a.home().unwrap().mode, HomeMode::Search);
        update(&mut a, Action::HomeKey(SearchKey::Char('x')));
        assert_eq!(a.home().unwrap().query, "x");
        update(&mut a, Action::HomeKey(SearchKey::Cancel));
        let h = a.home().unwrap();
        assert!(h.query.is_empty(), "first Esc clears the query");
        assert_eq!(h.mode, HomeMode::Search);
        update(&mut a, Action::HomeKey(SearchKey::Cancel));
        assert_eq!(a.home().unwrap().mode, HomeMode::Normal, "second leaves");
    }

    #[test]
    fn opening_a_search_hit_lands_on_the_first_match_with_n_ready() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("doc.md");
        std::fs::write(
            &f,
            "# T\n\nfiller\n\nthe needle sits here\n\nneedle again\n",
        )
        .unwrap();
        let mut a = App::new_home(d.path().into(), vec![], 40, 8);
        update(&mut a, Action::HomeSearchMode);
        for c in "needle".chars() {
            update(&mut a, Action::HomeKey(SearchKey::Char(c)));
        }
        // The loop normally streams these in; inject directly.
        if let Some(h) = a.home_mut() {
            h.hits.push(crate::grep::Hit {
                path: f.clone(),
                count: 2,
                first_line: "the needle sits here".into(),
            });
        }
        update(&mut a, Action::HomeOpen);
        assert!(!a.is_home(), "opened the hit");
        let m = a.matches.as_ref().expect("search is live");
        assert_eq!(m.len(), 2);
        assert_eq!(m.current, Some(0), "landed on the first match");
    }

    #[test]
    fn search_hits_selection_moves_and_clamps() {
        let d = tempfile::tempdir().unwrap();
        let mut a = App::new_home(d.path().into(), vec![], 40, 10);
        update(&mut a, Action::HomeSearchMode);
        if let Some(h) = a.home_mut() {
            for i in 0..3 {
                h.hits.push(crate::grep::Hit {
                    path: PathBuf::from(format!("/x/{i}.md")),
                    count: 1,
                    first_line: String::new(),
                });
            }
        }
        update(&mut a, Action::HomeMove(10));
        assert_eq!(a.home().unwrap().hit_selected, 2, "clamped to last hit");
        update(&mut a, Action::HomeMove(-10));
        assert_eq!(a.home().unwrap().hit_selected, 0);
    }

    #[test]
    fn editing_the_query_resets_the_hits() {
        let d = tempfile::tempdir().unwrap();
        let mut a = App::new_home(d.path().into(), vec![], 40, 10);
        update(&mut a, Action::HomeSearchMode);
        if let Some(h) = a.home_mut() {
            h.hits.push(crate::grep::Hit {
                path: PathBuf::from("/x/stale.md"),
                count: 1,
                first_line: String::new(),
            });
        }
        update(&mut a, Action::HomeKey(SearchKey::Char('q')));
        assert!(
            a.home().unwrap().hits.is_empty(),
            "stale hits cleared on edit"
        );
    }

    const MERMAID: &str = "intro paragraph\n\n```mermaid\ngraph TD\n a-->b\n```\n\ntail\n";

    #[test]
    fn diagram_art_arrival_is_just_another_reflow() {
        let mut a = App::new("t.md".into(), Document::parse(MERMAID), 40, 8);
        update(&mut a, Action::Scroll(Span::Line, 1));
        let anchor = a.view.anchor;
        let before = a.layout.total_rows();

        a.diagram_art.insert(BlockIdx(1), vec!["┌─┐".into(); 12]);
        a.relayout();

        assert_eq!(a.view.anchor, anchor, "the anchor is the authority");
        assert!(
            a.layout.total_rows() > before,
            "12 art rows grew a 2-line code block"
        );
    }

    #[test]
    fn m_toggles_rendered_blocks_between_art_and_source() {
        let mut a = App::new("t.md".into(), Document::parse(MERMAID), 40, 8);
        a.diagram_art.insert(BlockIdx(1), vec!["┌─┐".into(); 12]);
        a.relayout();
        let with_art = a.layout.total_rows();

        update(&mut a, Action::RenderedToggle);
        assert!(!a.show_rendered);
        assert_eq!(a.note.as_deref(), Some("rendered: source"));
        assert!(a.layout.total_rows() < with_art, "source rows are shorter");

        update(&mut a, Action::RenderedToggle);
        assert!(a.show_rendered);
        assert_eq!(a.note.as_deref(), Some("rendered: art"));
        assert_eq!(a.layout.total_rows(), with_art);
    }

    /// `m` widened to cover math, so it must actually move math too -- the
    /// mermaid half passing is not evidence for the math half.
    #[test]
    fn m_also_flips_math_between_art_and_source() {
        let mut a = App::new(
            "t.md".into(),
            Document::parse("$$\\frac{a+b}{c}$$\n"),
            40,
            12,
        );
        let b = *a.math_art.keys().next().expect("math art");
        assert_eq!(a.math_form(b, 40), MathForm::Display);
        let with_art = a.layout.height(b);

        update(&mut a, Action::RenderedToggle);
        assert_eq!(a.math_form(b, 40), MathForm::Source, "m reaches math");
        assert!(
            a.layout.height(b) < with_art,
            "the source form is one wrapped line, shorter than three art rows"
        );

        update(&mut a, Action::RenderedToggle);
        assert_eq!(a.math_form(b, 40), MathForm::Display);
        assert_eq!(a.layout.height(b), with_art);
    }

    #[test]
    fn opening_another_file_clears_the_art() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("other.md");
        std::fs::write(&f, "plain\n").unwrap();
        let mut a = App::new("t.md".into(), Document::parse(MERMAID), 40, 8);
        a.diagram_art.insert(BlockIdx(1), vec!["┌─┐".into(); 3]);
        a.open_path(&f).unwrap();
        assert!(a.diagram_art.is_empty(), "old blocks, old art");
    }

    #[test]
    fn a_resize_leaves_the_match_set_and_current_index_untouched() {
        let mut a = app();
        update(&mut a, Action::SearchOpen(Direction::Forward));
        update(&mut a, Action::SearchKey(SearchKey::Char('a')));
        update(&mut a, Action::SearchKey(SearchKey::Accept));
        let ranges = a.matches.as_ref().unwrap().ranges.clone();
        let current = a.matches.as_ref().unwrap().current;

        a.on_resize(11, 6);

        assert_eq!(a.matches.as_ref().unwrap().ranges, ranges, "mdfried #52");
        assert_eq!(a.matches.as_ref().unwrap().current, current);
    }
}
