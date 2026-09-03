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

use carrel::action::Targets;
use carrel::app::{App, Outcome, update};
use carrel::images::{self, ImageMsg};
use carrel::keys::Keys;
use carrel::render::{OscLink, Painted};
use carrel::{config, home, render, scan};
use carrel_core::{BlockIdx, Document, cluster_width, cols_for_doc_range, search, wrap};
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyEvent, KeyEventKind, MouseButton,
    MouseEvent, MouseEventKind,
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
    cmd | carrel                 read the pipe, streaming as it arrives
    cmd | carrel - <PATTERN>     print search results for the pipe and exit
    git show | carrel            a diff, as sections you can fold
    carrel --diff <FILE>         read FILE as a diff whatever it is called
    carrel --no-diff             never adapt a diff, even on a pipe
    carrel --no-mouse            hand the pointer back to the terminal, so
                                 its own selection and menus work as usual
    carrel --plain <FILE> [W]    the document as plain text (screen readers,
                                 pipes; a bare pipe does this by default)
    carrel --plain - [W]         piped input as plain text
    carrel --tasks <FILE>        the task list as checkbox lines, then exit
    carrel --render <FILE> [W]   styled ANSI text (attributes and links,
                                 never colours) for embedding elsewhere
    carrel --render - [W]        piped input as styled ANSI text
    carrel --help                (-h)
    carrel --version             (-V)

    NO_COLOR is honoured: colours off, weight and emphasis kept.
    Options come first; --diff and --no-diff are the exception and may
    appear anywhere. `--` ends the options, for a file whose name starts
    with a dash.
    Exit 1 on an unreadable file, an unknown or misplaced option, a [W]
    that is not a column count, or a search with no matches — so
    `if carrel FILE PATTERN; then` behaves like grep.

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
    Space b PgDn PgUp Ctrl-F Ctrl-B   page       42G             go to row 42
    / ?                          search          n N             next / previous
    zz zt zb                     put the current match middle / top / bottom
    o                            outline: jump to a section
    za zM zR                     fold this section or <details> / all / none
    t                            tables: cards ↔ wrapped
    r                            mermaid, math: rendered ↔ source
    m ' \"                        set a bookmark / go to the next / list them
    %                            footnote reference ↔ its definition
    L l                          what links here / what this points at
    I S                          document info card / spotlight the paragraph
    A                            auto-read: drift down until you scroll
    T                            cycle themes    q               close file
    h F1                         help            Q Ctrl-C        quit
    Tab Shift-Tab                select the next / previous link
    Enter                        follow the selected link
    Ctrl-O                       back
    Esc                          close an overlay / drop the selection
    H B                          hide the key hints / the breadcrumb
    ] [                          next / previous code block
    X                            jump to the next task
    y                            copy the code block
    F                            follow a document that is still arriving
    mouse: drag selects and copies; double-click a word, triple-click a block

    Diffs: a pipe, or a .diff/.patch file, is read as one — a heading per
    commit and per file, hunks as code. A .md file never is. Use it as
    git's pager with:  git config core.pager carrel
";

/// `--diff` / `--no-diff`, for the whole run. A `OnceLock` because the
/// entry points are many and threading one bool through all of them would
/// touch every signature for a flag almost nobody passes.
static DIFF_FORCED: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();

fn diff_forced() -> Option<bool> {
    DIFF_FORCED.get().copied().flatten()
}

/// Pull `--diff` / `--no-diff` out of the argument list.
fn take_diff_flag(args: &mut Vec<String>) -> Option<bool> {
    let mut forced = None;
    args.retain(|a| match a.as_str() {
        "--diff" => {
            forced = Some(true);
            false
        }
        "--no-diff" => {
            forced = Some(false);
            false
        }
        _ => true,
    });
    forced
}

/// `--no-mouse`, for the whole run. A `OnceLock` beside [`DIFF_FORCED`] and
/// for the same reason: the entry points are many, and threading a bool that
/// only the terminal guard reads through all of them would touch every
/// signature.
static MOUSE_OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// Is the pointer ours this run?
///
/// The flag wins over the config key, because a flag is this run and a config
/// file is every run. Both default to on: carrel is a click-first reader, and
/// the escape hatch is for the terminal that disagrees, not the common case.
fn mouse_enabled() -> bool {
    if MOUSE_OFF.get().copied().unwrap_or(false) {
        return false;
    }
    config::load_all().mouse.unwrap_or(true)
}

/// Pull `--no-mouse` out of the argument list, as the diff flags are pulled.
fn take_mouse_flag(args: &mut Vec<String>) -> bool {
    let mut off = false;
    args.retain(|a| {
        if a == "--no-mouse" {
            off = true;
            false
        } else {
            true
        }
    });
    off
}

/// A single-dash flag carrel does not know, arriving from something that
/// thinks it is talking to `less`. `-` itself is the stdin marker and is
/// never foreign; `--plain` and friends are carrel's own.
fn is_foreign_pager_flag(a: &str) -> bool {
    a.len() >= 2 && a.starts_with('-') && !a.starts_with("--") && !matches!(a, "-h" | "-V")
}

fn main() -> ExitCode {
    // no-color.org: any non-empty value means monochrome. Once, at startup —
    // the flag is process-global presentation state, like the theme.
    if std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()) {
        carrel::theme::set_mono(true);
    }
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let diff_forced = take_diff_flag(&mut args);
    let mouse_off = take_mouse_flag(&mut args);

    // Pager mode: git hands its pager flags it does not expect to be
    // understood (`-R`, `-F`, `-X`, …). Ignore unknown single-dash flags
    // when stdin is a pipe, and keep erroring on them otherwise — silently
    // swallowing a typo in interactive use would be worse than the error.
    if !std::io::stdin().is_terminal() {
        args.retain(|a| !is_foreign_pager_flag(a));
    }
    DIFF_FORCED.set(diff_forced).ok();
    MOUSE_OFF.set(mouse_off).ok();
    // `--` ends the options, so `carrel -- ./-weird.md` opens a file whose
    // name begins with a dash. Without it such a file was unopenable. Only
    // what precedes it is checked for flags.
    let operands_from = match args.iter().position(|a| a == "--") {
        Some(i) => {
            args.remove(i);
            i
        }
        None => args.len(),
    };
    if let Some(complaint) = flag_complaint(&args[..operands_from]) {
        eprintln!("carrel: {complaint}");
        eprint!("{USAGE}");
        return ExitCode::FAILURE;
    }
    match args.as_slice() {
        [] if !std::io::stdin().is_terminal() => open_stdin(None, false),
        [] => open_home(None),
        [a] if a == "-h" || a == "--help" => emit(USAGE),
        [a] if a == "-V" || a == "--version" => {
            emit(&format!("carrel {}\n", env!("CARGO_PKG_VERSION")))
        }
        [a] if a == "-" => open_stdin(None, true),
        [a, pattern] if a == "-" => open_stdin(Some(pattern), true),
        [p] if Path::new(p).is_dir() => open_home(Some(Path::new(p))),
        [flag, a] if flag == "--plain" && a == "-" => print_plain_stdin(80),
        [flag, a, w] if flag == "--plain" && a == "-" => match width_arg(w) {
            Ok(w) => print_plain_stdin(w),
            Err(code) => code,
        },
        [flag, file] if flag == "--tasks" => print_tasks(Path::new(file)),
        [flag, a] if flag == "--render" && a == "-" => print_ansi(None, 80),
        [flag, a, w] if flag == "--render" && a == "-" => match width_arg(w) {
            Ok(w) => print_ansi(None, w),
            Err(code) => code,
        },
        [flag, file] if flag == "--render" => print_ansi(Some(Path::new(file)), 80),
        [flag, file, w] if flag == "--render" => match width_arg(w) {
            Ok(w) => print_ansi(Some(Path::new(file)), w),
            Err(code) => code,
        },
        [flag, file] if flag == "--plain" => print_plain(Path::new(file), 80),
        [flag, file, w] if flag == "--plain" => match width_arg(w) {
            Ok(w) => print_plain(Path::new(file), w),
            Err(code) => code,
        },
        [file] => open(Path::new(file), None),
        [file, pattern] => open(Path::new(file), Some(pattern)),
        _ => {
            eprint!("{USAGE}");
            ExitCode::FAILURE
        }
    }
}

/// A `[W]` argument, or a refusal.
///
/// `w.parse().unwrap_or(80)` silently rendered at 80 columns for `10O` (letter
/// O), `-5`, or a number past `u16` — so a typo produced plausible-looking but
/// wrong output and exited 0, with nothing to notice.
fn width_arg(w: &str) -> Result<u16, ExitCode> {
    w.parse::<u16>().map_err(|_| {
        eprintln!("carrel: {w:?} is not a width — expected a column count like 72");
        ExitCode::FAILURE
    })
}

/// Every option carrel knows. All of them lead: there is no form where one
/// follows a path.
const KNOWN_FLAGS: &[&str] = &[
    "-h",
    "--help",
    "-V",
    "--version",
    "--plain",
    "--render",
    "--tasks",
    "--diff",
    "--no-diff",
    "--no-mouse",
];

/// Why `args` cannot be dispatched, if it cannot.
///
/// Every one- and two-argument typo used to be swallowed by the `[file]` and
/// `[file, pattern]` arms before it could reach the usage fallback:
/// `carrel --verbose` reported "No such file or directory", and
/// `carrel doc.md --plain` printed a search report for the literal pattern
/// `--plain` and exited 0 — a typo producing output that looks real.
///
/// `-` is the stdin marker, and `--` has already been stripped by the caller
/// along with everything after it, so a file named `./-weird.md` stays
/// openable.
fn flag_complaint(args: &[String]) -> Option<String> {
    if let Some(bad) = args
        .iter()
        .find(|a| a.len() > 1 && a.starts_with('-') && !KNOWN_FLAGS.contains(&a.as_str()))
    {
        return Some(format!("unknown option {bad}"));
    }
    args.iter()
        .skip(1)
        .find(|a| KNOWN_FLAGS.contains(&a.as_str()))
        .map(|f| format!("{f} is an option, and options come first"))
}

/// `--render`: styled linear text to stdout. Attributes and OSC 8 links,
/// never colours — `NO_COLOR` reduces it to `--plain` exactly.
fn print_ansi(path: Option<&Path>, width: u16) -> ExitCode {
    let src = match path {
        Some(p) => carrel::app::read_document(p),
        None => read_stdin_capped(),
    };
    match src {
        Ok(src) => {
            let doc = carrel_core::Document::parse(&src);
            emit(&carrel::ansi::render(&doc, width))
        }
        Err(e) => {
            eprintln!("carrel: {e}");
            ExitCode::FAILURE
        }
    }
}

/// `--tasks`: the report goes to stdout and the process exits. A file that
/// fails to read or parse says so on stderr rather than printing nothing.
fn print_tasks(path: &Path) -> ExitCode {
    match carrel::app::read_document(path) {
        Ok(src) => {
            let doc = carrel_core::Document::parse(&src);
            let report = carrel::plain::task_report(&doc);
            if report.is_empty() {
                eprintln!("carrel: no task lists in {}", path.display());
                return ExitCode::FAILURE;
            }
            emit(&report)
        }
        Err(e) => {
            eprintln!("carrel: cannot read {}: {e}", path.display());
            ExitCode::FAILURE
        }
    }
}

/// Read all of stdin, refusing at the 32-bit position ceiling exactly as
/// `check_document_size` does for files.
fn read_stdin_capped() -> std::io::Result<String> {
    use std::io::Read;
    let mut src = String::new();
    std::io::stdin()
        .lock()
        .take(u64::from(u32::MAX))
        .read_to_string(&mut src)?;
    if src.len() >= u32::MAX as usize {
        return Err(std::io::Error::other(
            "stdin exceeds 4 GiB; carrel positions are 32-bit byte offsets",
        ));
    }
    Ok(src)
}

/// A document arriving on a pipe. `forced` is `carrel -`.
fn open_stdin(pattern: Option<&str>, forced: bool) -> ExitCode {
    if std::io::stdin().is_terminal() {
        debug_assert!(forced);
        let _ = forced;
        eprintln!("carrel: stdin is a terminal; pipe a document in or pass a file");
        return ExitCode::FAILURE;
    }
    // A pattern means the non-interactive report, exactly as it does for
    // files — stdout's tty-ness notwithstanding.
    if let Some(p) = pattern {
        return match read_stdin_capped() {
            Ok(src) => {
                let (text, found) = report(Path::new("(stdin)"), &src, p);
                let code = emit(&text);
                // Exit 1 on no match, as grep, rg and ag all do — the README
                // sells this mode for scripting, and `if carrel - pat; then`
                // was always true.
                if found { code } else { ExitCode::FAILURE }
            }
            Err(e) => {
                eprintln!("carrel: {e}");
                ExitCode::FAILURE
            }
        };
    }
    if !std::io::stdout().is_terminal() {
        // Both ends piped: the Q17 rule — a pipe wants the document.
        return print_plain_stdin(80);
    }
    run_stdin_or_fallback()
}

/// The streaming TUI, or — where no tty can be opened at all (a truly
/// detached environment) — the plain fallback, so the spin-at-EOF failure
/// mode from the gotchas stays impossible: we never enter the event loop
/// without a working tty source.
fn run_stdin_or_fallback() -> ExitCode {
    let rx = carrel::stream::spawn();
    if let Ok(()) = run_stdin(&rx) {
        ExitCode::SUCCESS
    } else {
        // The reader thread owns stdin now; collect through its channel
        // — it reads to EOF regardless, so this is the whole document.
        let src: String = rx.iter().collect();
        let doc = carrel::app::adapt(&src, diff_forced().unwrap_or(true));
        emit(&carrel::plain::render(&doc, 80))
    }
}

/// The continue-reading rows: remembered documents that still exist and
/// that the reader is genuinely part-way through.
///
/// A document at 0% is one that was opened and not read; at 100% it is
/// finished. Neither is something to continue, and offering them would make
/// the list noise rather than an answer.
/// Start, feed, or stop the backlinks query as the pane opens and closes.
///
/// The scan the home screen builds is not available here — a document opened
/// directly never had one — so the walk is rooted at the document's own
/// directory, which is also the only place a relative link could point.
fn drive_backlinks(
    app: &mut App,
    rx: &mut Option<Receiver<carrel::links::Msg>>,
    started_for: &mut Option<PathBuf>,
) {
    use std::sync::mpsc::TryRecvError;

    // Closed: drop the receiver, which stops the thread on its next send.
    let Some(_) = app.backlinks.as_ref() else {
        *rx = None;
        *started_for = None;
        return;
    };
    let Some(file) = app.file.clone() else { return };

    if started_for.as_deref() != Some(file.as_path()) {
        let root = file
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        let (entries, _) = scan::walk_blocking(&root);
        *rx = Some(carrel::links::spawn(entries, file.clone(), 0));
        *started_for = Some(file);
        return;
    }

    let Some(chan) = rx.as_ref() else { return };
    loop {
        match chan.try_recv() {
            Ok(carrel::links::Msg::Found(b, _)) => {
                if let Some(pane) = app.backlinks.as_mut() {
                    pane.rows.push(b);
                }
            }
            Ok(carrel::links::Msg::Done(_)) | Err(TryRecvError::Disconnected) => {
                if let Some(pane) = app.backlinks.as_mut() {
                    pane.done = true;
                }
                *rx = None;
                break;
            }
            Err(TryRecvError::Empty) => break,
        }
    }
}

/// Home-screen preferences that live on `Home` rather than `App`.
fn set_home_prefs(app: &mut App) {
    let c = config::load_all();
    if let Some(h) = app.home_mut() {
        h.show_titles = c.titles.unwrap_or(false);
        h.places = c.places;
    }
}

/// Read titles for the rows about to be painted, and cache them.
///
/// Lazy on purpose: fourteen file heads per frame is free, and the whole
/// index is not. Runs before the draw so a title appears on the frame the
/// row does, rather than one frame late.
fn fill_titles(app: &mut App) {
    let Some(h) = app.home() else { return };
    let wanted = h.visible_entries(app.cols, app.rows, app.hints);
    if wanted.is_empty() {
        return;
    }
    let read: Vec<_> = wanted
        .into_iter()
        .map(|e| ((e.path.clone(), e.mtime), scan::title_of(&e.path)))
        .collect();
    if let Some(h) = app.home_mut() {
        h.cache_titles(read);
    }
}

/// Fill the home screen's continue-reading band.
///
/// Read once at startup: it is a small file, and a list that changed under
/// the reader would be worse than one that is a few seconds stale.
fn load_resume(app: &mut App) {
    let Some(dir) = app.state_dir.clone() else {
        return;
    };
    let resume = resume_rows(&dir);
    if let Some(h) = app.home_mut() {
        h.resume = resume;
    }
}

fn resume_rows(dir: &Path) -> Vec<carrel::home::Resume> {
    carrel::state::recent_in(dir)
        .into_iter()
        .filter_map(|e| {
            let path = PathBuf::from(&e.path);
            // The disk check is the only part that needs one; the rule about
            // what counts as "part way through" is pure and lives in `home`.
            path.is_file()
                .then(|| carrel::home::resume_from(path, e.permille, e.words))
                .flatten()
        })
        .take(carrel::home::RESUME_ROWS)
        .collect()
}

/// Whether a path is allowed to be sniffed as a diff. `.md` never is.
fn diff_ok_for(path: &Path) -> bool {
    diff_forced().unwrap_or_else(|| {
        matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("diff" | "patch")
        )
    })
}

/// `--plain -`: piped input as linear text at a width.
fn print_plain_stdin(width: u16) -> ExitCode {
    match read_stdin_capped() {
        Ok(src) => {
            let doc = carrel::app::adapt(&src, diff_forced().unwrap_or(true));
            emit(&carrel::plain::render(&doc, width))
        }
        Err(e) => {
            eprintln!("carrel: {e}");
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
    use std::fmt::Write as _;
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
        let mut out = format!(
            "{} markdown file(s) under {}\n",
            entries.len(),
            root.display()
        );
        for e in entries.iter().take(50) {
            let _ = writeln!(out, "  {}", e.path.display());
        }
        return emit(&out);
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
    let src = match carrel::app::read_document(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("carrel: {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
    };

    // With a pattern, stay non-interactive: useful for scripting and for
    // checking the core without a terminal.
    if let Some(p) = pattern {
        let (text, found) = report(path, &src, p);
        let code = emit(&text);
        return if found { code } else { ExitCode::FAILURE };
    }

    // No terminal, no TUI. `less` behaves the same way, and the alternative is
    // ugly: with stdin at EOF, `event::poll` reports ready forever and the
    // reader spins at 100% CPU redrawing frames nobody can see. Found by
    // running the binary under a pty with piped input — no unit test reaches
    // this, because `TestBackend` never touches a real stdin.
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        // Piping implies plain (Q17): a pipe wants the document, not a
        // summary — and linear text is also what a screen reader can use.
        let doc = carrel::app::adapt(&src, diff_ok_for(path));
        return emit(&carrel::plain::render(&doc, 80));
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

use ratatui::crossterm::cursor::Show;
use ratatui::crossterm::terminal::EndSynchronizedUpdate;

impl TerminalGuard {
    fn engage_mouse() -> Self {
        // `--no-mouse` / `mouse = false`: never turn capture on in the first
        // place. Nothing else needs a branch — with capture off the terminal
        // keeps every pointer event, so no `MouseEvent` ever reaches the loop
        // and every click target is simply never consulted.
        if mouse_enabled() {
            let _ = ratatui::crossterm::execute!(std::io::stdout(), EnableMouseCapture);
        }
        // Chain in FRONT of ratatui's restoring panic hook. Mouse capture is
        // ours; so is synchronized-update mode, which `paint` opens around
        // every frame. A panic inside `draw` unwinds past the `End`, and
        // ratatui's hook does not know about mode 2026 — so without this the
        // terminal is left buffering and the shell prompt may never appear.
        // Some terminals time mode 2026 out on their own; that is the
        // terminal's mercy, not this program's correctness.
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = ratatui::crossterm::execute!(
                std::io::stdout(),
                EndSynchronizedUpdate,
                DisableMouseCapture,
                Show
            );
            prev(info);
        }));
        Self::watch_for_signals();
        Self
    }

    /// Restore the terminal when the process is signalled rather than exiting.
    ///
    /// `Drop` covers every ordinary path and the panic hook covers panics, but
    /// a default-disposition signal runs neither — so `pkill carrel`, a
    /// session manager tearing down at logout, or killing a reader that is
    /// wedged on a slow read all left the terminal in the alternate screen
    /// with mouse capture on and the cursor hidden, needing `reset`. The
    /// comment above says this project refuses to inflict that kind of exit
    /// damage; this is the half that was missing.
    ///
    /// Raw mode clears ISIG, so Ctrl-C reaches the reader as a key and never
    /// as SIGINT — but an explicit `kill -INT` still arrives here.
    ///
    /// `signal-hook`'s iterator delivers on an ordinary thread through a
    /// self-pipe, so the restore runs as normal Rust rather than inside a
    /// signal handler, where `execute!` would not be async-signal-safe.
    /// Exiting with 128 + signal is the shell's convention for it.
    fn watch_for_signals() {
        use signal_hook::consts::{SIGHUP, SIGINT, SIGQUIT, SIGTERM};
        let Ok(mut signals) =
            signal_hook::iterator::Signals::new([SIGTERM, SIGHUP, SIGINT, SIGQUIT])
        else {
            return; // Nothing to do but leave the old behaviour in place.
        };
        std::thread::spawn(move || {
            if let Some(sig) = signals.forever().next() {
                let _ = ratatui::crossterm::execute!(
                    std::io::stdout(),
                    EndSynchronizedUpdate,
                    DisableMouseCapture,
                    Show
                );
                ratatui::restore();
                std::process::exit(128 + sig);
            }
        });
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        // No `EndSynchronizedUpdate` here: on an ordinary exit `paint` has
        // already closed the update it opened, and emitting a second one
        // unbalances the pairing that `every_frame_is_bracketed_in_a_\
        // synchronized_update` checks. Only the two ABNORMAL paths — a panic
        // unwinding out of `draw`, and a signal arriving mid-frame — can
        // leave one open, and both close it themselves.
        let _ = ratatui::crossterm::execute!(std::io::stdout(), DisableMouseCapture, Show);
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

/// How long after a walk finishes before the home screen quietly walks the
/// tree again, so a file created while it is up appears without a restart.
///
/// Measured from the END of the previous walk, not the start: a slow root — a
/// cold cache, a network mount — then backs itself off instead of queueing
/// walks nose to tail. Two seconds reads as "immediately" to a person, and the
/// walk itself is 6 ms warm over 110k files (see `scan.rs`).
const RESCAN_EVERY: Duration = Duration::from_secs(2);

/// The home screen's background walk, and what the event loop remembers about
/// it. Loop bookkeeping, so it lives here rather than in `Home` — same reason
/// the theme and the click streak do.
struct Scan {
    rx: Option<std::sync::mpsc::Receiver<scan::Msg>>,
    /// A walk nobody asked for: it reports nothing to the screen and does not
    /// hurry the loop along.
    quiet: bool,
    /// The list moved during this walk, so the index cache is worth rewriting.
    changed: bool,
    /// When the next quiet rescan falls due.
    next: Instant,
}

impl Scan {
    fn start(root: &Path) -> Self {
        Self {
            rx: Some(scan::spawn(root)),
            quiet: false,
            changed: false,
            next: Instant::now() + RESCAN_EVERY,
        }
    }

    /// A new root: throw the old walk away and start the new one openly.
    fn restart(&mut self, root: &Path) {
        *self = Self::start(root);
    }

    /// Walk the same root again, quietly.
    fn rescan(&mut self, root: &Path) {
        self.rx = Some(scan::spawn(root));
        self.quiet = true;
        self.changed = false;
    }

    /// Idle, and due for another look at the tree.
    fn due(&self) -> bool {
        self.rx.is_none() && Instant::now() >= self.next
    }

    /// Worth waking often for. A quiet rescan is not: the list is already
    /// painted, so there is nothing to stream into view and no reason to
    /// repaint at 60 Hz every couple of seconds.
    fn busy(&self) -> bool {
        self.rx.is_some() && !self.quiet
    }

    /// The walk ended. Time the next rescan from HERE, not from its start, so
    /// a slow root — a cold cache, a network mount — backs itself off instead
    /// of queueing walks nose to tail.
    fn ended(&mut self) {
        self.rx = None;
        self.quiet = false;
        self.next = Instant::now() + RESCAN_EVERY;
    }
}

/// Files created while the home screen is up should appear on it.
///
/// There is no notify dependency: the reader watches its own file by polling
/// mtime for the reasons in the wave-D spec, and this is that same decision
/// applied to the list. Never while a content search is running — the grep is
/// already walking this tree — and never from the reader, where the list is
/// not on screen and the walk would be work for nobody.
fn maybe_rescan(app: &mut App, scan: &mut Scan, root: &Path, grep: &Grep) {
    if !scan.due() || !app.is_home() || grep.busy() {
        return;
    }
    let Some(h) = app.home_mut() else { return };
    h.begin_rescan();
    scan.rescan(root);
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

/// `omarchy theme set` restyles the whole desktop; this is how a reader that
/// is already open follows it rather than staying on last week's colours.
///
/// The file is re-read and re-parsed rather than stat'ed. It is under 500
/// bytes; `current` is a symlink Omarchy swaps, so an mtime belongs to
/// whichever file it now points at rather than to the change itself; and two
/// themes written by the same installer in the same second with the same
/// length would be indistinguishable by stat. Parsing costs less than the
/// extra syscall that would try to avoid it.
struct Desktop {
    last: Instant,
    /// No palette file when the reader opened. Checked once, so a machine
    /// without Omarchy never opens a file it does not have.
    present: bool,
}

impl Desktop {
    fn new() -> Self {
        Self {
            last: Instant::now(),
            // Latch on whether the PATH resolves — a pure environment lookup,
            // no I/O — rather than on whether a palette parsed. The old test
            // meant "a palette installed at startup", so a reader launched
            // during an `omarchy theme set`, or before the file was first
            // generated, read `None` once and never looked again for the life
            // of the process.
            present: carrel::omarchy::path().is_some(),
        }
    }

    /// Re-read the desktop palette. Nothing to return: the loop repaints on
    /// every wake anyway, and ratatui writes only the cells that moved.
    fn poll(&mut self) {
        // Only re-read what something is actually reading FROM. `DESKTOP` is
        // consulted only while the omarchy palette is active, so a reader on
        // gruvbox was opening and parsing a file 3600 times an hour for a
        // value nothing looked at — and defeating idle detection with it.
        if !self.present
            || carrel::theme::current_name() != carrel::theme::OMARCHY
            || self.last.elapsed() < RELOAD_POLL
        {
            return;
        }
        self.last = Instant::now();
        if let Some(c) = carrel::omarchy::load() {
            carrel::theme::install_omarchy(&c);
        }
    }
}

/// A wheel notch is three lines; spinning faster than this makes it more.
///
/// A terminal cannot scroll by a fraction of a row, so the smooth curve a GUI
/// uses is not available — but the part of it a hand actually feels is the
/// velocity, not the interpolation. Notches arriving inside this window
/// compound the step; a pause, or a reversal, drops straight back to three so
/// that a correction stays precise.
const WHEEL_WINDOW: Duration = Duration::from_millis(80);
const WHEEL_STEP: i32 = 3;
const WHEEL_MAX: i32 = 12;

#[derive(Default)]
struct Wheel {
    /// When the last notch arrived, and which way it went.
    last: Option<(Instant, bool)>,
    step: i32,
}

impl Wheel {
    /// The number of lines this notch should move.
    fn notch(&mut self, down: bool) -> i32 {
        let now = Instant::now();
        let fast = self
            .last
            .is_some_and(|(t, d)| d == down && now.duration_since(t) < WHEEL_WINDOW);
        self.step = if fast {
            (self.step + WHEEL_STEP).min(WHEEL_MAX)
        } else {
            WHEEL_STEP
        };
        self.last = Some((now, down));
        self.step
    }
}

/// Everything the event loop remembers about the pointer between events.
///
/// Presentation state, so it lives here rather than in `App` — the same rule
/// that keeps the active palette out of the state layer. Bundled because all
/// three are threaded through the same two handlers.
#[derive(Default)]
struct Pointer {
    /// Where on the scrollbar thumb the hand took hold, while a drag is in
    /// flight. `None` when nothing is being dragged.
    dragging: Option<u16>,
    clicks: Clicks,
    wheel: Wheel,
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
        // Within one cell, not on it: a hand drifts between presses, and an
        // exact-cell rule silently downgraded a double-click to two singles.
        let same = self.last.is_some_and(|(t, c, r)| {
            now.duration_since(t) < MULTI_CLICK && c.abs_diff(col) <= 1 && r.abs_diff(row) <= 1
        });
        self.streak = if same { (self.streak % 3) + 1 } else { 1 };
        self.last = Some((now, col, row));
        self.streak
    }
}

/// The `(start, end)` doc bytes of the grapheme cluster under a pointer cell.
///
/// Lives on `App` now — it is pure state logic with no terminal in it, so it
/// belongs where a test can reach it and the GTK frontend can reuse it.
fn doc_span_at(app: &App, col: u16, row: u16) -> Option<(u32, u32)> {
    app.doc_span_at(col, row)
}

/// Copy to the system clipboard via OSC 52. Terminals without it ignore the
/// sequence — the same graceful degradation as OSC 8.
fn osc52(text: &str) -> std::io::Result<()> {
    // `copy_selection` caps a DRAG at 100 KiB, but `YankBlock` copies a whole
    // code block with no bound — a generated document with a 2 MB embedded
    // blob produced a ~2.7 MB escape sequence written synchronously to the tty
    // from the UI thread. Terminals cap OSC string length and truncate or fall
    // back to printing the tail as literal text, so the large case was never
    // going to work; refusing it is the honest outcome.
    if text.len() > carrel::app::CLIPBOARD_MAX {
        return Ok(());
    }
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
    // `.md` is never sniffed. `.diff`/`.patch` always are. `--diff` wins.
    let diff_ok = diff_forced().unwrap_or_else(|| {
        matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("diff" | "patch")
        )
    });
    let doc = carrel::app::adapt(src, diff_ok);

    // Protocol + font detection queries the terminal, so it must happen
    // before the alternate screen swallows the replies.
    let images = Images::detect();

    let terminal = ratatui::init();
    let _guard = TerminalGuard::engage_mouse();

    let size = terminal.size()?;
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let mut app = App::new(name, doc, size.width, size.height);
    app.diff_ok = diff_ok;
    app.diff_forced = diff_forced();
    app.file = Some(path.to_path_buf());
    app.note = theme_note;
    app.state_dir = carrel::state::state_dir();
    // Rooted at the document's own directory: links inside the project follow
    // silently, links out of it ask first. See `App::escapes_library`.
    // `Path::new("doc.md").parent()` is `Some("")`, which canonicalizes to an
    // error and would silently disable containment — the working directory is
    // what an empty parent means.
    app.library_root = Some(match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    });
    apply_config(&mut app);
    app.on_resize(app.cols, app.rows);
    // A direct open builds the App by hand rather than via open_path, so the
    // saved reading position needs restoring explicitly. Its note outranks
    // the theme note — the theme is only news when it failed to load.
    app.restore_position();
    run_loop(terminal, app, images, None)
}

/// The reader's event loop, shared by the file entry (`run`) and the piped
/// entry (`run_stdin`). `stream` is the stdin channel when the document is
/// arriving live; `None` for a file.
fn run_loop(
    mut terminal: ratatui::DefaultTerminal,
    mut app: App,
    mut images: Images,
    mut stream: Option<&Receiver<String>>,
) -> std::io::Result<()> {
    let mut keys = Keys::new();

    let mut pending: Option<(u16, u16)> = None;
    let mut deadline: Option<Instant> = None;
    let mut painted = Painted::default();
    let mut repaint = true;
    let mut ptr = Pointer::default();
    let mut reloader = Reloader::new();
    let mut desktop = Desktop::new();
    let mut diagrams = Diagrams::new();
    // The backlinks query: started when the pane opens, dropped when it
    // closes — which is what stops an abandoned walk on its next send.
    let mut backlinks: Option<Receiver<carrel::links::Msg>> = None;
    let mut backlinks_for: Option<PathBuf> = None;
    let mut next_auto: Option<Instant> = None;

    loop {
        drive_backlinks(&mut app, &mut backlinks, &mut backlinks_for);
        images.sync(&mut app);
        images.drain(&mut app);
        diagrams.sync(&app);
        diagrams.drain(&mut app);
        if repaint {
            paint(&mut terminal, &app, &mut painted, &mut images.protocols)?;
        }
        repaint = true; // only a motion burst turns this off, and only briefly

        // Auto-read's heartbeat: schedule on the same wake budget as the
        // resize debounce, so one `poll` timeout serves both clocks.
        if app.auto_read && next_auto.is_none() {
            next_auto = Some(Instant::now() + Duration::from_millis(carrel::app::AUTO_READ_MS));
        } else if !app.auto_read {
            next_auto = None;
        }
        let auto_timeout = next_auto.map_or(Duration::MAX, |d| {
            d.saturating_duration_since(Instant::now())
        });
        // Wake in time to apply a debounced resize even with no input arriving.
        let timeout = deadline
            .map_or(Duration::from_millis(100), |d| {
                d.saturating_duration_since(Instant::now())
            })
            .min(auto_timeout);

        if event::poll(timeout)? {
            // A read error means the input stream is gone. Exit rather than
            // spin: `poll` keeps reporting ready once stdin is at EOF.
            let Ok(ev) = event::read() else { break };
            match ev {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    let action = key_action(&mut keys, &app, k);
                    if let Some(action) = action {
                        if action == carrel::action::Action::ThemeCycle {
                            cycle_theme_now(&mut app);
                        } else if update(&mut app, action) == Outcome::Quit {
                            break;
                        }
                    }
                }
                Event::Mouse(m) => match mouse_action(m, &app, &painted.targets, &mut ptr) {
                    Some(action) => {
                        if update(&mut app, action) == Outcome::Quit {
                            break;
                        }
                    }
                    None => repaint = !coalescing_motion(m),
                },
                Event::Resize(w, h) => {
                    pending = Some((w, h));
                    deadline = Some(Instant::now() + DEBOUNCE);
                }
                _ => {}
            }
        }

        drain_outboxes(&mut app)?;

        poll_reload(&mut reloader, &mut app, &mut images, &mut diagrams);
        poll_stream(&mut stream, &mut app, &mut images, &mut diagrams);
        desktop.poll();

        // Auto-read's own clock, and NOT nested in the resize branch below:
        // `pending` is only ever set by `Event::Resize`, so a tick guarded by
        // it fires only while a window is being dragged. That is where this
        // block used to live, which made `A` scroll nothing and — because
        // `next_auto` then never advanced — drove `auto_timeout` to zero and
        // spun the poll at 100% of a core. The unit tests could not see it:
        // they call `update(.., AutoTick)` directly.
        if let Some(t) = next_auto
            && Instant::now() >= t
        {
            next_auto = Some(Instant::now() + Duration::from_millis(carrel::app::AUTO_READ_MS));
            if update(&mut app, carrel::action::Action::AutoTick) == Outcome::Quit {
                break;
            }
        }

        apply_debounced_resize(&mut app, &mut pending, &mut deadline);
    }
    Ok(())
}

/// Drain the stdin channel: one re-parse per wake no matter how many chunks
/// arrived, and only while the piped document is the one on screen — after
/// following a link out, the text keeps accumulating in `app.piped` and the
/// `Ctrl-O` return re-parses it. Disconnect is EOF and flips the label.
fn poll_stream(
    stream: &mut Option<&Receiver<String>>,
    app: &mut App,
    images: &mut Images,
    diagrams: &mut Diagrams,
) {
    use std::sync::mpsc::TryRecvError;
    let Some(rx) = *stream else { return };
    let mut buf = app.piped.take().unwrap_or_default();
    let mut grew = false;
    let mut done = false;
    loop {
        match rx.try_recv() {
            Ok(chunk) => {
                if buf.len() + chunk.len() >= u32::MAX as usize {
                    app.set_note("stdin exceeds 4 GiB; showing what fits".into());
                    done = true;
                    break;
                }
                buf.push_str(&chunk);
                grew = true;
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                done = true;
                break;
            }
        }
    }
    if grew && app.file.is_none() {
        app.reload_from(&buf);
        images.for_file = None;
        diagrams.for_file = None;
        // Following is applied HERE, not in `update()`: the stream is drained
        // by the event loop, and no action fires when a chunk lands.
        if app.following {
            let h = app.text_h();
            app.view.scroll_to(&app.doc, &app.layout, u32::MAX, h);
        }
    }
    app.piped = Some(buf);
    if done {
        app.streaming = false;
        // Nothing left to follow; the lamp says `reading` again.
        app.following = false;
        if app.file.is_none() {
            app.path = "(stdin)".into();
        }
        *stream = None;
    }
}

/// The piped-document reader: the TUI opens immediately and content streams
/// in. Keys arrive through `/dev/tty` — crossterm's own fallback when stdin
/// is not a terminal — so this fails cleanly where no tty exists.
fn run_stdin(rx: &Receiver<String>) -> std::io::Result<()> {
    let theme_note = startup_theme();
    let images = Images::detect();
    let terminal = ratatui::try_init()?;
    let _guard = TerminalGuard::engage_mouse();

    let size = terminal.size()?;
    let mut app = App::new(
        "(stdin — streaming…)".into(),
        Document::parse(""),
        size.width,
        size.height,
    );
    app.streaming = true;
    // A pipe is the pager case: `git show | carrel` is the whole point.
    app.diff_ok = diff_forced().unwrap_or(true);
    app.diff_forced = diff_forced();
    app.piped = Some(String::new());
    app.note = theme_note;
    // No state_dir: a pathless document has no position to resume or save.
    app.library_root = std::env::current_dir().ok();
    apply_config(&mut app);
    app.on_resize(app.cols, app.rows);
    run_loop(terminal, app, images, Some(rx))
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
    app.state_dir = carrel::state::state_dir();
    app.library_root = Some(root.clone());
    apply_config(&mut app);
    load_resume(&mut app);
    set_home_prefs(&mut app);
    app.on_resize(app.cols, app.rows);
    if let Some(n) = note
        && let Some(h) = app.home_mut()
    {
        h.note = Some(n);
    }
    let mut keys = Keys::new();
    let mut ptr = Pointer::default();
    let mut reloader = Reloader::new();
    let mut desktop = Desktop::new();
    let mut diagrams = Diagrams::new();
    let mut scan = Scan::start(&root);
    let mut scan_root = root;

    // Content search (wave E): see `Grep`.
    let mut grep = Grep::new();

    let mut pending: Option<(u16, u16)> = None;
    let mut deadline: Option<Instant> = None;
    let mut painted = Painted::default();
    let mut repaint = true;

    loop {
        // Same draw path as run(): documents opened FROM the home screen must
        // get the same OSC 8 hyperlinks — and the same images — as documents
        // opened directly. On the home screen both are no-ops.
        images.sync(&mut app);
        images.drain(&mut app);
        diagrams.sync(&app);
        diagrams.drain(&mut app);
        fill_titles(&mut app);
        if repaint {
            paint(&mut terminal, &app, &mut painted, &mut images.protocols)?;
        }
        repaint = true; // only a motion burst turns this off, and only briefly

        drain_scan(&mut app, &mut scan);

        grep.tick(&mut app);

        maybe_rescan(&mut app, &mut scan, &scan_root, &grep);

        // While a scan or search is live, wake often enough to stream.
        let busy = scan.busy() || grep.busy();
        let idle = if busy { 16 } else { 100 };
        let timeout = deadline.map_or(Duration::from_millis(idle), |d| {
            d.saturating_duration_since(Instant::now())
        });

        if event::poll(timeout)? {
            let Ok(ev) = event::read() else { break };
            match ev {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    let action = key_action(&mut keys, &app, k);
                    if let Some(a) = action {
                        if a == carrel::action::Action::ThemeCycle {
                            cycle_theme_now(&mut app);
                        } else if update(&mut app, a) == Outcome::Quit {
                            break;
                        }
                    }
                }
                Event::Mouse(m) => match home_mouse_action(m, &app, &painted.targets, &mut ptr) {
                    Some(a) => {
                        if update(&mut app, a) == Outcome::Quit {
                            break;
                        }
                    }
                    None => repaint = !coalescing_motion(m),
                },
                Event::Resize(w, h) => {
                    pending = Some((w, h));
                    deadline = Some(Instant::now() + DEBOUNCE);
                }
                _ => {}
            }
        }

        drain_outboxes(&mut app)?;

        // And the same live reload (a no-op while the home screen is up —
        // there is no open file to watch), and the same desktop palette.
        poll_reload(&mut reloader, &mut app, &mut images, &mut diagrams);
        desktop.poll();

        apply_debounced_resize(&mut app, &mut pending, &mut deadline);

        // The picker changed root: restart the walk against the new one.
        if let Some(h) = app.home()
            && h.root != scan_root
        {
            scan_root.clone_from(&h.root);
            scan.restart(&scan_root);
        }
    }
    Ok(())
}

/// Apply a resize once its debounce has elapsed.
///
/// Both loops debounce the same way — one SIGWINCH per drag frame would
/// otherwise rebuild the layout on every cell of a window drag — so they share
/// the code rather than each keeping a copy of the condition.
fn apply_debounced_resize(
    app: &mut App,
    pending: &mut Option<(u16, u16)>,
    deadline: &mut Option<Instant>,
) {
    if deadline.is_some_and(|d| Instant::now() >= d)
        && let Some((w, h)) = pending.take()
    {
        *deadline = None;
        app.on_resize(w, h);
    }
}

/// Which key map owns this keystroke.
///
/// **The two loops used to answer differently.** `run()` consulted the
/// backlinks, forward-links and bookmark panes before the outline; `run_home()`
/// consulted only the outline. So a document opened FROM the home screen gave
/// those three panes the READER's keymap: with the pane up, `j` scrolled the
/// document underneath it instead of moving the pane's cursor, and `Enter`
/// followed a link instead of opening the selected row. Opening the same
/// document directly behaved correctly, which is what kept it hidden.
///
/// One function, both loops, so the answer cannot diverge again. The order is
/// the precedence: innermost overlay first, the home screen next, the reader
/// last.
fn key_action(keys: &mut Keys, app: &App, k: KeyEvent) -> Option<carrel::action::Action> {
    if app.backlinks.is_some() {
        Keys::map_backlinks(k)
    } else if app.forward.is_some() {
        Keys::map_forward(k)
    } else if app.mark_list.is_some() {
        Keys::map_marks(k)
    } else if app.outline.is_some() {
        Keys::map_outline(k)
    } else if app.is_home() {
        let mode = app.home().map_or(home::HomeMode::Filter, |h| h.mode);
        keys.map_home(k, mode)
    } else {
        keys.map(k, app.searching())
    }
}

/// Drain what `update` asked the outside world for.
///
/// The state machine does no I/O: it fills the outbox and the loop empties it.
/// Both loops call this, so a document opened FROM the home screen copies
/// exactly like one opened directly — the asymmetry that has bitten this file
/// before. A copy is the only thing that leaves here; carrel launches nothing.
fn drain_outboxes(app: &mut App) -> std::io::Result<()> {
    if let Some(text) = app.clipboard.take() {
        osc52(&text)?;
    }
    Ok(())
}

/// A pointer motion that changed nothing, with more input already queued.
///
/// `EnableMouseCapture` turns on mode 1003, so the terminal reports every cell
/// the pointer crosses — and `paint` runs at the top of every iteration, so a
/// mouse dragged across the window cost one full frame, plus its two
/// synchronized-update escapes, per cell, for a picture that did not change.
///
/// Skipping is only safe while another event is ALREADY waiting, because that
/// event drives the very next iteration: the burst coalesces and the frame
/// that ends it is painted. When the pointer stops, the queue empties, this
/// returns false, and the next iteration paints as it always did — so nothing
/// can be starved by a motion that is over.
fn coalescing_motion(m: MouseEvent) -> bool {
    matches!(m.kind, MouseEventKind::Moved) && event::poll(Duration::ZERO).unwrap_or(false)
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
    targets: &Targets,
    ptr: &mut Pointer,
) -> Option<carrel::action::Action> {
    use carrel::action::{Action, Span};
    use carrel::keys::{drag_target, thumb_geometry};

    // What the last frame painted wins, before any geometry is re-derived.
    // A target is only here because the painter put it here, so it agrees
    // with the pixels by construction — which is the whole reason chrome is
    // recorded rather than inverted.
    if matches!(m.kind, MouseEventKind::Down(MouseButton::Left))
        && let Some(hit) = targets.hit(m.column, m.row)
    {
        // `Absorb` is a pane saying the click was inside it and meant
        // nothing. It must stop here rather than fall through, or it reaches
        // the document underneath and starts a selection there.
        return (hit.action != Action::Absorb).then_some(hit.action);
    }

    let bar_x = app.cols.saturating_sub(1); // the scrollbar column
    let text_h = app.text_h();
    let total = app.layout.total_rows();
    let max_scroll = app.layout.max_scroll(text_h);
    // The bar's track starts at the text's top edge; pointer rows map into
    // it through the same accessor paint uses.
    let top = app.text_y();
    let bar_y = move |row: u16| row.saturating_sub(top);

    match m.kind {
        MouseEventKind::ScrollDown => Some(Action::Scroll(Span::Line, ptr.wheel.notch(true))),
        MouseEventKind::ScrollUp => Some(Action::Scroll(Span::Line, -ptr.wheel.notch(false))),
        // The lamp — lit on the footer, folded on the status row — is the
        // switch. Both live on the bottom row's first cells.
        MouseEventKind::Down(MouseButton::Left) if m.row + 1 == app.rows && m.column < 3 => {
            Some(Action::HintsToggle)
        }
        // The track is only as tall as the text. Without the row bounds this
        // guard claimed the whole rightmost COLUMN, so a click on the status
        // row's far right mapped past the thumb and paged the document. A
        // drag, once the thumb is grabbed, deliberately keeps tracking
        // outside the track — that is what every desktop scrollbar does.
        MouseEventKind::Down(MouseButton::Left)
            if m.column >= bar_x && max_scroll > 0 && m.row >= top && m.row < top + text_h =>
        {
            let (top, len) = thumb_geometry(text_h, total, app.view.scroll_row);
            let row = bar_y(m.row);
            if row >= top && row < top + len {
                // Grab. The view does not move until the hand does.
                ptr.dragging = Some(row - top);
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
            // The margin outline owns clicks in its own columns, before the
            // text does — a click there is a jump, not a selection.
            if let Some(b) = app.margin_row_at(m.column, m.row) {
                return Some(Action::OutlineJumpTo(b));
            }
            let span = doc_span_at(app, m.column, m.row)?;
            match ptr.clicks.press(m.column, m.row) {
                2 => Some(Action::SelectWord(span.0)),
                3 => Some(Action::SelectBlock(span.0)),
                _ => Some(Action::SelectAnchor(span)),
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => match ptr.dragging {
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
            if ptr.dragging.is_some() {
                ptr.dragging = None;
                None
            } else {
                Some(Action::SelectRelease)
            }
        }
        _ => None,
    }
}

/// The preferences every entry point applies to a fresh `App`.
///
/// `state_dir` is deliberately not here: a piped document has no path, so it
/// has no reading position to resume or save, and the entry point that knows
/// that is the one that must say so.
fn apply_config(app: &mut App) {
    // ONE read of the config file. Six wrappers each used to open and rescan
    // it independently, so startup read the same small file eight times.
    let c = config::load_all();
    app.config_dir = config::config_dir();
    // Read once, at startup, and never again: the picker's "here" has to be
    // the directory the command was typed in, and it must not drift.
    app.launch_dir = std::env::current_dir().ok().filter(|d| d.is_dir());
    app.hints = c.hints.unwrap_or(true);
    app.breadcrumb = c.breadcrumb.unwrap_or(true);
    app.outline_margin = c.outline_margin.unwrap_or(false);
    // The reading measure. Absent means the default; an explicit 0 means off.
    app.max_width = c.max_width.unwrap_or(config::DEFAULT_MEASURE);
}

/// Apply the saved theme. An unknown name falls back to `terminal` with a
/// status note — a stale config is never an error.
///
/// The desktop's own palette is installed first, so that `theme omarchy`
/// resolves and so that a reader with no theme on record opens wearing what
/// the rest of the desktop is wearing.
fn startup_theme() -> Option<String> {
    if let Some(c) = carrel::omarchy::load() {
        carrel::theme::install_omarchy(&c);
    }
    let Some(name) = config::load_theme() else {
        // Nothing chosen yet. Follow the desktop where there is one to
        // follow; `set_theme` declines and leaves `terminal` where there is
        // not, which is the same default carrel has always opened with.
        carrel::theme::set_theme(carrel::theme::OMARCHY);
        return None;
    };
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
///
/// A quiet rescan ([`Scan::quiet`]) reports nothing to the screen: interrupted,
/// it leaves the list it could not improve on standing, with no note. And the
/// index cache is rewritten only when the list actually moved — a rescan that
/// finds nothing must not rewrite a 110k-line file every couple of seconds.
fn drain_scan(app: &mut App, scan: &mut Scan) {
    let Some(rx) = scan.rx.as_ref() else { return };
    let mut batch: Vec<scan::Entry> = Vec::new();
    loop {
        match rx.try_recv() {
            Ok(scan::Msg::Found(e)) => batch.push(e),
            Ok(scan::Msg::Done { unreadable }) => {
                if let Some(h) = app.home_mut() {
                    scan.changed |= h.push_many(std::mem::take(&mut batch));
                    scan.changed |= h.finish_scan(unreadable);
                    if scan.changed {
                        scan::save_cache(&h.root, &h.entries);
                    }
                }
                scan.ended();
                return;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => break,
            // The walker died mid-scan. Keep everything — the cached entries
            // are still the best available answer — and say so, rather than
            // pruning down to whatever it reported.
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                if let Some(h) = app.home_mut() {
                    scan.changed |= h.push_many(std::mem::take(&mut batch));
                    if !scan.quiet {
                        h.abort_scan();
                    }
                }
                scan.ended();
                return;
            }
        }
    }
    if let Some(h) = app.home_mut()
        && !batch.is_empty()
    {
        scan.changed |= h.push_many(batch);
    }
}

/// Pointer events while the home screen is up: the wheel moves the list.
/// Once a document is open, the reader's full mouse handling applies.
fn home_mouse_action(
    m: MouseEvent,
    app: &App,
    targets: &Targets,
    ptr: &mut Pointer,
) -> Option<carrel::action::Action> {
    use carrel::action::Action;
    if !app.is_home() {
        return mouse_action(m, app, targets, ptr);
    }
    if matches!(m.kind, MouseEventKind::Down(MouseButton::Left))
        && let Some(hit) = targets.hit(m.column, m.row)
    {
        return (hit.action != Action::Absorb).then_some(hit.action);
    }
    match m.kind {
        // Deliberately not accelerated: this moves the SELECTION, and a
        // selection that gathers speed overshoots the row you were aiming
        // for. Acceleration belongs to scrolling, where nothing is being
        // pointed at.
        MouseEventKind::ScrollDown => Some(Action::HomeMove(3)),
        MouseEventKind::ScrollUp => Some(Action::HomeMove(-3)),
        // Same lamp, same switch as the reader's bottom row.
        MouseEventKind::Down(MouseButton::Left) if m.row + 1 == app.rows && m.column < 3 => {
            Some(Action::HintsToggle)
        }
        // A click on a file row: first press selects, second opens — the
        // file-manager idiom, and forgiving of a misclick. The row → index
        // mapping is `Home::row_at`, the inverse of the paint's own geometry.
        //
        // While the directory picker is up it owns the pointer, exactly as it
        // owns the keyboard; the list underneath must not take clicks through
        // the overlay.
        MouseEventKind::Down(MouseButton::Left) => {
            let home = app.home()?;
            if home.mode == carrel::home::HomeMode::Picker {
                let i = home.picker_row_at(m.column, m.row, app.cols, app.rows)?;
                return Some(match ptr.clicks.press(m.column, m.row) {
                    1 => Action::PickerSelect(i),
                    _ => Action::PickerChoose,
                });
            }
            // A continue row is its own affordance: it is numbered, and a
            // single click opens it. The file list keeps click-to-select /
            // double-click-to-open, because a misclick there is cheap and a
            // misclick here costs you the screen.
            if let Some(i) = home.resume_row_at(m.row, app.cols, app.rows) {
                return Some(Action::HomeResume(i));
            }
            let i = home.row_at(m.row, app.cols, app.rows, app.hints)?;
            match ptr.clicks.press(m.column, m.row) {
                1 => Some(Action::HomeSelect(i)),
                // The first press of the pair already selected it.
                _ => Some(Action::HomeOpen),
            }
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
/// One frame: the ratatui paint plus the OSC 8 hyperlink pass that rides on
/// top of it, together inside a single synchronized update so the terminal
/// never shows the two half-applied.
fn paint(
    terminal: &mut ratatui::DefaultTerminal,
    app: &App,
    painted: &mut Painted,
    images: &mut HashMap<BlockIdx, StatefulProtocol>,
) -> std::io::Result<()> {
    synchronized(|| {
        terminal.draw(|f| {
            render::draw_full(f, app, painted, images);
            render::declare_wide_cells(f);
        })?;
        emit_osc8(&painted.links)
    })
}

/// Paint one frame inside a synchronized update (DEC mode 2026).
///
/// Without it the terminal may render a half-applied frame. A slow scroll
/// changes few cells and lands within one refresh; a fast one rewrites most of
/// the screen, the update spans a refresh boundary, and characters from the
/// previous frame are still on screen where the new one has not arrived — read
/// as stray letters scattered "in places". The bytes were always correct; what
/// was missing was the frame boundary. `End` runs even when the paint fails,
/// or the screen would stay frozen. Terminals that do not know mode 2026
/// ignore both halves.
fn synchronized<T>(paint: impl FnOnce() -> std::io::Result<T>) -> std::io::Result<T> {
    use ratatui::crossterm::execute;
    use ratatui::crossterm::terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate};

    execute!(std::io::stdout(), BeginSynchronizedUpdate)?;
    let painted = paint();
    execute!(std::io::stdout(), EndSynchronizedUpdate)?;
    painted
}

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
/// Write to stdout, treating a closed pipe as success.
///
/// Rust ignores SIGPIPE, so `print!` PANICS on `EPIPE` — and `carrel doc.md |
/// head`, `carrel --plain x.md | less` and `git show | carrel | head` are all
/// ordinary invocations, the last especially since the README recommends
/// `git config core.pager carrel`. What the user saw was an internal Rust
/// panic and a backtrace hint:
///
/// ```text
/// $ carrel --plain big.md | head -2
/// thread 'main' panicked at library/std/src/io/stdio.rs:
/// failed printing to stdout: Broken pipe (os error 32)
/// note: run with `RUST_BACKTRACE=1` …
/// ```
///
/// A small document does not reproduce it: the whole thing fits the pipe
/// buffer and the write never fails. It needs an output larger than 64 KiB,
/// which is why this survived so long.
fn emit(s: &str) -> ExitCode {
    use std::io::Write;
    match std::io::stdout().lock().write_all(s.as_bytes()) {
        // The reader downstream stopped listening. That is `head` doing its
        // job, not an error, and every well-behaved Unix filter exits 0.
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("carrel: {e}");
            ExitCode::FAILURE
        }
    }
}

fn print_plain(path: &Path, width: u16) -> ExitCode {
    match carrel::app::read_document(path) {
        Ok(src) => {
            let doc = carrel::app::adapt(&src, diff_ok_for(path));
            emit(&carrel::plain::render(&doc, width))
        }
        Err(e) => {
            eprintln!("carrel: {}: {e}", path.display());
            ExitCode::FAILURE
        }
    }
}

fn report(path: &Path, src: &str, pattern: &str) -> (String, bool) {
    use std::fmt::Write as _;
    let mut out = String::new();
    let doc = Document::parse(src);
    let width: u16 = 80;

    let mut rows_total = 0u32;
    for i in 0..doc.block_count() {
        rows_total += wrap(&doc, BlockIdx(i as u32), width, &cluster_width, |_| {});
    }

    let _ = writeln!(out, "{}", path.display());
    let _ = writeln!(out, "  {} bytes source", src.len());
    let _ = writeln!(out, "  {} bytes display text", doc.text.len());
    let _ = writeln!(out, "  {} blocks", doc.block_count());
    let _ = writeln!(out, "  {rows_total} rows at width {width}");

    if pattern.is_empty() {
        return (out, true);
    }

    let matches = search(&doc, pattern, true);
    let found = !matches.is_empty();
    let _ = writeln!(out, "\n  {} match(es) for {pattern:?}", matches.len());

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
            let _ = writeln!(out, "    {}. cols {c0}..{c1}  │ {}", n + 1, text.trim());
        }
    }
    if matches.len() > 10 {
        let _ = writeln!(out, "    … and {} more", matches.len() - 10);
    }
    (out, found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
        }
    }

    /// The scrollbar is as tall as the text, not as tall as the terminal.
    ///
    /// The press guard read `m.column >= bar_x` and never bounded the row, so
    /// a click on the far right of the STATUS ROW — below the track, on
    /// chrome — mapped past the thumb and paged the document down.
    #[test]
    fn a_click_right_of_the_status_row_does_not_page_the_document() {
        use carrel::action::{Action, Span};
        let mut src = String::new();
        for n in 0..200 {
            use std::fmt::Write as _;
            let _ = write!(src, "line {n}\n\n");
        }
        let mut app = App::new("t.md".into(), carrel_core::Document::parse(&src), 80, 24);
        app.on_resize(80, 24);
        assert!(
            app.layout.max_scroll(app.text_h()) > 0,
            "the fixture must have a scrollbar"
        );

        let bar_x = app.cols - 1;
        let targets = Targets::new();
        let mut ptr = Pointer::default();

        let inside = mouse(
            MouseEventKind::Down(MouseButton::Left),
            bar_x,
            app.text_y() + 1,
        );
        assert!(
            mouse_action(inside, &app, &targets, &mut ptr).is_some() || ptr.dragging.is_some(),
            "a press on the track itself still acts"
        );

        let mut ptr = Pointer::default();
        let below = mouse(MouseEventKind::Down(MouseButton::Left), bar_x, app.rows - 1);
        assert_eq!(
            mouse_action(below, &app, &targets, &mut ptr),
            None,
            "the bottom row is chrome; clicking it must not scroll"
        );
        assert!(ptr.dragging.is_none(), "and must not grab the thumb");

        let above = mouse(MouseEventKind::Down(MouseButton::Left), bar_x, 0);
        let act = mouse_action(above, &app, &targets, &mut ptr);
        assert!(
            act != Some(Action::Scroll(Span::Page, 1)),
            "nor may a row above the track page forward"
        );
    }

    /// A pane's keymap must not depend on which loop is running.
    ///
    /// `run_home()` consulted only the outline, so a document opened from the
    /// home screen gave the backlinks, forward-links and bookmark panes the
    /// READER's keymap: `j` scrolled the document under the open pane instead
    /// of moving its cursor. Opening the same file directly was correct, which
    /// is exactly why nobody noticed.
    #[test]
    fn an_open_pane_owns_j_whichever_loop_is_running() {
        use carrel::action::Action;
        use ratatui::crossterm::event::{KeyCode, KeyModifiers};

        let j = KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE);
        let src = "see [a](https://example.com/a)\n";
        let mut app = App::new("t.md".into(), carrel_core::Document::parse(src), 60, 20);
        let mut keys = Keys::new();

        assert!(
            matches!(key_action(&mut keys, &app, j), Some(Action::Scroll(..))),
            "with no pane up, j scrolls the document"
        );

        update(&mut app, Action::ForwardToggle);
        assert!(app.forward.is_some());
        assert_eq!(
            key_action(&mut keys, &app, j),
            Some(Action::ForwardMove(1)),
            "with the forward pane up, j moves the PANE, not the document"
        );

        update(&mut app, Action::ForwardToggle);
        update(&mut app, Action::MarkToggle);
        update(&mut app, Action::MarkListToggle);
        assert_eq!(
            key_action(&mut keys, &app, j),
            Some(Action::MarkListMove(1)),
            "and the bookmark list owns it too"
        );
    }

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

    /// A hand drifts. Requiring the EXACT same cell turned a double-click
    /// into two single clicks whenever the pointer moved one column between
    /// presses, which is most of them — and this reader is aimed at people
    /// for whom the double-click IS the gesture. One cell of slack, the
    /// tolerance herdr settled on.
    #[test]
    fn a_double_click_survives_a_hand_that_drifts_one_cell() {
        let mut c = Clicks::default();
        assert_eq!(c.press(10, 5), 1);
        assert_eq!(c.press(11, 5), 2, "one column over is the same click");
        assert_eq!(c.press(11, 6), 3, "and one row over");

        let mut c = Clicks::default();
        assert_eq!(c.press(10, 5), 1);
        assert_eq!(c.press(12, 5), 1, "two columns is a new click");
        let mut c = Clicks::default();
        assert_eq!(c.press(10, 5), 1);
        assert_eq!(c.press(10, 7), 1, "and so is two rows");
    }

    #[test]
    fn a_spun_wheel_gathers_speed_and_a_reversed_one_does_not() {
        let mut w = Wheel::default();
        // A sustained spin compounds, and stops compounding at the cap.
        assert_eq!(w.notch(true), 3);
        assert_eq!(w.notch(true), 6);
        assert_eq!(w.notch(true), 9);
        assert_eq!(w.notch(true), 12);
        assert_eq!(w.notch(true), 12, "and no faster than the cap");

        // Turning back is a correction, and a correction must be precise.
        assert_eq!(w.notch(false), 3, "reversing drops to a single step");
        assert_eq!(w.notch(false), 6);

        // So is picking the wheel up again after a pause.
        w.last = Some((Instant::now().checked_sub(WHEEL_WINDOW * 2).unwrap(), false));
        assert_eq!(w.notch(false), 3, "a pause drops to a single step");
    }
}
