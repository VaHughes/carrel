//! Carrel — a quiet place to read your markdown.
//!
//! The binary is deliberately thin: argument handling, terminal lifecycle, and
//! the event loop. Everything else lives in the library so integration tests
//! can drive it. See the TUI design doc (notes repo).

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use std::io::Write as _;

use carrel::app::{App, Outcome, update};
use carrel::images::{self, ImageMsg};
use carrel::keys::Keys;
use carrel::render::OscLink;
use carrel::{config, home, render, scan};
use carrel_core::{BlockIdx, Document, cluster_width, cols_for_doc_range, search, wrap};
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind, MouseButton, MouseEvent,
    MouseEventKind,
};
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;
use std::collections::HashMap;
use std::sync::mpsc::Receiver;

const USAGE: &str = "\
carrel — a quiet place to read your markdown

USAGE:
    carrel                       the home screen: what is around you to read
    carrel <DIR>                 the home screen, rooted at DIR
    carrel <FILE>                read a document
    carrel <FILE> <PATTERN>      print search results and exit
    carrel --plain <FILE> [W]    the document as plain text (screen readers,
                                 pipes; a bare pipe does this by default)
    carrel --help
    carrel --version

    NO_COLOR is honoured: colours off, weight and emphasis kept.

KEYS (home screen):
    j k ↓ ↑                      move           enter open
    i                            filter names: type to narrow, esc to leave
    /                            search inside files, enter opens at a match
    gg G                         ends           d     choose a directory
    T                            cycle themes   q     quit
    h F1                         help

KEYS (while reading):
    j k ↓ ↑ Ctrl-E Ctrl-Y        line            gg G Home End   start / end
    Ctrl-D Ctrl-U                half page       { }             block
    Space b Ctrl-F Ctrl-B        page            42G             go to row 42
    / ?                          search          n N             next / previous
    zz zt zb                     put the current match middle / top / bottom
    o                            outline: jump to a section
    t                            tables: cards ↔ wrapped
    m                            mermaid diagrams: rendered ↔ source
    T                            cycle themes    q Ctrl-C        quit
    h F1                         help            Ctrl-O          back
    mouse: drag selects and copies; double-click a word, triple-click a block
";

fn main() -> ExitCode {
    // no-color.org: any non-empty value means monochrome. Once, at startup —
    // the flag is process-global presentation state, like the theme.
    if std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()) {
        carrel::theme::set_mono(true);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [] => open_home(None),
        [a] if a == "-h" || a == "--help" => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        [a] if a == "-V" || a == "--version" => {
            println!("carrel {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        [p] if Path::new(p).is_dir() => open_home(Some(Path::new(p))),
        [flag, file] if flag == "--plain" => print_plain(Path::new(file), 80),
        [flag, file, w] if flag == "--plain" => {
            let width = w.parse().unwrap_or(80);
            print_plain(Path::new(file), width)
        }
        [file] => open(Path::new(file), None),
        [file, pattern] => open(Path::new(file), Some(pattern)),
        _ => {
            eprint!("{USAGE}");
            ExitCode::FAILURE
        }
    }
}

/// Resolve the root and open the home screen.
///
/// **Precedence: explicit argument > saved root > current directory.** A saved
/// root must beat the working directory, or choosing `~/Documents` would
/// silently stop applying the moment you `cd`. `carrel .` is the escape hatch,
/// and the active root is always on screen.
fn open_home(explicit: Option<&Path>) -> ExitCode {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let (root, note) = match explicit {
        Some(p) => (p.to_path_buf(), None),
        None => match config::load_root() {
            Some(saved) if saved.is_dir() => (saved, None),
            Some(saved) => (
                cwd.clone(),
                Some(format!("saved directory is gone: {}", saved.display())),
            ),
            None => (cwd.clone(), None),
        },
    };

    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        let (entries, _) = scan::walk_blocking(&root);
        println!(
            "{} markdown file(s) under {}",
            entries.len(),
            root.display()
        );
        for e in entries.iter().take(50) {
            println!("  {}", e.path.display());
        }
        return ExitCode::SUCCESS;
    }

    match run_home(root, note) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("carrel: {e}");
            ExitCode::FAILURE
        }
    }
}

fn open(path: &Path, pattern: Option<&str>) -> ExitCode {
    if let Err(e) = carrel::app::check_document_size(path) {
        eprintln!("carrel: {}: {e}", path.display());
        return ExitCode::FAILURE;
    }
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("carrel: {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
    };

    // With a pattern, stay non-interactive: useful for scripting and for
    // checking the core without a terminal.
    if let Some(p) = pattern {
        report(path, &src, p);
        return ExitCode::SUCCESS;
    }

    // No terminal, no TUI. `less` behaves the same way, and the alternative is
    // ugly: with stdin at EOF, `event::poll` reports ready forever and the
    // reader spins at 100% CPU redrawing frames nobody can see. Found by
    // running the binary under a pty with piped input — no unit test reaches
    // this, because `TestBackend` never touches a real stdin.
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        // Piping implies plain (Q17): a pipe wants the document, not a
        // summary — and linear text is also what a screen reader can use.
        print!("{}", carrel::plain::render(&Document::parse(&src), 80));
        return ExitCode::SUCCESS;
    }

    match run(path, &src) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("carrel: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Restores the terminal on every ordinary exit path, including `?`.
///
/// `ratatui::init()` installs the panic hook for raw mode and the alternate
/// screen, but **mouse capture is ours**: a terminal left in mouse mode
/// breaks the user's scrollback and text selection, which is exactly the
/// kind of exit damage this project refuses to inflict. Disable it in both
/// the guard and a wrapped panic hook.
struct TerminalGuard;

impl TerminalGuard {
    fn engage_mouse() -> Self {
        let _ = ratatui::crossterm::execute!(std::io::stdout(), EnableMouseCapture);
        // Chain a mouse-disable in FRONT of ratatui's restoring panic hook.
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = ratatui::crossterm::execute!(std::io::stdout(), DisableMouseCapture);
            prev(info);
        }));
        Self
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = ratatui::crossterm::execute!(std::io::stdout(), DisableMouseCapture);
        ratatui::restore();
    }
}

/// Everything the image pipeline owns on the frontend side: the protocol
/// picker (kitty where advertised, half-blocks otherwise), the decode
/// receiver, and per-block protocol state for the painter.
struct Images {
    picker: Picker,
    rx: Option<Receiver<ImageMsg>>,
    protocols: HashMap<BlockIdx, StatefulProtocol>,
    /// The file the current pipeline belongs to; a document change restarts it.
    for_file: Option<std::path::PathBuf>,
}

impl Images {
    /// Detect the protocol and font size **without stdio queries**.
    ///
    /// `Picker::from_query_stdio` spawns a thread that blocks reading stdin;
    /// when the terminal never answers (a `script` pty, CI, some multiplexer
    /// paths) the call times out and falls back — but the orphaned thread
    /// keeps reading and STEALS every subsequent keystroke, hanging the app.
    /// Found by the pty smoke test, which is exactly why it exists.
    ///
    /// Instead: font size from the `TIOCGWINSZ` ioctl (synchronous, no reply
    /// needed), protocol from the environment via the deprecated-but-safe
    /// `from_fontsize`, whose deprecation points at the query path we are
    /// deliberately avoiding.
    fn detect() -> Self {
        let font = match ratatui::crossterm::terminal::window_size() {
            Ok(ws) if ws.width > 0 && ws.height > 0 && ws.columns > 0 && ws.rows > 0 => {
                ratatui_image::FontSize::new(ws.width / ws.columns, ws.height / ws.rows)
            }
            _ => ratatui_image::FontSize::new(8, 16),
        };
        #[allow(deprecated)]
        let picker = Picker::from_fontsize(font);
        Self {
            picker,
            rx: None,
            protocols: HashMap::new(),
            for_file: None,
        }
    }

    /// (Re)start decoding when the open document changed.
    fn sync(&mut self, app: &mut App) {
        if app.is_home() || app.file == self.for_file {
            return;
        }
        self.for_file.clone_from(&app.file);
        self.protocols.clear();
        let fs = self.picker.font_size();
        app.font_px = (fs.width, fs.height);
        let base = app.file.as_deref().and_then(std::path::Path::parent);
        let reqs = images::local_image_requests(&app.doc, base);
        self.rx = if reqs.is_empty() {
            None
        } else {
            Some(images::spawn_decoder(reqs))
        };
    }

    /// Drain decode results. Dimension arrival is just another reflow.
    fn drain(&mut self, app: &mut App) {
        let Some(rx) = self.rx.as_ref() else { return };
        let mut changed = false;
        loop {
            match rx.try_recv() {
                Ok(ImageMsg::Decoded(block, img)) => {
                    app.image_dims.insert(block, (img.width(), img.height()));
                    self.protocols
                        .insert(block, self.picker.new_resize_protocol(img));
                    changed = true;
                }
                Ok(ImageMsg::Failed(_, e)) => {
                    app.note = Some(e);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.rx = None;
                    break;
                }
            }
        }
        if changed {
            app.relayout();
        }
    }
}

/// Mermaid box-art rendering, mirroring [`Images`]: restarted when the open
/// document changes, drained once per frame. Art arrival is just another
/// reflow through the anchor machinery.
struct Diagrams {
    rx: Option<Receiver<(BlockIdx, Vec<String>)>>,
    for_file: Option<PathBuf>,
}

impl Diagrams {
    fn new() -> Self {
        Self {
            rx: None,
            for_file: None,
        }
    }

    fn sync(&mut self, app: &App) {
        if app.is_home() || app.file == self.for_file {
            return;
        }
        self.for_file.clone_from(&app.file);
        let reqs = carrel::diagrams::requests(&app.doc);
        self.rx = (!reqs.is_empty()).then(|| carrel::diagrams::spawn(reqs));
    }

    fn drain(&mut self, app: &mut App) {
        let Some(rx) = self.rx.as_ref() else { return };
        let mut changed = false;
        loop {
            match rx.try_recv() {
                Ok((block, art)) => {
                    app.diagram_art.insert(block, art);
                    changed = true;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.rx = None;
                    break;
                }
            }
        }
        if changed {
            app.relayout();
        }
    }
}

/// Coalesces the SIGWINCH storm: a drag emits one event per frame, and each
/// one would otherwise trigger a full O(N) height pass. `architecture.md` §3.5.
const DEBOUNCE: Duration = Duration::from_millis(40);

/// Double/triple-click detection: same cell within this window.
const MULTI_CLICK: Duration = Duration::from_millis(400);

/// How often the open file is stat'ed for live reload.
const RELOAD_POLL: Duration = Duration::from_secs(1);

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Reload {
    /// Content changed (or a vanished file came back): re-parse.
    Changed,
    /// The file disappeared. Reported ONCE; the last good parse stays up.
    Vanished,
}

/// Live reload by polling mtime+len once a second — chosen over inotify
/// deliberately: one syscall a second costs nothing, needs no platform
/// backend dependency, and works on network mounts where inotify is
/// silently unreliable. See the wave-D spec.
struct Reloader {
    last_check: Instant,
    /// The last snapshot: which file, its mtime, its length.
    seen: Option<(PathBuf, std::time::SystemTime, u64)>,
    missing: bool,
}

impl Reloader {
    fn new() -> Self {
        Self {
            last_check: Instant::now(),
            seen: None,
            missing: false,
        }
    }

    fn poll(&mut self, file: Option<&Path>) -> Option<Reload> {
        if self.last_check.elapsed() < RELOAD_POLL {
            return None;
        }
        self.last_check = Instant::now();
        let path = file?;
        let snap = std::fs::metadata(path).ok().map(|m| {
            (
                path.to_path_buf(),
                m.modified().unwrap_or(std::time::UNIX_EPOCH),
                m.len(),
            )
        });
        let Some(snap) = snap else {
            // Gone. Say so once; keep watching for it to come back.
            let first = !self.missing && self.seen.is_some();
            self.missing = true;
            return first.then_some(Reload::Vanished);
        };
        let came_back = std::mem::take(&mut self.missing);
        let changed = match &self.seen {
            // A different PATH is a navigation, not an edit: rebaseline.
            Some((p, ..)) if p != &snap.0 => false,
            Some(prev) => *prev != snap || came_back,
            None => false,
        };
        self.seen = Some(snap);
        changed.then_some(Reload::Changed)
    }
}

/// Press streak tracking for double/triple click. Presentation state, so it
/// lives in the event loop like the theme — never in `App`.
#[derive(Default)]
struct Clicks {
    last: Option<(Instant, u16, u16)>,
    streak: u8,
}

impl Clicks {
    /// Record a press; the returned streak is 1, 2, or 3 (then wraps).
    fn press(&mut self, col: u16, row: u16) -> u8 {
        let now = Instant::now();
        let same = self
            .last
            .is_some_and(|(t, c, r)| now.duration_since(t) < MULTI_CLICK && c == col && r == row);
        self.streak = if same { (self.streak % 3) + 1 } else { 1 };
        self.last = Some((now, col, row));
        self.streak
    }
}

/// The `(start, end)` doc bytes of the grapheme cluster under a pointer cell,
/// or `None` outside the text area / off the end of the content. The one
/// place pixels become bytes — everything downstream is doc space.
fn doc_span_at(app: &App, col: u16, row: u16) -> Option<(u32, u32)> {
    use carrel::app::{PAD_LEFT, PAD_TOP};
    if row < PAD_TOP || row >= PAD_TOP + app.text_h() {
        return None;
    }
    if col < PAD_LEFT || col >= PAD_LEFT + app.text_w() {
        return None;
    }
    let vrow = app.view.scroll_row + u32::from(row - PAD_TOP);
    let block = app.layout.block_at_row(vrow);
    if block.get() >= app.doc.block_count() {
        return None;
    }
    let mut rows = Vec::new();
    app.layout.rows_for(&app.doc, block, &mut rows);
    let sub = usize::try_from(vrow.saturating_sub(app.layout.row_start(block))).ok()?;
    let r = rows.get(sub)?;
    if r.doc.is_empty() {
        return None;
    }
    let text = app.doc.text.get(r.doc.start as usize..r.doc.end as usize)?;
    Some(carrel_core::cluster_at_col(
        text,
        r.doc.start,
        r.indent,
        col - PAD_LEFT,
    ))
}

/// Copy to the system clipboard via OSC 52. Terminals without it ignore the
/// sequence — the same graceful degradation as OSC 8.
fn osc52(text: &str) -> std::io::Result<()> {
    let mut out = std::io::stdout();
    write!(out, "\x1b]52;c;{}\x07", b64(text.as_bytes()))?;
    out.flush()
}

/// RFC 4648 base64. Hand-rolled: ~20 lines beats a dependency in a project
/// that tracks `regex`'s 1.59 MiB as its largest binary cost.
fn b64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(char::from(T[(n >> 18) as usize & 63]));
        out.push(char::from(T[(n >> 12) as usize & 63]));
        out.push(if chunk.len() > 1 {
            char::from(T[(n >> 6) as usize & 63])
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            char::from(T[n as usize & 63])
        } else {
            '='
        });
    }
    out
}

fn run(path: &Path, src: &str) -> std::io::Result<()> {
    let theme_note = startup_theme();
    let doc = Document::parse(src);

    // Protocol + font detection queries the terminal, so it must happen
    // before the alternate screen swallows the replies.
    let mut images = Images::detect();

    let mut terminal = ratatui::init();
    let _guard = TerminalGuard::engage_mouse();

    let size = terminal.size()?;
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let mut app = App::new(name, doc, size.width, size.height);
    app.file = Some(path.to_path_buf());
    app.note = theme_note;
    app.config_dir = config::config_dir();
    app.state_dir = carrel::state::state_dir();
    app.hints = config::load_hints().unwrap_or(true);
    // A direct open builds the App by hand rather than via open_path, so the
    // saved reading position needs restoring explicitly. Its note outranks
    // the theme note — the theme is only news when it failed to load.
    app.restore_position();
    let mut keys = Keys::new();

    let mut pending: Option<(u16, u16)> = None;
    let mut deadline: Option<Instant> = None;
    let mut links: Vec<OscLink> = Vec::new();
    let mut dragging: Option<u16> = None;
    let mut clicks = Clicks::default();
    let mut reloader = Reloader::new();
    let mut diagrams = Diagrams::new();

    loop {
        images.sync(&mut app);
        images.drain(&mut app);
        diagrams.sync(&app);
        diagrams.drain(&mut app);
        terminal.draw(|f| render::draw_full(f, &app, &mut links, &mut images.protocols))?;
        emit_osc8(&links)?;

        // Wake in time to apply a debounced resize even with no input arriving.
        let timeout = deadline.map_or(Duration::from_millis(100), |d| {
            d.saturating_duration_since(Instant::now())
        });

        if event::poll(timeout)? {
            // A read error means the input stream is gone. Exit rather than
            // spin: `poll` keeps reporting ready once stdin is at EOF.
            let Ok(ev) = event::read() else { break };
            match ev {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    let action = if app.outline.is_some() {
                        Keys::map_outline(k)
                    } else {
                        keys.map(k, app.searching())
                    };
                    if let Some(action) = action {
                        if action == carrel::action::Action::ThemeCycle {
                            cycle_theme_now(&mut app);
                        } else if update(&mut app, action) == Outcome::Quit {
                            break;
                        }
                    }
                }
                Event::Mouse(m) => {
                    if let Some(action) = mouse_action(m, &app, &mut dragging, &mut clicks)
                        && update(&mut app, action) == Outcome::Quit
                    {
                        break;
                    }
                }
                Event::Resize(w, h) => {
                    pending = Some((w, h));
                    deadline = Some(Instant::now() + DEBOUNCE);
                }
                _ => {}
            }
        }

        // A copy requested by the state machine leaves through OSC 52 here —
        // the outbox keeps I/O out of `update`.
        if let Some(text) = app.clipboard.take() {
            osc52(&text)?;
        }

        poll_reload(&mut reloader, &mut app, &mut images, &mut diagrams);

        if deadline.is_some_and(|d| Instant::now() >= d)
            && let Some((w, h)) = pending.take()
        {
            deadline = None;
            app.on_resize(w, h);
        }
    }
    Ok(())
}

/// One reload check per wake: on change, re-parse in place and restart the
/// image pipeline (same file, new content — `Images::sync` keys on the path,
/// so it must be told to look again).
fn poll_reload(
    reloader: &mut Reloader,
    app: &mut App,
    images: &mut Images,
    diagrams: &mut Diagrams,
) {
    match reloader.poll(app.file.as_deref()) {
        Some(Reload::Changed) => match app.reload() {
            Ok(()) => {
                images.for_file = None;
                diagrams.for_file = None;
            }
            Err(e) => app.note = Some(format!("reload failed: {e}")),
        },
        Some(Reload::Vanished) => {
            app.note = Some("file removed — showing the last good copy".into());
        }
        None => {}
    }
}

/// The home content-search lifecycle: debounce keystrokes, one background
/// thread per settled query, only current-generation results applied. A
/// stale thread's receiver is dropped; it stops on its next send.
struct Grep {
    rx: Option<Receiver<carrel::grep::Msg>>,
    generation: u64,
    spawned: Option<String>,
    pending: Option<(String, Instant)>,
}

const GREP_DEBOUNCE: Duration = Duration::from_millis(150);

impl Grep {
    fn new() -> Self {
        Self {
            rx: None,
            generation: 0,
            spawned: None,
            pending: None,
        }
    }

    /// Streaming in progress (or imminent): the loop should wake fast.
    fn busy(&self) -> bool {
        self.rx.is_some() || self.pending.is_some()
    }

    fn tick(&mut self, app: &mut App) {
        match app.home() {
            Some(h) if h.mode == home::HomeMode::Search && !h.query.trim().is_empty() => {
                let q = h.query.clone();
                if self.spawned.as_deref() != Some(q.as_str()) {
                    match &self.pending {
                        Some((pq, since)) if *pq == q => {
                            if since.elapsed() >= GREP_DEBOUNCE {
                                self.generation += 1;
                                let entries = h.entries.clone();
                                self.rx =
                                    Some(carrel::grep::spawn(entries, q.clone(), self.generation));
                                self.spawned = Some(q);
                                self.pending = None;
                            }
                        }
                        _ => self.pending = Some((q, Instant::now())),
                    }
                }
            }
            _ => {
                self.rx = None;
                self.spawned = None;
                self.pending = None;
            }
        }
        let Some(rx) = self.rx.as_ref() else { return };
        let mut disconnect = false;
        loop {
            match rx.try_recv() {
                Ok(carrel::grep::Msg::Hit(hit, generation)) if generation == self.generation => {
                    if let Some(h) = app.home_mut() {
                        h.hits.push(hit);
                    }
                }
                Ok(carrel::grep::Msg::Done(generation)) => {
                    if generation == self.generation
                        && let Some(h) = app.home_mut()
                    {
                        h.grep_done = true;
                    }
                    disconnect = true;
                    break;
                }
                Ok(carrel::grep::Msg::Hit(..)) => {} // stale generation
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    disconnect = true;
                    break;
                }
            }
        }
        if disconnect {
            self.rx = None;
        }
    }
}

fn run_home(root: PathBuf, note: Option<String>) -> std::io::Result<()> {
    let theme_note = startup_theme();
    let note = note.or(theme_note);
    let mut images = Images::detect();

    let mut terminal = ratatui::init();
    let _guard = TerminalGuard::engage_mouse();

    let size = terminal.size()?;
    // The cache paints before any syscall; the walk refines it.
    let cached = scan::load_cache(&root);
    let mut app = App::new_home(root.clone(), cached, size.width, size.height);
    app.config_dir = config::config_dir();
    app.state_dir = carrel::state::state_dir();
    app.hints = config::load_hints().unwrap_or(true);
    if let Some(n) = note
        && let Some(h) = app.home_mut()
    {
        h.note = Some(n);
    }
    let mut keys = Keys::new();
    let mut dragging: Option<u16> = None;
    let mut clicks = Clicks::default();
    let mut reloader = Reloader::new();
    let mut diagrams = Diagrams::new();
    let mut scan_rx = Some(scan::spawn(&root));
    let mut scan_root = root;

    // Content search (wave E): see `Grep`.
    let mut grep = Grep::new();

    let mut pending: Option<(u16, u16)> = None;
    let mut deadline: Option<Instant> = None;
    let mut links: Vec<OscLink> = Vec::new();

    loop {
        // Same draw path as run(): documents opened FROM the home screen must
        // get the same OSC 8 hyperlinks — and the same images — as documents
        // opened directly. On the home screen both are no-ops.
        images.sync(&mut app);
        images.drain(&mut app);
        diagrams.sync(&app);
        diagrams.drain(&mut app);
        terminal.draw(|f| render::draw_full(f, &app, &mut links, &mut images.protocols))?;
        emit_osc8(&links)?;

        drain_scan(&mut app, &mut scan_rx);

        grep.tick(&mut app);

        // While a scan or search is live, wake often enough to stream.
        let busy = scan_rx.is_some() || grep.busy();
        let idle = if busy { 16 } else { 100 };
        let timeout = deadline.map_or(Duration::from_millis(idle), |d| {
            d.saturating_duration_since(Instant::now())
        });

        if event::poll(timeout)? {
            let Ok(ev) = event::read() else { break };
            match ev {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    let action = if app.outline.is_some() {
                        Keys::map_outline(k)
                    } else if app.is_home() {
                        let mode = app.home().map_or(home::HomeMode::Filter, |h| h.mode);
                        // `Other…` sits one past the listed roots; while it is
                        // highlighted, printable keys edit the path.
                        let typing_path = app.home().is_some_and(|h| {
                            h.mode == home::HomeMode::Picker
                                && h.picker.selected == h.picker.roots.len()
                        });
                        keys.map_home(k, mode, typing_path)
                    } else {
                        keys.map(k, app.searching())
                    };
                    if let Some(a) = action {
                        if a == carrel::action::Action::ThemeCycle {
                            cycle_theme_now(&mut app);
                        } else if update(&mut app, a) == Outcome::Quit {
                            break;
                        }
                    }
                }
                Event::Mouse(m) => {
                    if let Some(a) = home_mouse_action(m, &app, &mut dragging, &mut clicks)
                        && update(&mut app, a) == Outcome::Quit
                    {
                        break;
                    }
                }
                Event::Resize(w, h) => {
                    pending = Some((w, h));
                    deadline = Some(Instant::now() + DEBOUNCE);
                }
                _ => {}
            }
        }

        // Same clipboard outbox as run(): documents opened FROM the home
        // screen must copy exactly like documents opened directly.
        if let Some(text) = app.clipboard.take() {
            osc52(&text)?;
        }

        // And the same live reload (a no-op while the home screen is up —
        // there is no open file to watch).
        poll_reload(&mut reloader, &mut app, &mut images, &mut diagrams);

        if deadline.is_some_and(|d| Instant::now() >= d)
            && let Some((w, h)) = pending.take()
        {
            deadline = None;
            app.on_resize(w, h);
        }

        // The picker changed root: restart the walk against the new one.
        if let Some(h) = app.home()
            && h.root != scan_root
        {
            scan_root.clone_from(&h.root);
            scan_rx = Some(scan::spawn(&scan_root));
        }
    }
    Ok(())
}

/// Pointer events for the reader.
///
/// Desktop scrollbar semantics, because anything else feels jarring:
/// clicking the **thumb grabs it and moves nothing** — the drag then tracks
/// the pointer 1:1, preserving where on the thumb the hand took hold;
/// clicking the empty **track pages** toward the click rather than
/// teleporting the document.
fn mouse_action(
    m: MouseEvent,
    app: &App,
    dragging: &mut Option<u16>,
    clicks: &mut Clicks,
) -> Option<carrel::action::Action> {
    use carrel::action::{Action, Span};
    use carrel::keys::{drag_target, thumb_geometry};

    let bar_x = app.cols.saturating_sub(1); // the scrollbar column
    let text_h = app.text_h();
    let total = app.layout.total_rows();
    let max_scroll = app.layout.max_scroll(text_h);
    // The bar's track starts PAD_TOP rows down; pointer rows map into it.
    let bar_y = |row: u16| row.saturating_sub(carrel::app::PAD_TOP);

    match m.kind {
        MouseEventKind::ScrollDown => Some(Action::Scroll(Span::Line, 3)),
        MouseEventKind::ScrollUp => Some(Action::Scroll(Span::Line, -3)),
        // The lamp — lit on the footer, folded on the status row — is the
        // switch. Both live on the bottom row's first cells.
        MouseEventKind::Down(MouseButton::Left) if m.row + 1 == app.rows && m.column < 3 => {
            Some(Action::HintsToggle)
        }
        MouseEventKind::Down(MouseButton::Left) if m.column >= bar_x && max_scroll > 0 => {
            let (top, len) = thumb_geometry(text_h, total, app.view.scroll_row);
            let row = bar_y(m.row);
            if row >= top && row < top + len {
                // Grab. The view does not move until the hand does.
                *dragging = Some(row - top);
                None
            } else {
                // Track click: one gentle page toward the pointer.
                Some(Action::Scroll(Span::Page, if row < top { -1 } else { 1 }))
            }
        }
        // A press in the text proper starts (or extends, on double/triple) a
        // selection — the reason mouse capture disabled native selection is
        // that this one copies CLEAN text: no bars, no markers, no gutter.
        MouseEventKind::Down(MouseButton::Left) => {
            let span = doc_span_at(app, m.column, m.row)?;
            match clicks.press(m.column, m.row) {
                2 => Some(Action::SelectWord(span.0)),
                3 => Some(Action::SelectBlock(span.0)),
                _ => Some(Action::SelectAnchor(span)),
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => match *dragging {
            Some(grab) => Some(Action::ScrollTo(drag_target(
                bar_y(m.row),
                grab,
                text_h,
                total,
                max_scroll,
            ))),
            None => doc_span_at(app, m.column, m.row).map(Action::SelectDrag),
        },
        MouseEventKind::Up(MouseButton::Left) => {
            if dragging.is_some() {
                *dragging = None;
                None
            } else {
                Some(Action::SelectRelease)
            }
        }
        _ => None,
    }
}

/// Apply the saved theme. An unknown name falls back to `terminal` with a
/// status note — a stale config is never an error.
fn startup_theme() -> Option<String> {
    let name = config::load_theme()?;
    if carrel::theme::set_theme(&name) {
        None
    } else {
        Some(format!("unknown theme \"{name}\" — using terminal"))
    }
}

/// Advance the theme, tell the reader which one this is, and persist it.
///
/// Lives in the event loop rather than `update` because the active palette is
/// presentation state — rule 6 keeps colour out of the (ratatui-free) `App`.
fn cycle_theme_now(app: &mut App) {
    let name = carrel::theme::cycle_theme();
    app.set_note(format!("theme: {name}"));
    // The choice applies either way; a failed write just doesn't survive the
    // session, and the note stays honest about the theme itself.
    let _ = config::save_theme(name);
}

/// Drain the background walk into ONE batch per frame. Per-entry
/// reconciliation was O(N²) across a scan and froze the home screen on
/// large trees.
fn drain_scan(app: &mut App, scan_rx: &mut Option<std::sync::mpsc::Receiver<scan::Msg>>) {
    let Some(rx) = scan_rx.as_ref() else { return };
    let mut batch: Vec<scan::Entry> = Vec::new();
    loop {
        match rx.try_recv() {
            Ok(scan::Msg::Found(e)) => batch.push(e),
            Ok(scan::Msg::Done { unreadable }) => {
                if let Some(h) = app.home_mut() {
                    h.push_many(std::mem::take(&mut batch));
                    h.finish_scan(unreadable);
                    scan::save_cache(&h.root, &h.entries);
                }
                *scan_rx = None;
                break;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => break,
            // The walker died mid-scan. Keep everything — the cached entries
            // are still the best available answer — and say so, rather than
            // pruning down to whatever it reported.
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                if let Some(h) = app.home_mut() {
                    h.push_many(std::mem::take(&mut batch));
                    h.abort_scan();
                }
                *scan_rx = None;
                break;
            }
        }
    }
    if let Some(h) = app.home_mut()
        && !batch.is_empty()
    {
        h.push_many(batch);
    }
}

/// Pointer events while the home screen is up: the wheel moves the list.
/// Once a document is open, the reader's full mouse handling applies.
fn home_mouse_action(
    m: MouseEvent,
    app: &App,
    dragging: &mut Option<u16>,
    clicks: &mut Clicks,
) -> Option<carrel::action::Action> {
    use carrel::action::Action;
    if !app.is_home() {
        return mouse_action(m, app, dragging, clicks);
    }
    match m.kind {
        MouseEventKind::ScrollDown => Some(Action::HomeMove(3)),
        MouseEventKind::ScrollUp => Some(Action::HomeMove(-3)),
        // Same lamp, same switch as the reader's bottom row.
        MouseEventKind::Down(MouseButton::Left) if m.row + 1 == app.rows && m.column < 3 => {
            Some(Action::HintsToggle)
        }
        _ => None,
    }
}

/// Re-emit visible link text wrapped in OSC 8 hyperlinks, after the frame.
///
/// ratatui has no cell-level hyperlink support, so this repaints each link's
/// glyphs in place — same text, the link style — with the OSC wrapper around
/// them. Terminals without OSC 8 ignore the sequences entirely, and when the
/// cells later change, ratatui repaints them without the wrapper, which is
/// exactly right. URLs were stripped of control characters at collection.
fn emit_osc8(links: &[OscLink]) -> std::io::Result<()> {
    use ratatui::crossterm::{cursor, queue, style};

    if links.is_empty() {
        return Ok(());
    }
    let mut out = std::io::stdout();
    for l in links {
        queue!(
            out,
            cursor::MoveTo(l.x, l.y),
            style::Print(format!(
                "\x1b]8;;{}\x1b\\\x1b[38;2;224;160;68m\x1b[4m{}\x1b[0m\x1b]8;;\x1b\\",
                l.url, l.text
            )),
        )?;
    }
    out.flush()
}

/// Non-interactive output: a document summary, plus search results when a
/// pattern is given.
/// `--plain`: the document as linear text on stdout — the accessible
/// rendering, and the pipe-friendly one. See `plain.rs` and the Q17 design.
fn print_plain(path: &Path, width: u16) -> ExitCode {
    match std::fs::read_to_string(path) {
        Ok(src) => {
            print!("{}", carrel::plain::render(&Document::parse(&src), width));
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("carrel: {}: {e}", path.display());
            ExitCode::FAILURE
        }
    }
}

fn report(path: &Path, src: &str, pattern: &str) {
    let doc = Document::parse(src);
    let width: u16 = 80;

    let mut rows_total = 0u32;
    for i in 0..doc.block_count() {
        rows_total += wrap(&doc, BlockIdx(i as u32), width, &cluster_width, |_| {});
    }

    println!("{}", path.display());
    println!("  {} bytes source", src.len());
    println!("  {} bytes display text", doc.text.len());
    println!("  {} blocks", doc.block_count());
    println!("  {rows_total} rows at width {width}");

    if pattern.is_empty() {
        return;
    }

    let matches = search(&doc, pattern, true);
    println!("\n  {} match(es) for {pattern:?}", matches.len());

    for (n, r) in matches.ranges.iter().take(10).enumerate() {
        let b = doc.block_at_doc(carrel_core::DocByte(r.start));
        let mut hit = None;
        wrap(&doc, b, width, &cluster_width, |row| {
            if hit.is_none() && r.start < row.doc.end && r.end > row.doc.start {
                let text = &doc.text[row.doc.start as usize..row.doc.end as usize];
                let cols = cols_for_doc_range(text, row.doc.start, row.indent, r);
                hit = Some((text.to_string(), cols));
            }
        });
        if let Some((text, (c0, c1))) = hit {
            println!("    {}. cols {c0}..{c1}  │ {}", n + 1, text.trim());
        }
    }
    if matches.len() > 10 {
        println!("    … and {} more", matches.len() - 10);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_rfc_4648_vectors() {
        assert_eq!(b64(b""), "");
        assert_eq!(b64(b"f"), "Zg==");
        assert_eq!(b64(b"fo"), "Zm8=");
        assert_eq!(b64(b"foo"), "Zm9v");
        assert_eq!(b64(b"foob"), "Zm9vYg==");
        assert_eq!(b64(b"fooba"), "Zm9vYmE=");
        assert_eq!(b64(b"foobar"), "Zm9vYmFy");
        assert_eq!(b64(&[0xFF, 0xEE]), "/+4=");
    }

    #[test]
    fn the_reloader_baselines_then_reports_changes_and_vanishing_once() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("live.md");
        std::fs::write(&f, "one").unwrap();
        let mut r = Reloader::new();
        let poll = |r: &mut Reloader, p: Option<&Path>| {
            r.last_check = Instant::now().checked_sub(Duration::from_secs(2)).unwrap();
            r.poll(p)
        };
        assert_eq!(poll(&mut r, Some(&f)), None, "first sighting is baseline");
        std::fs::write(&f, "one two three").unwrap();
        assert_eq!(poll(&mut r, Some(&f)), Some(Reload::Changed));
        assert_eq!(poll(&mut r, Some(&f)), None, "no change, no event");

        std::fs::remove_file(&f).unwrap();
        assert_eq!(poll(&mut r, Some(&f)), Some(Reload::Vanished));
        assert_eq!(poll(&mut r, Some(&f)), None, "vanished reports ONCE");
        std::fs::write(&f, "back").unwrap();
        assert_eq!(poll(&mut r, Some(&f)), Some(Reload::Changed), "reappeared");
    }

    #[test]
    fn the_reloader_rebaselines_on_a_different_file() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.md");
        let b = d.path().join("b.md");
        std::fs::write(&a, "aaa").unwrap();
        std::fs::write(&b, "bbbbbb").unwrap();
        let mut r = Reloader::new();
        r.last_check = Instant::now().checked_sub(Duration::from_secs(2)).unwrap();
        assert_eq!(r.poll(Some(&a)), None);
        // Following a link to another file must NOT read as "changed".
        r.last_check = Instant::now().checked_sub(Duration::from_secs(2)).unwrap();
        assert_eq!(r.poll(Some(&b)), None, "new file, new baseline");
    }

    #[test]
    fn clicks_count_single_double_triple_and_reset() {
        let mut c = Clicks::default();
        assert_eq!(c.press(5, 5), 1);
        assert_eq!(c.press(5, 5), 2);
        assert_eq!(c.press(5, 5), 3);
        assert_eq!(c.press(5, 5), 1, "a fourth click starts over");
        assert_eq!(c.press(9, 5), 1, "a different cell starts over");
    }
}
