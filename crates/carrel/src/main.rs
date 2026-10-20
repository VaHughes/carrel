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
use carrel::app::{App, Outcome, Pane, update};
use carrel::images::{self, ImageMsg};
use carrel::keys::Keys;
use carrel::render::{OscLink, Painted};
use carrel::{config, home, render, scan};
use carrel_core::{BlockIdx, Document, cluster_width, cols_for_doc_range, search, wrap};
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;
use std::collections::HashMap;
use std::sync::mpsc::Receiver;

/// `eprintln!`, minus the panic.
///
/// `eprintln!` panics when stderr cannot be written — and the one time carrel
/// has something to say after the TUI is the time the terminal may have just
/// gone away. Release builds abort on panic, so a message nobody could read
/// became a core dump. There is no one to tell that the telling failed.
macro_rules! say {
    ($($t:tt)*) => {{
        let _ = writeln!(std::io::stderr(), $($t)*);
    }};
}

/// [`say!`] without the newline: `eprint!`, minus the panic.
macro_rules! say_bare {
    ($($t:tt)*) => {{
        let _ = write!(std::io::stderr(), $($t)*);
    }};
}

const USAGE: &str = "\
carrel — a quiet place to read your markdown

USAGE:
    carrel                       the home screen: what is around you to read
    carrel <DIR>                 the home screen, rooted at DIR
    carrel <FILE>                read a document
    carrel <FILE>:<LINE>         read it from that line of the file; FILE#section
                                 opens at a heading — the way a tool or an
                                 agent tells you where to look
    carrel --latest [DIR]        read the document written most recently, here
                                 or under DIR — what the agent just finished
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
    carrel --tutorial            the built-in first document: what carrel
                                 does, on a page you can try it all on
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
    #                            tags: a page of them, each with its documents
    s                            order: newest, by name, recently read
    gg G                         ends           d     choose a directory
    T                            cycle themes   q     quit
    h F1                         help
    click                        select a file; double-click opens it

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
    c                            after a reload: the next thing it changed
    #                            tags: a page of them, each with its documents
    y                            copy the code block
    v a                          highlight text / add a note
    V M                          notes list / next note
    Click image or Enter at it   full-screen image; [ ] browse, Esc closes
    F                            follow a document that is still arriving
    Mouse: click a link, a heading, a fold marker, a footnote mark, a tag,
    a row of any list or of the help sheet, or any hint along the bottom
    row. The status row names the documents behind you; click one to go
    back to it. Right-click for a menu of whatever is
    under the pointer; the ≡ at the end of the status row opens the global
    one. Drag selects and copies; double-click takes the word, triple-click
    the block. --no-mouse hands the pointer back to your terminal.

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
    let had_separator = args.iter().any(|a| a == "--");
    let operands_from = match args.iter().position(|a| a == "--") {
        Some(i) => {
            args.remove(i);
            i
        }
        None => args.len(),
    };
    if let Some(complaint) = flag_complaint(&args[..operands_from]) {
        say!("carrel: {complaint}");
        say_bare!("{USAGE}");
        return ExitCode::FAILURE;
    }
    // Everything after a leading `--` is a name, whatever it looks like:
    // `carrel -- --latest` opens the file called `--latest`.
    if had_separator && operands_from == 0 {
        return match args.as_slice() {
            [p] if Path::new(p).is_dir() => open_home(Some(Path::new(p))),
            [file] => open(file, None),
            [file, pattern] => open(file, Some(pattern)),
            _ => {
                say_bare!("{USAGE}");
                ExitCode::FAILURE
            }
        };
    }
    // `--plain PLAN.md:42` prints PLAN.md: the place is for a reader, and
    // refusing the name an agent printed because of it helps nobody.
    let file_of = |arg: &str| carrel::cli::split_target(arg).0;
    match args.as_slice() {
        [] if !std::io::stdin().is_terminal() => open_stdin(None, false),
        [] => open_home(None),
        [a] if a == "-h" || a == "--help" => emit(USAGE),
        // The built-in first document. It needs a terminal like any other
        // reading session; piped, it prints as plain text, which is what a
        // beginner running `carrel --tutorial | less` should get.
        [a] if a == "--tutorial" => {
            if std::io::stdout().is_terminal() && std::io::stdin().is_terminal() {
                match run_welcome() {
                    Ok(_) => ExitCode::SUCCESS,
                    Err(e) => {
                        say!("carrel: {e}");
                        ExitCode::FAILURE
                    }
                }
            } else {
                emit(&carrel::plain::render(
                    &carrel_core::Document::parse(carrel::app::WELCOME),
                    80,
                ))
            }
        }
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
        [flag, file] if flag == "--tasks" => print_tasks(&file_of(file)),
        [flag, a] if flag == "--render" && a == "-" => print_ansi(None, 80),
        [flag, a, w] if flag == "--render" && a == "-" => match width_arg(w) {
            Ok(w) => print_ansi(None, w),
            Err(code) => code,
        },
        [flag, file] if flag == "--render" => print_ansi(Some(&file_of(file)), 80),
        [flag, file, w] if flag == "--render" => match width_arg(w) {
            Ok(w) => print_ansi(Some(&file_of(file)), w),
            Err(code) => code,
        },
        [flag, file] if flag == "--plain" => print_plain(&file_of(file), 80),
        [flag, file, w] if flag == "--plain" => match width_arg(w) {
            Ok(w) => print_plain(&file_of(file), w),
            Err(code) => code,
        },
        [a] if a == "--latest" => open_latest(None),
        [a, dir] if a == "--latest" => open_latest(Some(Path::new(dir))),
        [file] => open(file, None),
        [file, pattern] => open(file, Some(pattern)),
        _ => {
            say_bare!("{USAGE}");
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
        say!("carrel: {w:?} is not a width — expected a column count like 72");
        ExitCode::FAILURE
    })
}

/// Every option carrel knows. All of them lead: there is no form where one
/// follows a path.
const KNOWN_FLAGS: &[&str] = &[
    "-h",
    "--help",
    "--tutorial",
    "-V",
    "--version",
    "--plain",
    "--render",
    "--tasks",
    "--diff",
    "--no-diff",
    "--no-mouse",
    "--latest",
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
        // One suggestion, when one option is near: `--plian` is `--plain`.
        return Some(match carrel::cli::suggest_flag(bad, KNOWN_FLAGS) {
            Some(near) => format!("unknown option {bad} — did you mean {near}?"),
            None => format!("unknown option {bad}"),
        });
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
            say!("carrel: {e}");
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
                say!("carrel: no task lists in {}", path.display());
                return ExitCode::FAILURE;
            }
            emit(&report)
        }
        Err(e) => {
            say!("carrel: cannot read {}: {e}", path.display());
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
        say!("carrel: stdin is a terminal; pipe a document in or pass a file");
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
                say!("carrel: {e}");
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
    if let Ok(home) = run_stdin(&rx) {
        match home {
            None => ExitCode::SUCCESS,
            Some(root) => open_home(Some(&root)),
        }
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
    app.titles = c.titles.unwrap_or(false);
    app.preview = c.preview.unwrap_or(true);
    let (titles, preview) = (app.titles, app.preview);
    // A word this version does not know is the default, not an error.
    let sort = c
        .sort
        .as_deref()
        .and_then(home::Sort::from_name)
        .unwrap_or_default();
    if let Some(h) = app.home_mut() {
        h.show_titles = titles;
        h.show_preview = preview;
        h.places = c.places;
        h.sort = sort;
        h.refilter();
    }
}

/// What the state file remembers about the documents under the root: how
/// far through each the reader is, and how recently they were in it.
///
/// Injected, as the continue band is, so the state layer is never reached
/// from library code. The state file keys documents by canonical path and
/// the walk spells them from the root as given, so each remembered path is
/// re-spelled the walk's way — one `canonicalize` of the root, none per file.
fn load_reading(app: &mut App) {
    let Some(dir) = app.state_dir.clone() else {
        return;
    };
    let Some(root) = app.home().map(|h| h.root.clone()) else {
        return;
    };
    let canon = root.canonicalize().unwrap_or_else(|_| root.clone());
    let reading: HashMap<PathBuf, home::Reading> = carrel::state::recent_in(&dir)
        .into_iter()
        .enumerate()
        .filter_map(|(rank, e)| {
            let path = PathBuf::from(&e.path);
            let spelled = path
                .strip_prefix(&canon)
                .map_or_else(|_| path.clone(), |rel| root.join(rel));
            Some((
                spelled,
                home::Reading {
                    permille: e.permille?,
                    rank: u32::try_from(rank).unwrap_or(u32::MAX),
                },
            ))
        })
        .collect();
    if let Some(h) = app.home_mut() {
        h.set_reading(reading);
    }
}

/// The preview beside the file list: the head of the highlighted document,
/// read on a thread so a slow disk never holds a frame.
///
/// One read in flight at a time. While it is out the highlight may move on;
/// an answer for a document that is no longer highlighted is dropped and the
/// next tick asks again for the one that is. That is the whole debounce:
/// holding `j` down costs one read at a time, not one per row passed.
struct PreviewLoad {
    rx: Option<Receiver<home::Preview>>,
}

impl PreviewLoad {
    fn busy(&self) -> bool {
        self.rx.is_some()
    }

    fn tick(&mut self, app: &mut App) {
        let cols = app.cols;
        let Some(h) = app.home() else {
            self.rx = None;
            return;
        };
        let Some(wanted) = h.preview_wanted(cols) else {
            // Nothing is wanted, so nothing is outstanding: a read left in
            // flight here kept this "busy", and the loop awake, until a
            // preview was wanted again.
            self.rx = None;
            return;
        };
        let Some((_, _, pane_w)) = home::preview_split(cols, h.show_preview) else {
            return;
        };
        let width = pane_w.saturating_sub(2);
        if let Some(rx) = self.rx.as_ref() {
            match rx.try_recv() {
                Ok(mut p) => {
                    self.rx = None;
                    if (p.path.clone(), p.mtime) == wanted {
                        p.lines = p
                            .text
                            .as_deref()
                            .map(|t| home::preview_lines(t, width))
                            .unwrap_or_default();
                        p.width = width;
                        if let Some(h) = app.home_mut() {
                            h.preview = Some(p);
                        }
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.rx = None,
            }
            return;
        }
        let have = h
            .preview
            .as_ref()
            .filter(|p| (p.path.clone(), p.mtime) == wanted);
        match have {
            // The window changed width: the same text, laid out again.
            Some(p) if p.width != width => {
                let lines = p
                    .text
                    .as_deref()
                    .map(|t| home::preview_lines(t, width))
                    .unwrap_or_default();
                if let Some(p) = app.home_mut().and_then(|h| h.preview.as_mut()) {
                    p.lines = lines;
                    p.width = width;
                }
            }
            Some(_) => {}
            None => {
                let (tx, rx) = std::sync::mpsc::channel();
                let (path, mtime) = wanted;
                std::thread::spawn(move || {
                    let text = scan::head_of(&path);
                    let _ = tx.send(home::Preview {
                        path,
                        mtime,
                        text,
                        lines: Vec::new(),
                        width: 0,
                    });
                });
                self.rx = Some(rx);
            }
        }
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
            say!("carrel: {e}");
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
/// The folder a bare `carrel` lists, and a note if the saved one is gone.
///
fn home_root(explicit: Option<&Path>) -> (PathBuf, Option<String>) {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    match explicit {
        // Made absolute here and nowhere else, so `Home::root` is absolute
        // for the rest of the program. The path row shows it as segments you
        // can click and `↑` walks up it, and neither means anything for
        // `docs/today` — `Path::parent` of a one-segment relative path is
        // `Some("")`, which is not a directory anyone can go to.
        Some(p) if p.is_relative() => (cwd.join(p), None),
        Some(p) => (p.to_path_buf(), None),
        None => match config::load_root() {
            Some(saved) if saved.is_dir() => (saved, None),
            Some(saved) => (
                cwd.clone(),
                Some(format!("saved directory is gone: {}", saved.display())),
            ),
            None => (cwd.clone(), None),
        },
    }
}

/// `--latest`: the document written most recently. An agent that has just
/// finished a plan has made it the newest file in the folder, and this is
/// the way to read it without knowing what it was called.
fn open_latest(explicit: Option<&Path>) -> ExitCode {
    if let Some(dir) = explicit
        && !dir.is_dir()
    {
        say!("carrel: {} is not a folder", dir.display());
        return ExitCode::FAILURE;
    }
    // Here, or the folder named — never the folder saved for the file
    // list. That one is "my library", chosen once and possibly long ago;
    // this is "what was just written where I am standing", and answering it
    // from another tree handed back a document from 2020.
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let root = explicit.map_or_else(|| cwd.clone(), |p| cwd.join(p));
    let (entries, _) = scan::walk_blocking(&root);
    let Some(newest) = carrel::cli::latest(&entries) else {
        say!("carrel: no markdown documents under {}", root.display());
        return ExitCode::FAILURE;
    };
    // As a path, never through a string: a name that is not UTF-8 is still
    // a file, and respelling it lossily named one that is not there.
    open_at(newest, None, None)
}

fn open_home(explicit: Option<&Path>) -> ExitCode {
    use std::fmt::Write as _;
    let (root, note) = home_root(explicit);

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
            say!("carrel: {e}");
            ExitCode::FAILURE
        }
    }
}

fn open(arg: &str, pattern: Option<&str>) -> ExitCode {
    // `PLAN.md:42` and `README.md#install` name a place in a file; a path
    // that exists as written is never split.
    let (path, start) = carrel::cli::split_target(arg);
    open_at(&path, start.as_ref(), pattern)
}

fn open_at(path: &Path, start: Option<&carrel::cli::Start>, pattern: Option<&str>) -> ExitCode {
    // Before the read, so a write that lands during it is seen as a change
    // rather than missed.
    let loaded = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let src = match carrel::app::read_document(path) {
        Ok(s) => s,
        Err(e) => {
            say!("{}", carrel::app::explain_open_error(path, &e));
            // A name one slip away from a file that is there. Said, never
            // acted on: opening a different file than the one named would
            // be carrel deciding what you meant.
            if e.kind() == std::io::ErrorKind::NotFound
                && let Some(near) = carrel::cli::suggest_file(path)
            {
                say!("carrel: did you mean {}?", near.display());
            }
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

    match run(path, &src, start, loaded) {
        Ok(None) => ExitCode::SUCCESS,
        // The `⌂`: the reader is closed and the terminal restored, so the
        // home screen starts exactly as `carrel <DIR>` would. It is an
        // explicit root, so browsing here never overwrites the saved one.
        Ok(Some(root)) => open_home(Some(&root)),
        Err(e) => {
            say!("carrel: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Raw mode and the alternate screen, with a panic hook that hands them back.
///
/// `ratatui::init()` does the same, but both its hook and its `restore()`
/// report a failed restore with `eprintln!` — and a restore fails exactly when
/// the terminal is gone, which is when stderr is gone too. That `eprintln!`
/// panics, the hook runs again and panics again, and the process aborts: every
/// window closed with carrel open dumped core and raised a "carrel crashed"
/// notification. Here, and in `TerminalGuard`, a failed restore is silent —
/// there is no one left to tell.
fn init_terminal() -> std::io::Result<ratatui::DefaultTerminal> {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = ratatui::try_restore();
        prev(info);
    }));
    ratatui::crossterm::terminal::enable_raw_mode()?;
    ratatui::crossterm::execute!(
        std::io::stdout(),
        ratatui::crossterm::terminal::EnterAlternateScreen
    )?;
    ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(std::io::stdout()))
}

/// Whether the terminal this reader paints into has been hung up.
///
/// A hung-up tty fails every ioctl, so `isatty` on stdout — the one stream a
/// reader always paints to, piped or not — is the cheap, exact test.
fn terminal_gone() -> bool {
    !std::io::stdout().is_terminal()
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
                DisableBracketedPaste,
                Show
            );
            prev(info);
        }));
        Self::watch_for_signals();
        Self::watch_for_hangup();
        Self
    }

    /// End the reader when its terminal goes away and no signal says so.
    ///
    /// Closing a window usually delivers SIGHUP (see `watch_for_signals`),
    /// but not always: a shell in front of the reader that ignores the
    /// hang-up keeps it from ever arriving. The terminal then reads as
    /// permanently ready with nothing in it, and crossterm 0.29's
    /// `try_read` retries a read that returns end-of-file or `EIO` forever,
    /// inside `event::poll` — so no check in the event loop can run again,
    /// and the reader was left orphaned at ~90% CPU with nowhere to draw.
    /// A thread outside that loop is the only place the hang-up can be seen.
    fn watch_for_hangup() {
        std::thread::spawn(|| {
            loop {
                std::thread::sleep(Duration::from_millis(500));
                if terminal_gone() {
                    let _ = ratatui::try_restore();
                    std::process::exit(128 + signal_hook::consts::SIGHUP);
                }
            }
        });
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
                    DisableBracketedPaste,
                    Show
                );
                let _ = ratatui::try_restore();
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
        let _ = ratatui::crossterm::execute!(
            std::io::stdout(),
            DisableMouseCapture,
            DisableBracketedPaste,
            Show
        );
        let _ = ratatui::try_restore();
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
    initialized: bool,
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
            initialized: false,
        }
    }

    /// What this terminal will actually draw images WITH, in words a reader
    /// can act on.
    ///
    /// Nearly every terminal lands on half-blocks: `from_fontsize` sniffs
    /// tmux and iTerm2 environment variables and nothing else, because the
    /// stdio probe for kitty and sixel blocks on stdin and once left the
    /// binary hanging (see the gotcha). Carrel said nothing about this while
    /// the README promised "kitty protocol first", so a reader seeing a
    /// coloured mosaic had no way to know whether that was carrel, their
    /// terminal, or the file.
    fn kind(&self) -> &'static str {
        use ratatui_image::picker::ProtocolType as P;
        match self.picker.protocol_type() {
            P::Kitty => "kitty graphics",
            P::Sixel => "sixel graphics",
            P::Iterm2 => "iTerm2 graphics",
            P::Halfblocks => "blocks of colour — no graphics protocol selected",
        }
    }

    /// (Re)start decoding when the open document changed.
    fn sync(&mut self, app: &mut App) {
        if app.is_home() || (self.initialized && app.file == self.for_file) {
            return;
        }
        self.initialized = true;
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
                Ok(ImageMsg::Failed(block, e)) => {
                    app.image_errors.insert(block, e.clone());
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

    /// `loaded` is the file's mtime when the document on screen was read.
    /// It is the baseline for a file this has not looked at before: taking
    /// the first LOOK as the baseline instead meant a file rewritten in the
    /// second after it was opened — an agent still writing it — was never
    /// noticed, and the reader sat on the old text with nothing to say so.
    fn poll(
        &mut self,
        file: Option<&Path>,
        loaded: Option<std::time::SystemTime>,
    ) -> Option<Reload> {
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
            Some(prev) if prev.0 == snap.0 => *prev != snap || came_back,
            // A path not seen before — the first look, or a navigation. Not
            // an edit in itself; one only if the file has been written since
            // it was read.
            _ => loaded.is_some_and(|at| at != snap.1),
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
    /// The button went down in the text and began a selection, so its
    /// release is that selection's. A release is otherwise the tail of a
    /// click that already acted — on a link, a chip, a footnote mark — and
    /// means nothing, however many words are still selected from earlier.
    selecting: bool,
    clicks: Clicks,
    wheel: Wheel,
}

impl Pointer {
    /// The window changed size: every coordinate remembered here belongs to
    /// a frame that no longer exists. Whether a selection is in flight is
    /// not a coordinate — the button is still down, and its release in the
    /// new frame still ends that selection.
    fn forget_geometry(&mut self) {
        *self = Self {
            selecting: self.selecting,
            ..Self::default()
        };
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

/// The built-in first document.
///
/// Carrel is a reader, so the honest way to teach it is to hand the reader
/// something to read — a page that describes each thing on the page it is
/// describing it on. It is offered on a first run and from the empty-folder
/// dead end, and `carrel --tutorial` opens it whenever anyone wants it.
/// Open the built-in document. It has no path, so nothing is persisted
/// against it and the reloader stays inert — the same shape as a pipe.
fn run_welcome() -> std::io::Result<Option<PathBuf>> {
    let doc = carrel_core::Document::parse(carrel::app::WELCOME);
    let images = Images::detect();
    let theme_note = startup_theme();
    let terminal = init_terminal()?;
    let _guard = TerminalGuard::engage_mouse();
    let size = terminal.size()?;
    let mut app = App::new("welcome".into(), doc, size.width, size.height);
    app.image_kind = Some(images.kind());
    app.note = theme_note;
    app.state_dir = carrel::state::state_dir();
    carrel::annotation_state::load(&mut app);
    apply_config(&mut app);
    // The constructor laid the page out with the defaults; the reader's own
    // text width, heading bar and hyphenation only apply from here.
    app.on_resize(app.cols, app.rows);
    app.piped = Some(carrel::app::WELCOME.to_string());
    run_loop(terminal, app, images, None)
}

fn run(
    path: &Path,
    src: &str,
    start: Option<&carrel::cli::Start>,
    loaded: Option<std::time::SystemTime>,
) -> std::io::Result<Option<PathBuf>> {
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

    let terminal = init_terminal()?;
    let _guard = TerminalGuard::engage_mouse();

    let size = terminal.size()?;
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let mut app = App::new(name, doc, size.width, size.height);
    app.mtime = loaded;
    // What images will actually look like here, for the info card.
    app.image_kind = Some(images.kind());
    app.diff_ok = diff_ok;
    app.diff_forced = diff_forced();
    app.file = Some(path.to_path_buf());
    app.note = theme_note;
    app.state_dir = carrel::state::state_dir();
    carrel::annotation_state::load(&mut app);
    // The one-time footer invitation. No state directory at all means we
    // cannot tell, and a line that might never go away is worse than one
    // that never appears.
    app.first_run = app
        .state_dir
        .as_deref()
        .is_some_and(carrel::state::is_first_run_in);
    // Rooted at the document's own directory: links inside the project follow
    // silently, links out of it ask first. See `App::escapes_library`.
    // `Path::new("doc.md").parent()` is `Some("")`, which canonicalizes to an
    // error and would silently disable containment — the working directory is
    // what an empty parent means.
    app.library_root = Some(match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    });
    // The file and its folder are known now, which is what resolving a
    // `[[note]]` and finding a dead link both needed.
    app.index_links();
    apply_config(&mut app);
    app.on_resize(app.cols, app.rows);
    // A direct open builds the App by hand rather than via open_path, so the
    // saved reading position needs restoring explicitly. Its note outranks
    // the theme note — the theme is only news when it failed to load.
    app.restore_position();
    // A place asked for outranks a place remembered.
    if let Some(start) = start {
        app.note = None;
        app.start_at(start);
    }
    run_loop(terminal, app, images, None)
}

/// The reader's event loop, shared by the file entry (`run`) and the piped
/// entry (`run_stdin`). `stream` is the stdin channel when the document is
/// arriving live; `None` for a file.
/// Returns the directory to open the home screen on, when the reader was
/// asked for the file list and there is no home screen behind it to restore
/// — `Action::GoHome` from a document opened directly, or piped in.
fn run_loop(
    mut terminal: ratatui::DefaultTerminal,
    mut app: App,
    mut images: Images,
    mut stream: Option<&Receiver<String>>,
) -> std::io::Result<Option<PathBuf>> {
    let mut keys = Keys::new();

    let mut resize = Resize::default();
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
    // The tags page, asked for from inside a document: see `TagScan`.
    let mut tag_scan = TagScan::new();
    let mut siblings = Siblings::new();

    loop {
        drive_backlinks(&mut app, &mut backlinks, &mut backlinks_for);
        tag_scan.tick(&mut app);
        siblings.tick(&mut app);
        images.sync(&mut app);
        images.drain(&mut app);
        diagrams.sync(&app);
        diagrams.drain(&mut app);
        if repaint && resize.pending.is_none() {
            resize.clear_if_needed(&mut terminal)?;
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
        let timeout = resize
            .deadline
            .map_or(Duration::from_millis(100), |d| {
                d.saturating_duration_since(Instant::now())
            })
            .min(auto_timeout);

        if event::poll(timeout)? {
            // A read error means the input stream is gone. Exit rather than
            // spin: `poll` keeps reporting ready once stdin is at EOF.
            let Ok(ev) = event::read() else { break };
            if resize.prepare_input(&ev, &mut app, &mut painted.targets, &mut ptr)? {
                continue;
            }
            match ev {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    if let Some(action) = key_action(&mut keys, &app, k)
                        && update(&mut app, action) == Outcome::Quit
                    {
                        break;
                    }
                }
                Event::Paste(text) if app.notes.draft.is_some() => {
                    carrel::annotation_state::paste(&mut app, &text);
                }
                Event::Mouse(m) => match mouse_action(m, &app, &painted.targets, &mut ptr) {
                    Some(action) => {
                        if update(&mut app, action) == Outcome::Quit {
                            break;
                        }
                    }
                    None => repaint = !coalescing_motion(m),
                },
                Event::Resize(..) => resize.schedule()?,
                _ => {}
            }
        }

        drain_outboxes(&mut app)?;
        // Asked for the file list with nothing to go back to: leave the
        // loop and let `main` build one. Nothing here can start a
        // filesystem walk — that is `run_home`'s machinery, and duplicating
        // it inside the reader's loop is how the two would drift.
        if app.goto_home.is_some() {
            break;
        }

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

        if resize.apply(&mut app) {
            painted.targets.clear();
            ptr.forget_geometry();
        }
    }
    Ok(app.goto_home.take())
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
    // The text goes back BEFORE asking whether the pipe is on screen:
    // `showing_pipe` reads `piped`, and asking while it was taken out said
    // no every time, so nothing piped was ever shown (2026-09-30 to
    // 2026-10-08). Only while the pipe is what is on screen: a chunk that
    // lands behind a file, the tags page or search results waits in
    // `piped`, and coming back parses all of it.
    app.piped = Some(buf);
    if grew && app.showing_pipe() {
        let text = app.piped.clone().unwrap_or_default();
        app.reload_from(&text);
        images.for_file = None;
        images.initialized = false;
        diagrams.for_file = None;
        // Following is applied HERE, not in `update()`: the stream is drained
        // by the event loop, and no action fires when a chunk lands.
        if app.following {
            let h = app.text_h();
            app.view.scroll_to(&app.doc, &app.layout, u32::MAX, h);
        }
    }
    if done {
        app.streaming = false;
        // Nothing left to follow; the lamp says `reading` again.
        app.following = false;
        if app.showing_pipe() {
            app.path = "(stdin)".into();
        }
        *stream = None;
    }
}

/// The piped-document reader: the TUI opens immediately and content streams
/// in. Keys arrive through `/dev/tty` — crossterm's own fallback when stdin
/// is not a terminal — so this fails cleanly where no tty exists.
fn run_stdin(rx: &Receiver<String>) -> std::io::Result<Option<PathBuf>> {
    let theme_note = startup_theme();
    let images = Images::detect();
    let terminal = init_terminal()?;
    let _guard = TerminalGuard::engage_mouse();

    let size = terminal.size()?;
    let mut app = App::new(
        "(stdin — streaming…)".into(),
        Document::parse(""),
        size.width,
        size.height,
    );
    // What images will actually look like here, for the info card.
    app.image_kind = Some(images.kind());
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
    match reloader.poll(app.file.as_deref(), app.mtime) {
        Some(Reload::Changed) => match app.reload() {
            Ok(()) => {
                images.for_file = None;
                images.initialized = false;
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

/// How often the folder is looked at while a document is open.
const SIBLINGS_EVERY: Duration = Duration::from_secs(4);

/// Watches the folder a document is being read from, and says when another
/// document in it is written.
///
/// An agent that is asked for a plan and a changelog writes two files, and
/// the reader is looking at one of them. This is what tells them the other
/// exists, without a second terminal and without polling a file list by
/// hand. It is a walk on a thread — the home screen's own, which honors
/// `.gitignore` — one at a time, so a slow disk makes it late rather than
/// making it pile up.
///
/// It only ever SETS `App::sibling`. The reader puts the news away (`Esc`)
/// or acts on it (the chip); either empties it, and what counts as new is
/// then measured from that moment.
struct Siblings {
    rx: Option<Receiver<(Vec<scan::Entry>, Duration)>>,
    last: Instant,
    /// How long to leave between walks: [`SIBLINGS_EVERY`], or longer where
    /// a walk is slow. The folder is whatever the document happens to live
    /// in — `carrel ~/notes.md` makes it the whole home directory — and a
    /// reader sitting on one page must not cost a steady slice of a core.
    /// Each walk buys a rest twenty-five times its own length.
    every: Duration,
    /// The document being watched from, canonical, and the time newer
    /// writes are measured against.
    watching: Option<(PathBuf, PathBuf, std::time::SystemTime)>,
    reported: bool,
    /// The document last offered on the status row.
    offered: Option<PathBuf>,
}

impl Siblings {
    fn new() -> Self {
        Self {
            rx: None,
            last: Instant::now(),
            every: SIBLINGS_EVERY,
            watching: None,
            reported: false,
            offered: None,
        }
    }

    fn tick(&mut self, app: &mut App) {
        let file = match app.file.clone() {
            Some(f) if !app.is_home() && !app.streaming => f,
            _ => {
                self.rx = None;
                self.watching = None;
                return;
            }
        };
        let now = std::time::SystemTime::now();
        if self.watching.as_ref().map(|w| &w.0) != Some(&file) {
            // A different document: what is new is new from here — unless
            // it is the one that was just announced. Then the reader took
            // the offer, and the `(+1)` beside it named another document
            // that is still news; starting the clock again forgot it.
            let since = match (&self.watching, &self.offered) {
                (Some(w), Some(offered)) if *offered == file => w.2,
                _ => now,
            };
            let canon = file.canonicalize().unwrap_or_else(|_| file.clone());
            self.watching = Some((file, canon, since));
            self.rx = None;
            self.reported = false;
            app.sibling = None;
        }
        if self.reported && app.sibling.is_none() {
            // Put away, or opened: start counting again.
            self.reported = false;
            if let Some(w) = self.watching.as_mut() {
                w.2 = now;
            }
        }
        let Some((_, canon, since)) = self.watching.clone() else {
            return;
        };
        if let Some(rx) = self.rx.as_ref() {
            match rx.try_recv() {
                Ok((entries, took)) => {
                    self.rx = None;
                    self.every = SIBLINGS_EVERY.max(took * 25);
                    self.last = Instant::now();
                    if let Some(sibling) = carrel::app::newest_sibling(&entries, &canon, since, now)
                    {
                        self.offered = Some(sibling.path.clone());
                        app.sibling = Some(sibling);
                        self.reported = true;
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.rx = None,
            }
            return;
        }
        if self.last.elapsed() < self.every {
            return;
        }
        let Some(root) = app.tags_root() else { return };
        self.last = Instant::now();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let began = Instant::now();
            let entries = scan::walk_blocking(&root).0;
            let _ = tx.send((entries, began.elapsed()));
        });
        self.rx = Some(rx);
    }
}

/// The home screen's tag scan: one background thread per request, only
/// current-generation results kept, the finished index handed to the state
/// layer as an action. `Grep`'s shape, without the debounce — a request is a
/// single keystroke, not a query being typed.
struct TagScan {
    rx: Option<carrel::tags::Scan>,
    generation: u64,
    found: Vec<carrel::tags::Tagged>,
}

impl TagScan {
    fn new() -> Self {
        Self {
            rx: None,
            generation: 0,
            found: Vec::new(),
        }
    }

    /// A scan is running: the loop should wake fast enough to finish it.
    fn busy(&self) -> bool {
        self.rx.is_some()
    }

    fn tick(&mut self, app: &mut App) {
        if app.tags != carrel::tags::Request::Wanted {
            // Withdrawn. Dropping the scan stops its workers within a file;
            // bumping the generation discards anything already queued.
            if self.rx.take().is_some() {
                self.generation += 1;
            }
            self.found.clear();
            return;
        }
        if self.rx.is_none() {
            // The file list in front, the one behind the reader, or — for a
            // document opened by name, which never had one — a walk of the
            // folder it lives in.
            let (source, titles) = if let Some(h) = app.home() {
                // The walk is still finding files: wait for the list to be
                // whole rather than report tags for half a folder as all.
                if h.scanning {
                    return;
                }
                (
                    carrel::tags::Source::Entries(h.entries.clone()),
                    h.show_titles,
                )
            } else if let Some(h) = app.home_stash.as_deref() {
                (
                    carrel::tags::Source::Entries(h.entries.clone()),
                    h.show_titles,
                )
            } else if let Some(root) = app.tags_root() {
                (carrel::tags::Source::Walk(root), app.titles)
            } else {
                app.tags = carrel::tags::Request::Idle;
                return;
            };
            self.generation += 1;
            self.found.clear();
            self.rx = Some(carrel::tags::spawn(source, titles, self.generation));
        }
        let Some(rx) = self.rx.as_ref() else { return };
        let mut done = None;
        loop {
            match rx.try_recv() {
                Ok(carrel::tags::Msg::Found(t, generation)) => {
                    if generation == self.generation {
                        self.found.push(t);
                    }
                }
                Ok(carrel::tags::Msg::Done {
                    read,
                    capped,
                    generation,
                }) => {
                    if generation == self.generation {
                        done = Some((read, capped));
                    }
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => return,
                // The thread died without finishing. Stop waiting on a
                // request nobody is serving.
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            }
        }
        self.rx = None;
        let Some((read, capped)) = done else {
            // Not "no tags": carrel does not know, and says that instead.
            self.found.clear();
            app.tags = carrel::tags::Request::Idle;
            app.set_note("could not finish reading tags — press # to try again".into());
            return;
        };
        app.tags = carrel::tags::Request::Ready(carrel::tags::Index {
            tagged: std::mem::take(&mut self.found),
            read,
            capped,
        });
        update(app, carrel::action::Action::HomeOpenTags);
    }
}

/// The file list's `App`, with everything the binary injects into it: where
/// state lives, the reader's preferences, and what the state file remembers.
fn home_app(root: &Path, note: Option<String>, size: (u16, u16), images: &Images) -> App {
    // The cache paints before any syscall; the walk refines it.
    let cached = scan::load_cache(root);
    let mut app = App::new_home(root.to_path_buf(), cached, size.0, size.1);
    // What images will actually look like here, for the info card.
    app.image_kind = Some(images.kind());
    app.state_dir = carrel::state::state_dir();
    carrel::annotation_state::load(&mut app);
    // The one-time footer invitation. No state directory at all means we
    // cannot tell, and a line that might never go away is worse than one
    // that never appears.
    app.first_run = app
        .state_dir
        .as_deref()
        .is_some_and(carrel::state::is_first_run_in);
    app.library_root = Some(root.to_path_buf());
    apply_config(&mut app);
    load_resume(&mut app);
    set_home_prefs(&mut app);
    load_reading(&mut app);
    app.on_resize(app.cols, app.rows);
    if let Some(n) = note
        && let Some(h) = app.home_mut()
    {
        h.note = Some(n);
    }
    app
}

fn run_home(root: PathBuf, note: Option<String>) -> std::io::Result<()> {
    let theme_note = startup_theme();
    let note = note.or(theme_note);
    let mut images = Images::detect();

    let mut terminal = init_terminal()?;
    let _guard = TerminalGuard::engage_mouse();

    let size = terminal.size()?;
    let mut app = home_app(&root, note, (size.width, size.height), &images);
    let mut keys = Keys::new();
    let mut ptr = Pointer::default();
    let mut reloader = Reloader::new();
    let mut desktop = Desktop::new();
    let mut diagrams = Diagrams::new();
    let mut scan = Scan::start(&root);
    let mut scan_root = root;

    // Content search (wave E): see `Grep`.
    let mut grep = Grep::new();
    // The tags document: see `TagScan`.
    let mut tag_scan = TagScan::new();
    let mut preview = PreviewLoad { rx: None };
    let mut siblings = Siblings::new();
    // Coming back from a document, the list's reading columns are a
    // document out of date.
    let mut was_home = true;

    let mut resize = Resize::default();
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
        if repaint && resize.pending.is_none() {
            resize.clear_if_needed(&mut terminal)?;
            paint(&mut terminal, &app, &mut painted, &mut images.protocols)?;
        }
        repaint = true; // only a motion burst turns this off, and only briefly

        drain_scan(&mut app, &mut scan);

        grep.tick(&mut app);
        tag_scan.tick(&mut app);
        if app.is_home() && !was_home {
            load_reading(&mut app);
        }
        was_home = app.is_home();
        preview.tick(&mut app);
        siblings.tick(&mut app);

        maybe_rescan(&mut app, &mut scan, &scan_root, &grep);

        // While a scan or search is live, wake often enough to stream.
        let busy = scan.busy() || grep.busy() || tag_scan.busy() || preview.busy();
        let idle = if busy { 16 } else { 100 };
        let timeout = resize.deadline.map_or(Duration::from_millis(idle), |d| {
            d.saturating_duration_since(Instant::now())
        });

        if event::poll(timeout)? {
            let Ok(ev) = event::read() else { break };
            if resize.prepare_input(&ev, &mut app, &mut painted.targets, &mut ptr)? {
                continue;
            }
            match ev {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    if let Some(a) = key_action(&mut keys, &app, k)
                        && update(&mut app, a) == Outcome::Quit
                    {
                        break;
                    }
                }
                Event::Paste(text) if app.notes.draft.is_some() => {
                    carrel::annotation_state::paste(&mut app, &text);
                }
                // A path pasted or dropped onto the file list opens it.
                Event::Paste(text) if app.is_home() => {
                    carrel::app::paste_on_home(&mut app, &text);
                }
                Event::Mouse(m) => match home_mouse_action(m, &app, &painted.targets, &mut ptr) {
                    Some(a) => {
                        if update(&mut app, a) == Outcome::Quit {
                            break;
                        }
                    }
                    None => repaint = !coalescing_motion(m),
                },
                Event::Resize(..) => resize.schedule()?,
                _ => {}
            }
        }

        drain_outboxes(&mut app)?;

        // And the same live reload (a no-op while the home screen is up —
        // there is no open file to watch), and the same desktop palette.
        poll_reload(&mut reloader, &mut app, &mut images, &mut diagrams);
        desktop.poll();

        if resize.apply(&mut app) {
            painted.targets.clear();
            ptr.forget_geometry();
        }

        // The picker changed root: restart the walk against the new one.
        if let Some(h) = app.home()
            && h.root != scan_root
        {
            scan_root.clone_from(&h.root);
            scan.restart(&scan_root);
            load_reading(&mut app);
        }
    }
    Ok(())
}

/// Shared resize debounce and input barrier for both event loops.
#[derive(Default)]
struct Resize {
    pending: Option<(u16, u16)>,
    deadline: Option<Instant>,
    dirty: bool,
}

impl Resize {
    fn schedule(&mut self) -> std::io::Result<()> {
        self.dirty = true;
        self.pending = Some(ratatui::crossterm::terminal::size()?);
        self.deadline = Some(Instant::now() + DEBOUNCE);
        Ok(())
    }

    /// A shrink/grow burst can lose terminal cells even when its final size
    /// equals the last drawn size. Invalidate ratatui's retained buffer too.
    fn clear_if_needed(&mut self, terminal: &mut ratatui::DefaultTerminal) -> std::io::Result<()> {
        if self.dirty {
            // Fullscreen resize clears without Terminal::clear's cursor query.
            terminal.resize(terminal.size()?.into())?;
            self.dirty = false;
        }
        Ok(())
    }

    /// Avoid rebuilding layout for every cell of a continuous window drag.
    fn apply(&mut self, app: &mut App) -> bool {
        if self.deadline.is_some_and(|d| Instant::now() >= d)
            && let Some((w, h)) = self.pending.take()
        {
            self.deadline = None;
            app.on_resize(w, h);
            return true;
        }
        false
    }

    /// Input cannot use old modal visibility or hit geometry during debounce.
    /// Read the PTY size: input can arrive before its queued SIGWINCH event.
    fn prepare_input(
        &mut self,
        event: &Event,
        app: &mut App,
        targets: &mut Targets,
        pointer: &mut Pointer,
    ) -> std::io::Result<bool> {
        if matches!(event, Event::Resize(..)) {
            return Ok(false);
        }
        let size = ratatui::crossterm::terminal::size()?;
        self.pending = None;
        self.deadline = None;
        let changed = size != (app.cols, app.rows);
        if !changed && !self.dirty {
            return Ok(false);
        }
        self.dirty = true;
        if changed {
            app.on_resize(size.0, size.1);
        }
        targets.clear();
        pointer.forget_geometry();
        // Queued mouse coordinates refer to a frame that no longer exists.
        Ok(matches!(event, Event::Mouse(_)))
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
    if app.lightbox.is_some() {
        keys.reset();
        return Keys::map_lightbox(k);
    }
    if app.notes.draft.is_some() || app.notes.pane.is_some() {
        keys.reset();
        return Keys::map_notes(k, app.notes.draft.is_some());
    }
    // Ctrl-C with something selected COPIES it, and quits only when there is
    // nothing to copy. Raw mode clears ISIG, so this key is carrel's to
    // define and no SIGINT is ever generated — which means the reflex a
    // beginner brings from every other program (select, Ctrl-C) was
    // destroying the program instead of filling the clipboard. `Q` and a
    // Ctrl-C on an empty selection still quit, so the escape hatch is intact.
    if k.code == KeyCode::Char('c')
        && k.modifiers.contains(KeyModifiers::CONTROL)
        && app.selection.is_some()
    {
        return Some(carrel::action::Action::SelectRelease);
    }
    // A peek offers exactly one thing of its own; every other key closes it
    // and then does what it always does.
    if app.peek.is_some() && k.code == KeyCode::Enter {
        return Some(carrel::action::Action::PeekGo);
    }
    // The pane that owns the screen owns the keys — the SAME answer the
    // painter uses to decide what is on top (`App::pane`), so the two can
    // no longer disagree. A menu can only be up with no pane under it.
    match app.pane() {
        Some(Pane::Menu) => Keys::map_menu(k),
        // The sheet types: every printable key narrows it, so it needs its
        // own map the way the outline picker has — the reader's map would
        // scroll the document underneath instead.
        Some(Pane::Help) => Keys::map_help(k),
        // The panes with no text field: a key they do not bind, which would
        // open ANOTHER pane on the page underneath, opens it in their place
        // (`update` closes this one first) — the `≡` rule, by key. Before,
        // `h` over the info card did nothing at all, with nothing to say
        // why. The panes that type (help, outline, the prompts, a note)
        // are not here: their letters are text.
        Some(Pane::Backlinks) => Keys::map_backlinks(k).or_else(|| pane_key(app, k)),
        Some(Pane::Forward) => Keys::map_forward(k).or_else(|| pane_key(app, k)),
        Some(Pane::Settings) => Keys::map_settings(k).or_else(|| pane_key(app, k)),
        Some(Pane::Marks) => Keys::map_marks(k).or_else(|| pane_key(app, k)),
        Some(Pane::Outline) => Keys::map_outline(k),
        Some(Pane::Info) => Keys::map_info(k).or_else(|| pane_key(app, k)),
        // Answered above, before the selection and peek checks.
        Some(Pane::Lightbox | Pane::Notes) => None,
        // The search prompt and the folder dialog are typed into through
        // their own screen's map, which knows their modes.
        Some(Pane::Search | Pane::Picker) | None => {
            if app.is_home() {
                let mode = app.home().map_or(home::HomeMode::Filter, |h| h.mode);
                keys.map_home(k, mode)
            } else {
                keys.map(k, app.searching())
            }
        }
    }
}

/// `k` as the screen underneath would read it, if what it does there is open
/// a pane — and nothing else.
///
/// Only the plain pane keys: a key that ACTS on the page (Enter following a
/// link, `a` starting a note at the cursor) stays dead over a pane, because
/// what it would act on is hidden. A fresh `Keys`, so no count or pending
/// prefix leaks between the pane and the page.
fn pane_key(app: &App, k: KeyEvent) -> Option<carrel::action::Action> {
    let mut keys = Keys::new();
    let a = match app.home() {
        Some(h) => keys.map_home(k, h.mode),
        None => keys.map(k, false),
    }?;
    carrel::app::Pane::switches(a).then_some(a)
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
    // The palette is presentation state and never enters `App`, so `update`
    // records the REQUEST and the loop performs it. It used to be a branch
    // in each loop taken before `update` ever ran, which meant a theme
    // change could only ever come from a key — a menu row that cycles
    // themes would have been silently inert.
    if std::mem::take(&mut app.theme_cycle) {
        cycle_theme_now(app);
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

/// The pointer moved: light what is under it, but only when that CHANGES.
///
/// Reporting every motion would defeat the coalescing that stops a slow
/// mouse drag repainting the whole screen once per cell crossed — and the
/// answer is almost always the same target as the cell before.
fn hover_action(m: MouseEvent, app: &App, targets: &Targets) -> Option<carrel::action::Action> {
    let now = targets.hoverable(m.column, m.row);
    match app.hover {
        // Still inside the same target — including "still over nothing".
        Some((c, r)) if targets.hoverable(c, r) == now => None,
        _ => Some(carrel::action::Action::Hover((m.column, m.row))),
    }
}

/// Should a right-click here open a menu, and which one?
///
/// A pane owns the pointer while it is up exactly as it owns the keyboard,
/// so a right-click into one is inert rather than stacking a second modal
/// thing on screen with no rule for which answers — and one beside it never
/// gets here: it lands on the scrim and only closes the pane (`beside_pane`). Otherwise: the doc byte
/// under the pointer picks the context menu, and its absence — the chrome,
/// the margin, the blank below the last line, the whole home screen — picks
/// the global one.
fn open_menu_at(m: MouseEvent, app: &App) -> Option<carrel::action::Action> {
    // `App::pane`, not a list of its own: the list this used to keep had
    // no settings pane, no info card and no folder dialog, so a right-click
    // opened a menu over all three. The menu itself is the exception: a
    // menu's own opener REPLACES it (`update` lets a pane's own opener
    // through), which is how `menu_mouse` moves it to a right-click outside.
    if app.pane().is_some_and(|p| p != Pane::Menu) {
        return None;
    }
    let byte = if app.is_home() {
        None
    } else {
        doc_span_at(app, m.column, m.row).map(|(start, _)| start)
    };
    Some(carrel::action::Action::MenuOpen {
        at: (m.column, m.row),
        byte,
    })
}

/// Pointer events while a menu is open. Nothing falls through to the
/// document: the menu is modal, and `covered_at(.., Z_MENU)` is the painter's
/// own answer to "was that inside it".
fn menu_mouse(m: MouseEvent, app: &App, targets: &Targets) -> Option<carrel::action::Action> {
    use carrel::action::Action;
    let row_at = |col, row| {
        targets.hit(col, row).and_then(|h| match h.action {
            Action::MenuPick(i) => Some(i),
            _ => None,
        })
    };
    let inside = targets.covered_at(m.column, m.row, carrel::render::Z_MENU);
    match m.kind {
        // Hover moves the selection, so the keyboard cursor follows the
        // pointer — herdr's model. Reported only when it CHANGES something:
        // an unchanged hover on every cell of a drag across the box would
        // repaint the screen once per column.
        MouseEventKind::Moved => {
            // Border controls scroll from the current cursor. Clearing it on
            // hover would make each click restart at the first/last item.
            if targets
                .hit(m.column, m.row)
                .is_some_and(|hit| matches!(hit.action, Action::MenuMove(_) | Action::MenuClose))
            {
                return None;
            }
            let want = row_at(m.column, m.row);
            let now = app.menu.as_ref().and_then(|menu| menu.selected);
            if want.map(|i| i as usize) == now {
                None
            } else {
                // Out of range is the deliberate "over nothing" signal.
                Some(Action::MenuHover(want.unwrap_or(u32::MAX)))
            }
        }
        MouseEventKind::Down(MouseButton::Left) => {
            if !inside {
                // A press outside dismisses without acting — every menu
                // anyone has ever used does this, and a reader who opened
                // one by accident must be able to leave without learning a
                // key.
                return Some(Action::MenuClose);
            }
            targets
                .hit(m.column, m.row)
                .and_then(|hit| match hit.action {
                    a @ (Action::MenuPick(_) | Action::MenuMove(_) | Action::MenuClose) => Some(a),
                    _ => None,
                })
        }
        // Right-clicking elsewhere moves the menu there rather than making
        // you close it first.
        MouseEventKind::Down(MouseButton::Right) if !inside => open_menu_at(m, app),
        MouseEventKind::ScrollDown => Some(Action::MenuMove(1)),
        MouseEventKind::ScrollUp => Some(Action::MenuMove(-1)),
        _ => None,
    }
}

/// The pointer while a pane is up, outside the pane's own surface.
///
/// A press of ANY button beside the pane lands on the scrim the painter
/// laid under it (`render::paint_scrim`) and is used up closing the pane —
/// the menu's rule, for every pane: no link follows, no chip fires, no
/// selection starts and no context menu opens. The pane's own rows and
/// footer chips, and the `≡`, sit above the scrim and answer as usual.
///
/// A drag with a pane up is nothing at all: the press that began it was
/// either the pane's or used up closing it, and the text underneath must
/// not start selecting. `Some(Absorb)` is "answered: do nothing", which
/// the callers drop exactly as they drop a pane's own `Absorb`.
fn beside_pane(m: MouseEvent, app: &App, targets: &Targets) -> Option<carrel::action::Action> {
    use carrel::action::Action;
    app.pane()?;
    match m.kind {
        MouseEventKind::Down(_)
            if targets
                .hit(m.column, m.row)
                .is_some_and(|hit| hit.action == Action::PaneClose) =>
        {
            Some(Action::PaneClose)
        }
        MouseEventKind::Drag(_) => Some(Action::Absorb),
        _ => None,
    }
}

/// Pane wheels use the same arrow dispatcher as the keyboard, without
/// consuming the reader's pending count. Document scrolling keeps acceleration.
fn pane_wheel(m: MouseEvent, app: &App) -> Option<carrel::action::Action> {
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};
    let code = match m.kind {
        MouseEventKind::ScrollDown => KeyCode::Down,
        MouseEventKind::ScrollUp => KeyCode::Up,
        _ => return None,
    };
    if matches!(
        app.pane(),
        Some(
            Pane::Help
                | Pane::Backlinks
                | Pane::Forward
                | Pane::Settings
                | Pane::Marks
                | Pane::Outline
                | Pane::Info
        )
    ) {
        key_action(
            &mut Keys::new(),
            app,
            KeyEvent::new(code, KeyModifiers::NONE),
        )
    } else {
        None
    }
}

/// The pointer over an open peek, beyond the left click its painted
/// targets answer. The wheel reads on in the footnote; anywhere else it
/// scrolls the page — and, being something the reader did, closes the
/// peek. The box is a target only where it was painted.
///
/// A right-click or a drag inside the box is the box's, and the box has no
/// menu and nothing to select: `Some(Absorb)`, answered and dropped. Mapped
/// through with `doc_span_at`, either acted on the text the box hides. The
/// box does show document bytes, but a menu for one would close the box as
/// it opened (a peek closes on whatever comes next — the one-pane rule) and
/// leave its rows naming text no longer on screen; the box's own links and
/// marks answer a left click. Inert, then, as a right-click into a pane is
/// (`open_menu_at`).
fn peek_pointer(
    m: MouseEvent,
    app: &App,
    targets: &Targets,
    ptr: &mut Pointer,
) -> Option<carrel::action::Action> {
    use carrel::action::Action;
    if app.peek.is_none() || !targets.covered_at(m.column, m.row, carrel::render::Z_MENU) {
        return None;
    }
    match m.kind {
        MouseEventKind::ScrollDown => Some(Action::PeekScroll(ptr.wheel.notch(true))),
        MouseEventKind::ScrollUp => Some(Action::PeekScroll(-ptr.wheel.notch(false))),
        MouseEventKind::Down(MouseButton::Right) | MouseEventKind::Drag(_) => Some(Action::Absorb),
        _ => None,
    }
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

    if matches!(m.kind, MouseEventKind::Down(_)) {
        ptr.selecting = false;
    }
    if app.lightbox.is_some() {
        return if matches!(m.kind, MouseEventKind::Down(MouseButton::Left)) {
            targets.hit(m.column, m.row).map(|hit| hit.action)
        } else {
            None
        };
    }
    if app.notes.draft.is_some() || app.notes.pane.is_some() {
        return match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                targets.hit(m.column, m.row).map(|hit| hit.action)
            }
            MouseEventKind::ScrollDown if app.notes.draft.is_none() => Some(Action::NoteMove(1)),
            MouseEventKind::ScrollUp if app.notes.draft.is_none() => Some(Action::NoteMove(-1)),
            _ => None,
        };
    }
    if app.menu.is_some() {
        return menu_mouse(m, app, targets);
    }
    if let Some(action) = beside_pane(m, app, targets) {
        return (action != carrel::action::Action::Absorb).then_some(action);
    }
    if let Some(action) = pane_wheel(m, app).or_else(|| peek_pointer(m, app, targets, ptr)) {
        return (action != Action::Absorb).then_some(action);
    }
    if matches!(m.kind, MouseEventKind::Down(MouseButton::Right)) {
        return open_menu_at(m, app);
    }
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
                // Track click: go THERE. Paging one screen toward the
                // pointer is the old terminal idiom; every graphical
                // scrollbar a beginner has ever used jumps to the clicked
                // position, and the thumb lands centred under the pointer
                // because that is what `grab_offset = len / 2` means.
                Some(Action::ScrollTo(drag_target(
                    row,
                    len / 2,
                    text_h,
                    total,
                    max_scroll,
                )))
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
            ptr.selecting = true;
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
                std::mem::take(&mut ptr.selecting).then_some(Action::SelectRelease)
            }
        }
        // Decoration only, and last: every button the pointer could be over
        // has already had its chance to mean something.
        MouseEventKind::Moved => hover_action(m, app, targets),
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
    app.hyphenate = c.hyphenate.unwrap_or(true);
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
    // The same two rules as the reader, in the same order — `key_action`'s
    // lesson applied to the pointer: one decision, not a copy per loop.
    if app.lightbox.is_some() {
        return if matches!(m.kind, MouseEventKind::Down(MouseButton::Left)) {
            targets.hit(m.column, m.row).map(|hit| hit.action)
        } else {
            None
        };
    }
    if app.notes.draft.is_some() || app.notes.pane.is_some() {
        return match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                targets.hit(m.column, m.row).map(|hit| hit.action)
            }
            MouseEventKind::ScrollDown if app.notes.draft.is_none() => Some(Action::NoteMove(1)),
            MouseEventKind::ScrollUp if app.notes.draft.is_none() => Some(Action::NoteMove(-1)),
            _ => None,
        };
    }
    if app.menu.is_some() {
        return menu_mouse(m, app, targets);
    }
    if let Some(action) = beside_pane(m, app, targets) {
        return (action != carrel::action::Action::Absorb).then_some(action);
    }
    if let Some(action) = pane_wheel(m, app) {
        return Some(action);
    }
    if matches!(m.kind, MouseEventKind::Down(MouseButton::Right)) {
        return open_menu_at(m, app);
    }
    if matches!(m.kind, MouseEventKind::Down(MouseButton::Left))
        && let Some(hit) = targets.hit(m.column, m.row)
    {
        // The picker's rows keep the file list's press-count idiom even
        // though they are registered targets now (registration is what lets
        // hover light them): one press selects, the second opens.
        if let Action::PickerSelect(i) = hit.action {
            return Some(match ptr.clicks.press(m.column, m.row) {
                1 => Action::PickerSelect(i),
                _ => Action::PickerChoose,
            });
        }
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
                // The rows usually act as targets above, which is also what
                // lights them on hover; this geometry arm is the fallback for
                // a click that arrives with no painted frame behind it.
                if let Some(i) = home.picker_row_at(m.column, m.row, app.cols, app.rows) {
                    return Some(match ptr.clicks.press(m.column, m.row) {
                        1 => Action::PickerSelect(i),
                        _ => Action::PickerChoose,
                    });
                }
                // The dialog's buttons already acted above, and its chrome is
                // absorbed there too. Anything else inside the box is dead
                // air; anything outside it backs out of the dialog.
                return (!home.picker_contains(m.column, m.row, app.cols, app.rows))
                    .then_some(Action::PickerCancel);
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
        MouseEventKind::Moved => hover_action(m, app, targets),
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
        // Bracketed paste belongs to the note editor — pasted newlines must
        // become text, never the Enter command that saves a note — and to
        // the file list, where a pasted or dropped path opens that file
        // instead of being typed at the list as forty commands.
        if app.notes.draft.is_some() || app.is_home() {
            ratatui::crossterm::execute!(std::io::stdout(), EnableBracketedPaste)?;
        } else {
            ratatui::crossterm::execute!(std::io::stdout(), DisableBracketedPaste)?;
        }
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
                "\x1b]8;;{}\x1b\\{}{}\x1b[0m\x1b]8;;\x1b\\",
                l.url,
                sgr(l.style),
                l.text
            )),
        )?;
    }
    out.flush()
}

/// A ratatui `Style` as the SGR sequence that reproduces it.
///
/// Only what a link cell can actually carry: a foreground, a background, and
/// the modifiers the inline styles and hover use. Anything else is dropped
/// rather than approximated — this repaints cells ratatui already painted
/// correctly, so the failure mode of an omission is "looks like the terminal
/// default for one attribute", not a wrong colour.
fn sgr(style: ratatui::style::Style) -> String {
    use ratatui::style::Modifier;
    use std::fmt::Write as _;
    let mut s = String::from("\x1b[0m");
    if let Some(c) = style.fg {
        let _ = write!(s, "\x1b[{}m", colour(c, true));
    }
    if let Some(c) = style.bg {
        let _ = write!(s, "\x1b[{}m", colour(c, false));
    }
    for (m, code) in [
        (Modifier::BOLD, 1),
        (Modifier::DIM, 2),
        (Modifier::ITALIC, 3),
        (Modifier::UNDERLINED, 4),
        (Modifier::REVERSED, 7),
        (Modifier::CROSSED_OUT, 9),
    ] {
        if style.add_modifier.contains(m) {
            let _ = write!(s, "\x1b[{code}m");
        }
    }
    s
}

/// One colour as SGR parameters, foreground or background.
fn colour(c: ratatui::style::Color, fg: bool) -> String {
    use ratatui::style::Color as C;
    // 30-37 / 40-47 for the eight, +60 for the bright pair.
    let base = u8::from(!fg) * 10;
    let named = |n: u8| (30 + base + n).to_string();
    let bright = |n: u8| (90 + base + n).to_string();
    match c {
        C::Rgb(r, g, b) => format!("{};2;{r};{g};{b}", if fg { 38 } else { 48 }),
        C::Indexed(i) => format!("{};5;{i}", if fg { 38 } else { 48 }),
        C::Black => named(0),
        C::Red => named(1),
        C::Green => named(2),
        C::Yellow => named(3),
        C::Blue => named(4),
        C::Magenta => named(5),
        C::Cyan => named(6),
        C::Gray => named(7),
        C::DarkGray => bright(0),
        C::LightRed => bright(1),
        C::LightGreen => bright(2),
        C::LightYellow => bright(3),
        C::LightBlue => bright(4),
        C::LightMagenta => bright(5),
        C::LightCyan => bright(6),
        C::White => bright(7),
        C::Reset => (39 + base).to_string(),
    }
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
            say!("carrel: {e}");
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
            say!("carrel: {}: {e}", path.display());
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

    /// A release belongs to the press it ends. With words still selected
    /// from earlier, the release of a click on a BUTTON used to arrive as
    /// "copy the selection": the code block the chip had just copied was
    /// overwritten by the old word, and the note and the peek the press
    /// produced were gone before the hand had left the mouse.
    #[test]
    fn a_release_is_a_selection_release_only_when_the_press_began_one() {
        use carrel::action::Action;
        let src = "Some words here to select.\n\n```sh\ncargo build\n```\n\nafter\n";
        let mut app = App::new("t.md".into(), carrel_core::Document::parse(src), 60, 16);
        app.breadcrumb = false;
        app.on_resize(60, 16);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(60, 16)).unwrap();
        let mut painted = carrel::render::Painted::default();
        terminal
            .draw(|frame| carrel::render::draw_full(frame, &app, &mut painted, &mut HashMap::new()))
            .unwrap();
        let chip = painted
            .targets
            .as_slice()
            .iter()
            .find(|t| matches!(t.action, Action::YankBlockAt(_)))
            .expect("a copy chip")
            .zone;
        let mut ptr = Pointer::default();
        let down = MouseEventKind::Down(MouseButton::Left);
        let up = MouseEventKind::Up(MouseButton::Left);

        // Words selected earlier and still on screen.
        app.selection = Some(0..4);
        let press = mouse_action(
            mouse(down, chip.x, chip.y),
            &app,
            &painted.targets,
            &mut ptr,
        );
        assert!(matches!(press, Some(Action::YankBlockAt(_))), "{press:?}");
        assert_eq!(
            mouse_action(mouse(up, chip.x, chip.y), &app, &painted.targets, &mut ptr),
            None,
            "the button's click is over; it is not a selection ending"
        );

        // The control: a press in the text does begin one, and its release
        // ends it.
        let (x, y) = (app.text_x_now(), app.text_y());
        let press = mouse_action(mouse(down, x + 2, y), &app, &painted.targets, &mut ptr);
        assert!(matches!(press, Some(Action::SelectAnchor(_))), "{press:?}");
        assert_eq!(
            mouse_action(mouse(up, x + 2, y), &app, &painted.targets, &mut ptr),
            Some(Action::SelectRelease)
        );
    }

    #[test]
    fn small_menu_scroll_controls_reach_the_real_mouse_dispatcher() {
        use carrel::action::Action;
        let mut app = App::new("t.md".into(), carrel_core::Document::parse("text"), 24, 6);
        update(
            &mut app,
            Action::MenuOpen {
                at: (23, 5),
                byte: None,
            },
        );
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(24, 6)).unwrap();
        let mut painted = carrel::render::Painted::default();
        terminal
            .draw(|frame| carrel::render::draw_full(frame, &app, &mut painted, &mut HashMap::new()))
            .unwrap();
        for action in [Action::MenuMove(1), Action::MenuClose] {
            let target = painted
                .targets
                .as_slice()
                .iter()
                .find(|t| t.action == action)
                .unwrap();
            if matches!(action, Action::MenuMove(_)) {
                update(&mut app, Action::MenuMove(1));
                assert_eq!(
                    menu_mouse(
                        mouse(MouseEventKind::Moved, target.zone.x, target.zone.y),
                        &app,
                        &painted.targets
                    ),
                    None,
                    "hovering a scroll button must not clear the cursor it scrolls from"
                );
            }
            assert_eq!(
                menu_mouse(
                    mouse(
                        MouseEventKind::Down(MouseButton::Left),
                        target.zone.x,
                        target.zone.y
                    ),
                    &app,
                    &painted.targets
                ),
                Some(action)
            );
        }
        assert_eq!(
            menu_mouse(
                mouse(MouseEventKind::ScrollDown, 3, 3),
                &app,
                &painted.targets
            ),
            Some(Action::MenuMove(1))
        );
        assert_eq!(
            menu_mouse(
                mouse(MouseEventKind::ScrollUp, 3, 3),
                &app,
                &painted.targets
            ),
            Some(Action::MenuMove(-1))
        );
    }

    #[test]
    fn wheel_uses_the_open_panes_arrow_action_in_both_loops() {
        use carrel::action::{Action, Span};
        for home in [false, true] {
            for (open, expected) in [
                (Action::SettingsToggle, Action::SettingsMove(1)),
                (Action::HelpToggle, Action::Scroll(Span::Line, 1)),
                (Action::OutlineToggle, Action::OutlineMove(1)),
                (Action::MarkListToggle, Action::MarkListMove(1)),
                (Action::ForwardToggle, Action::ForwardMove(1)),
            ] {
                if home
                    && matches!(
                        open,
                        Action::MarkListToggle | Action::ForwardToggle | Action::OutlineToggle
                    )
                {
                    continue;
                }
                let mut app = if home {
                    App::new_home("/docs".into(), vec![], 24, 6)
                } else {
                    App::new(
                        "t.md".into(),
                        carrel_core::Document::parse("# Heading\n\ntext\n"),
                        24,
                        6,
                    )
                };
                app.marks.push(0);
                update(&mut app, open);
                let event = mouse(MouseEventKind::ScrollDown, 3, 3);
                let mut pointer = Pointer::default();
                let result = if home {
                    home_mouse_action(event, &app, &Targets::new(), &mut pointer)
                } else {
                    mouse_action(event, &app, &Targets::new(), &mut pointer)
                };
                assert_eq!(result, Some(expected), "home={home}, pane={open:?}");
            }
        }
    }

    /// A right-click, or a drag, anywhere inside an open footnote peek is
    /// the box's, and the box has no menu: it never reaches the document
    /// text hidden behind it. The bug: `open_menu_at` mapped the pointer
    /// straight through the box with `doc_span_at`, so a right-click on the
    /// footnote opened a context menu — "Open link", "Select word" — for
    /// whatever the box was covering. Beside the box a right-click is the
    /// page's, as it always was. Targets are the painter's own, not
    /// hand-built, so the box tested is the box drawn.
    #[test]
    fn a_right_click_inside_a_peek_never_reaches_the_text_behind_it() {
        use carrel::action::Action;
        let src = "Body[^n] and [a link](x.md).\n\n\
                   see [hidden](x.md) under the box and more words here\n\n\
                   and [another](y.md) line of text under it too\n\n\
                   [^n]: one [inside](z.md) the box, and a mark[^m] too\n\n\
                   [^m]: m\n";
        let (cols, rows) = (60, 20);
        let mut app = App::new("t.md".into(), carrel_core::Document::parse(src), cols, rows);
        let byte = u32::try_from(app.doc.text.find("[^n]").unwrap()).unwrap();
        update(&mut app, Action::FootnotePeek { at: (4, 0), byte });
        assert!(app.peek.is_some());
        let mut term =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(cols, rows)).unwrap();
        let mut painted = carrel::render::Painted::default();
        term.draw(|f| {
            carrel::render::draw_full(f, &app, &mut painted, &mut std::collections::HashMap::new());
        })
        .unwrap();
        let targets = painted.targets;
        let z = app
            .peek
            .as_ref()
            .unwrap()
            .zone(&app.doc, app.hyphenation(), cols, rows);
        let mut leaked = Vec::new();
        let mut covered_text = 0;
        for row in z.y..z.y + z.h {
            for col in z.x..z.x + z.w {
                if app.doc_span_at(col, row).is_some() {
                    covered_text += 1;
                }
                for kind in [
                    MouseEventKind::Down(MouseButton::Right),
                    MouseEventKind::Drag(MouseButton::Left),
                ] {
                    let a = mouse_action(
                        mouse(kind, col, row),
                        &app,
                        &targets,
                        &mut Pointer::default(),
                    );
                    if a.is_some() {
                        leaked.push(format!("{kind:?} at ({col},{row}) -> {a:?}"));
                    }
                }
            }
        }
        assert!(
            covered_text > 20,
            "the box must cover text, or this proves nothing"
        );
        assert!(leaked.is_empty(), "{}", leaked.join("\n"));

        // Beside the box, the page's own context menu, for the text there.
        let (col, row) = (0..rows)
            .flat_map(|r| (0..cols).map(move |c| (c, r)))
            .find(|&(c, r)| {
                app.doc_span_at(c, r).is_some() && !targets.covered_at(c, r, carrel::render::Z_MENU)
            })
            .expect("text beside the box");
        let a = mouse_action(
            mouse(MouseEventKind::Down(MouseButton::Right), col, row),
            &app,
            &targets,
            &mut Pointer::default(),
        );
        assert!(
            matches!(a, Some(Action::MenuOpen { byte: Some(_), .. })),
            "{a:?}"
        );
    }

    /// The wheel over a peek scrolls the footnote in it; anywhere else it
    /// scrolls the page, as it always did.
    #[test]
    fn the_wheel_over_a_peek_scrolls_the_footnote_and_beside_it_the_page() {
        use carrel::action::{Action, Span, Zone};
        let mut app = App::new(
            "t.md".into(),
            carrel_core::Document::parse("Body[^n].\n\n[^n]: one\n\n    two\n"),
            60,
            20,
        );
        let byte = u32::try_from(app.doc.text.find("[^n]").unwrap()).unwrap();
        update(&mut app, Action::FootnotePeek { at: (4, 0), byte });
        assert!(app.peek.is_some());
        let mut targets = Targets::new();
        targets.push(
            Action::Absorb,
            Zone::new(4, 1, 30, 6),
            carrel::render::Z_MENU,
        );
        let mut pointer = Pointer::default();
        let inside = mouse_action(
            mouse(MouseEventKind::ScrollDown, 10, 3),
            &app,
            &targets,
            &mut pointer,
        );
        assert!(
            matches!(inside, Some(Action::PeekScroll(n)) if n > 0),
            "{inside:?}"
        );
        let inside = mouse_action(
            mouse(MouseEventKind::ScrollUp, 10, 3),
            &app,
            &targets,
            &mut pointer,
        );
        assert!(
            matches!(inside, Some(Action::PeekScroll(n)) if n < 0),
            "{inside:?}"
        );
        let beside = mouse_action(
            mouse(MouseEventKind::ScrollDown, 50, 15),
            &app,
            &targets,
            &mut Pointer::default(),
        );
        assert!(
            matches!(beside, Some(Action::Scroll(Span::Line, _))),
            "{beside:?}"
        );
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
    /// **A click on the scrollbar track goes THERE.**
    ///
    /// It used to page one screen toward the pointer — the terminal idiom.
    /// Every graphical scrollbar a beginner has used jumps to the clicked
    /// position, and the thumb lands centred under the pointer.
    #[test]
    fn clicking_the_scrollbar_track_jumps_to_that_position() {
        use carrel::action::{Action, Targets};
        use ratatui::crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};

        // Tall enough that the bar has a real track to click in.
        let src = "line of text\n".repeat(400);
        let mut app = App::new("t.md".into(), carrel_core::Document::parse(&src), 80, 24);
        assert!(app.layout.max_scroll(app.text_h()) > 0, "there is a track");

        let targets = Targets::default();
        let mut ptr = Pointer::default();
        let bar_x = app.cols - 1;
        let press = move |row: u16| MouseEvent {
            kind: MouseEventKind::Down(ratatui::crossterm::event::MouseButton::Left),
            column: bar_x,
            row,
            modifiers: KeyModifiers::NONE,
        };

        // Near the bottom of the track: a jump deep into the document, not
        // one page. `Scroll(Page, 1)` is exactly the old, wrong answer.
        let low = app.text_y() + app.text_h() - 1;
        let got = mouse_action(press(low), &app, &targets, &mut ptr);
        match got {
            Some(Action::ScrollTo(row)) => assert!(
                row > u32::from(app.text_h()),
                "a click near the end lands near the end, got row {row}"
            ),
            other => panic!("expected a jump, got {other:?}"),
        }

        // And from the far end, a click at the top of the track comes back.
        // The app has to actually be scrolled first, or the thumb is still
        // sitting at the top and the press is a GRAB, not a track click.
        update(&mut app, Action::GoToEnd);
        assert!(app.view.scroll_row > 0, "we are away from the top");
        match mouse_action(press(app.text_y()), &app, &targets, &mut ptr) {
            Some(Action::ScrollTo(row)) => assert!(
                row < u32::from(app.text_h()),
                "a click at the top of the track lands at the top, got {row}"
            ),
            other => panic!("expected a jump back, got {other:?}"),
        }
    }

    /// **Ctrl-C is a copy key when there is something to copy.**
    ///
    /// Raw mode clears ISIG, so no SIGINT is ever generated and this key is
    /// carrel's to define. It quit instantly from every state — so the
    /// reflex a beginner brings from every other program (select some text,
    /// press Ctrl-C) destroyed the program instead of filling the clipboard.
    #[test]
    fn ctrl_c_copies_a_selection_and_quits_only_without_one() {
        use carrel::action::Action;
        use ratatui::crossterm::event::{KeyCode, KeyModifiers};

        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        let mut app = App::new(
            "t.md".into(),
            carrel_core::Document::parse("hello there"),
            40,
            10,
        );
        let mut keys = Keys::new();

        assert_eq!(
            key_action(&mut keys, &app, ctrl_c),
            Some(Action::Quit),
            "nothing selected: the escape hatch is intact"
        );

        app.selection = Some(0..5);
        assert_eq!(
            key_action(&mut keys, &app, ctrl_c),
            Some(Action::SelectRelease),
            "something selected: it copies instead of quitting"
        );
    }

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

    /// Everything modal that is up, read off the fields rather than through
    /// `App::pane`, so a pane that function forgets still counts.
    fn up(a: &App) -> Vec<&'static str> {
        [
            (a.lightbox.is_some(), "lightbox"),
            (a.notes.pane.is_some() || a.notes.draft.is_some(), "notes"),
            (a.menu.is_some(), "menu"),
            (a.help.is_some(), "help"),
            (a.settings.is_some(), "settings"),
            (a.outline.is_some(), "outline"),
            (a.mark_list.is_some(), "bookmarks"),
            (a.backlinks.is_some(), "backlinks"),
            (a.forward.is_some(), "forward"),
            (a.info, "info"),
            (a.peek.is_some(), "peek"),
            (a.searching(), "search"),
        ]
        .into_iter()
        .filter_map(|(on, name)| on.then_some(name))
        .collect()
    }

    /// The reported bug, by KEY, through the one dispatcher both loops use:
    /// `I` then `h` left the info card drawn over the help sheet. Every
    /// pane's opener key is pressed with every other pane up; whatever is up
    /// afterwards is at most ONE pane — the same one, or (over a pane with
    /// no text field) the one the key opens in its place. Then Esc must
    /// close it — the info card ignored Esc, and `q` with the card up quit
    /// the whole program.
    #[test]
    fn with_a_pane_up_a_key_never_stacks_another_and_esc_closes_it() {
        use carrel::action::{Action, Direction};
        use ratatui::crossterm::event::{KeyCode, KeyModifiers};

        let src = "# One\n\nSee [two](two.md).\n\n![alt](pic.png)\n\n## Two\n\nMore.\n";
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("one.md");
        std::fs::write(&file, src).unwrap();
        let fresh = || {
            let mut a = App::new(
                file.display().to_string(),
                carrel_core::Document::parse(src),
                80,
                24,
            );
            a.file = Some(file.clone());
            update(&mut a, Action::MarkToggle);
            a
        };
        let openers = [
            ("help", Action::HelpToggle),
            ("settings", Action::SettingsToggle),
            ("outline", Action::OutlineToggle),
            ("bookmarks", Action::MarkListToggle),
            ("backlinks", Action::BacklinksToggle),
            ("forward", Action::ForwardToggle),
            ("info", Action::InfoToggle),
            ("notes", Action::NotesToggle),
            ("lightbox", Action::ImageOpen(None)),
            ("search", Action::SearchOpen(Direction::Forward)),
        ];
        let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
        let opener_keys = ['h', ',', 'o', '"', 'L', 'l', 'I', 'V', 'a', '/', '?', 'q']
            .map(|c| key(KeyCode::Char(c)));
        let mut stacked = Vec::new();
        for (name, open) in openers {
            for k in opener_keys.iter().copied().chain([key(KeyCode::F(1))]) {
                let mut a = fresh();
                update(&mut a, open);
                assert_eq!(up(&a), vec![name], "{open:?} opens {name}");
                let mut keys = Keys::new();
                let out = key_action(&mut keys, &a, k).map(|act| update(&mut a, act));
                let now = up(&a);
                // Its own key may close it and a pane key may replace it;
                // nothing may JOIN it, a pane that types keeps its letters,
                // and a letter must not quit the program from under a pane.
                let types = matches!(name, "help" | "outline" | "search" | "notes");
                let replaced = now.len() == 1 && now != vec![name];
                if now.len() > 1 || (types && replaced) || out == Some(Outcome::Quit) {
                    stacked.push(format!("{name} + {:?} -> {now:?} {out:?}", k.code));
                }
            }
            // Esc closes it — in at most two presses, for the panes whose
            // first Esc clears a typed filter.
            let mut a = fresh();
            update(&mut a, open);
            let mut keys = Keys::new();
            for _ in 0..2 {
                if let Some(act) = key_action(&mut keys, &a, key(KeyCode::Esc)) {
                    update(&mut a, act);
                }
            }
            if !up(&a).is_empty() {
                stacked.push(format!("{name}: Esc Esc leaves {:?} up", up(&a)));
            }
        }
        assert!(stacked.is_empty(), "{}", stacked.join("\n"));
    }

    /// Paint `app` and return what the frame registered — the real input
    /// to both mouse dispatchers.
    fn frame_targets(app: &App) -> Targets {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(app.cols, app.rows)).unwrap();
        let mut painted = carrel::render::Painted::default();
        terminal
            .draw(|frame| carrel::render::draw_full(frame, app, &mut painted, &mut HashMap::new()))
            .unwrap();
        painted.targets
    }

    /// Where the frame painted a target for `action`, whatever is on top
    /// of it now.
    fn zone_of(targets: &Targets, want: fn(&carrel::action::Action) -> bool) -> (u16, u16) {
        let z = targets
            .as_slice()
            .iter()
            .find(|t| want(&t.action))
            .expect("the frame painted it")
            .zone;
        (z.x, z.y)
    }

    /// **A click outside an open pane closes it and is used up** — the
    /// maintainer's rule, through the real mouse path of BOTH loops.
    ///
    /// The bugs it pins: with the info card up, the status row's `q quit`
    /// quit the program (the `q` KEY closes the card); `T theme` cycled
    /// while the `T` key was refused; a click on the page beside the
    /// links pane followed a link, or started a selection, underneath.
    /// And `≡` did nothing while any pane was up — it replaces the pane.
    #[test]
    #[allow(clippy::too_many_lines)] // one walk: every pane, both loops, every way out
    fn a_click_outside_a_pane_closes_it_and_does_nothing_else_in_both_loops() {
        use carrel::action::Action;
        let src = "# One\n\nSee [two](two.md) and more words to click on.\n\n## Two\n";
        let down = |b, (x, y)| mouse(MouseEventKind::Down(b), x, y);
        let dispatch = |app: &App, m, targets: &Targets, ptr: &mut Pointer| {
            if app.is_home() || app.home().is_some() {
                home_mouse_action(m, app, targets, ptr)
            } else {
                mouse_action(m, app, targets, ptr)
            }
        };
        let fresh = |home: bool, open: Action| {
            let mut a = if home {
                App::new_home("/docs".into(), vec![], 80, 24)
            } else {
                App::new("t.md".into(), carrel_core::Document::parse(src), 80, 24)
            };
            update(&mut a, open);
            assert!(a.pane().is_some(), "{open:?} opened a pane");
            a
        };
        let quit = |a: &Action| matches!(a, Action::Quit | Action::CloseFile);
        let theme = |a: &Action| *a == Action::ThemeCycle;
        let launcher = |a: &Action| matches!(a, Action::MenuOpen { .. });
        let mut cases: Vec<(bool, Action)> = vec![
            (false, Action::InfoToggle),
            (false, Action::ForwardToggle),
            (false, Action::SettingsToggle),
            (false, Action::HelpToggle),
            (true, Action::SettingsToggle),
            (true, Action::HelpToggle),
            (true, Action::PickerOpen),
        ];
        // The reader's own chips are on its status row, the home screen's
        // `q quit` on its own; both loops are walked.
        let mut chips = Vec::new();
        for (home, open) in cases.drain(..) {
            let ctx = format!("home={home} {open:?}");
            // The chips: used up closing the pane, never quitting.
            for (chip, name) in [(quit as fn(&Action) -> bool, "q quit"), (theme, "T theme")] {
                let mut a = fresh(home, open);
                let t = frame_targets(&a);
                if !t.as_slice().iter().any(|x| chip(&x.action)) {
                    continue; // this row does not offer it at 80 columns
                }
                let at = zone_of(&t, chip);
                if t.hit(at.0, at.1)
                    .is_some_and(|h| h.action == Action::Absorb)
                {
                    continue; // under the pane itself: the pane's, not beside it
                }
                let mut ptr = Pointer::default();
                let got = dispatch(&a, down(MouseButton::Left, at), &t, &mut ptr);
                assert_eq!(got, Some(Action::PaneClose), "{ctx}: {name}");
                let out = update(&mut a, got.unwrap());
                assert_ne!(out, Outcome::Quit, "{ctx}: {name} quit from under the pane");
                assert_eq!(a.pane(), None, "{ctx}: {name} closed the pane");
                assert!(!a.theme_cycle, "{ctx}: {name} also cycled the theme");
                assert_eq!(a.is_home(), home, "{ctx}: {name} left the screen");
                chips.push((home, name));
            }
            // The page beside the pane, both buttons: closes, nothing else.
            for button in [MouseButton::Left, MouseButton::Right] {
                let mut a = fresh(home, open);
                let t = frame_targets(&a);
                let mut ptr = Pointer::default();
                let corner = (0, 0);
                assert!(
                    !t.covered_at(0, 0, carrel::render::Z_MENU),
                    "{ctx}: the corner is outside the pane"
                );
                let got = dispatch(&a, down(button, corner), &t, &mut ptr);
                assert_eq!(got, Some(Action::PaneClose), "{ctx}: {button:?} beside it");
                let anchor = a.view.anchor;
                update(&mut a, got.unwrap());
                assert_eq!(a.pane(), None, "{ctx}");
                assert_eq!(a.view.anchor, anchor, "{ctx}: the document moved");
                // The rest of the gesture is the same click: nothing.
                let rest = [
                    mouse(MouseEventKind::Drag(button), 5, 3),
                    mouse(MouseEventKind::Up(button), 5, 3),
                ];
                for m in rest {
                    if let Some(act) = dispatch(&a, m, &t, &mut ptr) {
                        update(&mut a, act);
                    }
                }
                assert_eq!(a.selection, None, "{ctx}: the used-up press selected");
                assert_eq!(a.menu.as_ref().map(|_| ()), None, "{ctx}: a menu opened");
            }
            // `≡` always works: it closes the pane and opens the menu.
            let mut a = fresh(home, open);
            let t = frame_targets(&a);
            let mut ptr = Pointer::default();
            let got = dispatch(
                &a,
                down(MouseButton::Left, zone_of(&t, launcher)),
                &t,
                &mut ptr,
            );
            assert!(
                matches!(got, Some(Action::MenuOpen { .. })),
                "{ctx}: {got:?}"
            );
            update(&mut a, got.unwrap());
            assert_eq!(
                a.pane(),
                Some(Pane::Menu),
                "{ctx}: \u{2261} replaced the pane"
            );
            assert_eq!(
                (
                    a.help.is_some(),
                    a.settings.is_some(),
                    a.info,
                    a.forward.is_some()
                ),
                (false, false, false, false),
                "{ctx}: nothing left under the menu"
            );
        }
        // Both loops, both chips, were really clicked through.
        for want in [
            (false, "q quit"),
            (false, "T theme"),
            (true, "q quit"),
            (true, "T theme"),
        ] {
            assert!(chips.contains(&want), "never clicked {want:?}: {chips:?}");
        }
    }

    /// The pane's OWN footer chips keep working with the pane up: they are
    /// the pane, not the chrome beside it. So does the lamp, and so does a
    /// click on the search prompt being typed into.
    #[test]
    fn a_panes_own_footer_chip_still_works() {
        use carrel::action::Action;
        let mut a = App::new(
            "t.md".into(),
            carrel_core::Document::parse("# One\n"),
            80,
            24,
        );
        update(&mut a, Action::InfoToggle);
        let t = frame_targets(&a);
        let mut ptr = Pointer::default();
        let at = zone_of(&t, |a| *a == Action::InfoToggle);
        let got = mouse_action(
            mouse(MouseEventKind::Down(MouseButton::Left), at.0, at.1),
            &a,
            &t,
            &mut ptr,
        );
        assert_eq!(
            got,
            Some(Action::InfoToggle),
            "`esc close` is the card's own"
        );

        // The lamp, lit on the footer or folded on the status row, works
        // from any state: it folds the hints and leaves the card up.
        for hints in [true, false] {
            let mut a = App::new(
                "t.md".into(),
                carrel_core::Document::parse("# One\n"),
                80,
                24,
            );
            a.hints = hints;
            update(&mut a, Action::InfoToggle);
            let t = frame_targets(&a);
            let at = zone_of(&t, |a| *a == Action::HintsToggle);
            let got = mouse_action(
                mouse(MouseEventKind::Down(MouseButton::Left), at.0, at.1),
                &a,
                &t,
                &mut Pointer::default(),
            );
            assert_eq!(got, Some(Action::HintsToggle), "hints={hints}");
            update(&mut a, got.unwrap());
            assert_eq!((a.hints, a.pane()), (!hints, Some(Pane::Info)));
        }

        // The search prompt IS the search pane: a click on what is being
        // typed keeps it, while a click on the page beside it cancels it.
        let src = "alpha beta gamma\n";
        let mut a = App::new("t.md".into(), carrel_core::Document::parse(src), 80, 24);
        update(
            &mut a,
            Action::SearchOpen(carrel::action::Direction::Forward),
        );
        update(
            &mut a,
            Action::SearchKey(carrel::action::SearchKey::Char('b')),
        );
        let t = frame_targets(&a);
        let (_, row) = zone_of(&t, |a| matches!(a, Action::MenuOpen { .. }));
        let on_prompt = mouse(MouseEventKind::Down(MouseButton::Left), 10, row);
        let got = mouse_action(on_prompt, &a, &t, &mut Pointer::default());
        assert_eq!(got, None, "a click on the prompt is the prompt's");
        assert!(a.searching());
        let beside = mouse(MouseEventKind::Down(MouseButton::Left), 2, 0);
        let got = mouse_action(beside, &a, &t, &mut Pointer::default());
        assert_eq!(got, Some(Action::PaneClose));
        update(&mut a, got.unwrap());
        assert!(!a.searching(), "a click beside the prompt is Esc");
    }

    /// A pane-opening key over a pane with no text field REPLACES it, the
    /// way `≡` replaces any pane — `h` over the info card used to do
    /// nothing at all, silently. Over a pane that types, the letter is
    /// text: it narrows the help sheet or the outline, never swaps them.
    #[test]
    fn a_pane_key_replaces_a_pane_that_does_not_type() {
        use carrel::action::Action;
        use ratatui::crossterm::event::{KeyCode, KeyModifiers};
        let src = "# One\n\nSee [two](two.md).\n\n## Two\n";
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("one.md");
        std::fs::write(&file, src).unwrap();
        let press = |open: Action, code| {
            let mut a = App::new(
                file.display().to_string(),
                carrel_core::Document::parse(src),
                80,
                24,
            );
            a.file = Some(file.clone());
            update(&mut a, Action::MarkToggle);
            update(&mut a, open);
            let k = KeyEvent::new(code, KeyModifiers::NONE);
            if let Some(act) = key_action(&mut Keys::new(), &a, k) {
                update(&mut a, act);
            }
            a
        };
        let c = KeyCode::Char;
        assert_eq!(press(Action::InfoToggle, c('h')).pane(), Some(Pane::Help));
        assert_eq!(
            press(Action::InfoToggle, KeyCode::F(1)).pane(),
            Some(Pane::Help)
        );
        assert_eq!(
            press(Action::BacklinksToggle, c('l')).pane(),
            Some(Pane::Forward)
        );
        assert_eq!(
            press(Action::ForwardToggle, c('L')).pane(),
            Some(Pane::Backlinks)
        );
        assert_eq!(
            press(Action::SettingsToggle, c('o')).pane(),
            Some(Pane::Outline)
        );
        assert_eq!(
            press(Action::MarkListToggle, c('I')).pane(),
            Some(Pane::Info)
        );
        assert_eq!(press(Action::InfoToggle, c('/')).pane(), Some(Pane::Search));
        // A key that is not a pane's opener stays dead over a pane: Enter
        // on the card must not follow a link nobody can see.
        let a = press(Action::InfoToggle, KeyCode::Enter);
        assert_eq!((a.pane(), a.lightbox), (Some(Pane::Info), None));
        assert_eq!(press(Action::InfoToggle, c('a')).pane(), Some(Pane::Info));
        // Panes that type keep their letters.
        let help = press(Action::HelpToggle, c('I'));
        assert_eq!(help.pane(), Some(Pane::Help));
        assert_eq!(help.help.as_ref().map(|h| h.filter.as_str()), Some("I"));
        let outline = press(Action::OutlineToggle, c('h'));
        assert_eq!(outline.pane(), Some(Pane::Outline));
        // And on the home screen: F1 over settings is help.
        let mut home = App::new_home("/docs".into(), vec![], 80, 24);
        update(&mut home, Action::SettingsToggle);
        let f1 = KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE);
        let act = key_action(&mut Keys::new(), &home, f1).expect("F1 answers");
        update(&mut home, act);
        assert_eq!(home.pane(), Some(Pane::Help));
    }

    /// A right-click never opens a menu over a pane. The list this used to
    /// keep had no settings pane and no info card, so both got a menu
    /// stacked on top with no rule for which one answered.
    #[test]
    fn a_right_click_opens_no_menu_over_any_pane() {
        use carrel::action::{Action, Direction};
        let src = "# One\n\nSee [two](two.md).\n";
        let right = mouse(MouseEventKind::Down(MouseButton::Right), 3, 3);
        for open in [
            Action::HelpToggle,
            Action::SettingsToggle,
            Action::OutlineToggle,
            Action::ForwardToggle,
            Action::InfoToggle,
            Action::NotesToggle,
            Action::SearchOpen(Direction::Forward),
        ] {
            let mut a = App::new("t.md".into(), carrel_core::Document::parse(src), 80, 24);
            assert!(
                open_menu_at(right, &a).is_some(),
                "with nothing up it opens"
            );
            update(&mut a, open);
            assert!(a.pane().is_some(), "{open:?} opened something");
            assert_eq!(open_menu_at(right, &a), None, "over {:?}", a.pane());
        }
    }

    /// The information card scrolls with the reader's whole vocabulary, not
    /// just the arrows. Before the one-pane rule the reader's map handled
    /// the card's keys, so `Ctrl-D` scrolled it and `g`/`G`/Home/End
    /// reached the document under it; `map_info` then dropped all of them,
    /// and `scroll_info` handled a half page that nothing could emit.
    #[test]
    fn the_info_card_takes_the_readers_scroll_keys_and_leaves_the_document() {
        use ratatui::crossterm::event::{KeyCode, KeyModifiers};
        let src = "# Heading\n\ntext\n".repeat(20);
        // Short enough that the card's dozen rows overflow it.
        let mut app = App::new("t.md".into(), carrel_core::Document::parse(&src), 28, 9);
        update(&mut app, carrel::action::Action::InfoToggle);
        assert_eq!(app.pane(), Some(Pane::Info));
        let height = usize::from(app.info_size().1.saturating_sub(2));
        let max = app.info_lines().len().saturating_sub(height);
        assert!(
            max > 4 && height >= 4,
            "the card must scroll: {max} {height}"
        );
        let anchor = app.view.anchor;
        let press = |app: &mut App, code, mods| {
            let action = key_action(&mut Keys::new(), app, KeyEvent::new(code, mods))
                .unwrap_or_else(|| panic!("{code:?} {mods:?} does nothing on the card"));
            update(app, action);
            app.info_scroll
        };
        let none = KeyModifiers::NONE;
        let ctrl = KeyModifiers::CONTROL;
        assert_eq!(press(&mut app, KeyCode::End, none), max, "End");
        assert_eq!(press(&mut app, KeyCode::Home, none), 0, "Home");
        assert_eq!(press(&mut app, KeyCode::Char('G'), none), max, "G");
        assert_eq!(press(&mut app, KeyCode::Char('g'), none), 0, "g");
        assert_eq!(
            press(&mut app, KeyCode::Char('d'), ctrl),
            height / 2,
            "Ctrl-D"
        );
        assert_eq!(press(&mut app, KeyCode::Char('u'), ctrl), 0, "Ctrl-U");
        assert_eq!(
            press(&mut app, KeyCode::Char('f'), ctrl),
            height.min(max),
            "Ctrl-F"
        );
        assert_eq!(press(&mut app, KeyCode::Char('b'), ctrl), 0, "Ctrl-B");
        assert_eq!(app.pane(), Some(Pane::Info), "still up");
        assert_eq!(app.view.anchor, anchor, "the document never moved");
    }

    /// The menu is the one pane a right-click may replace: right-clicking
    /// outside an open menu moves it there. The one-pane rule made
    /// `open_menu_at` refuse whenever ANY pane was up, and the menu counts,
    /// so the arm in `menu_mouse` that promises this was dead in both loops.
    #[test]
    fn a_right_click_outside_an_open_menu_moves_it_in_both_loops() {
        use carrel::action::Action;
        for home in [false, true] {
            let mut app = if home {
                App::new_home("/docs".into(), vec![], 80, 24)
            } else {
                App::new(
                    "t.md".into(),
                    carrel_core::Document::parse("# Heading\n\ntext\n"),
                    80,
                    24,
                )
            };
            update(
                &mut app,
                Action::MenuOpen {
                    at: (2, 2),
                    byte: None,
                },
            );
            assert_eq!(app.pane(), Some(Pane::Menu));
            let right = mouse(MouseEventKind::Down(MouseButton::Right), 60, 20);
            let mut pointer = Pointer::default();
            let got = if home {
                home_mouse_action(right, &app, &Targets::new(), &mut pointer)
            } else {
                mouse_action(right, &app, &Targets::new(), &mut pointer)
            };
            assert!(
                matches!(got, Some(Action::MenuOpen { at: (60, 20), .. })),
                "home={home}: {got:?}"
            );
            // …and `update` lets it through: a pane's own opener replaces it.
            update(&mut app, got.unwrap());
            assert_eq!(
                app.menu.as_ref().map(|m| m.at),
                Some((60, 20)),
                "home={home}"
            );
        }
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
            r.poll(p, None)
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
        assert_eq!(r.poll(Some(&a), None), None);
        // Following a link to another file must NOT read as "changed".
        r.last_check = Instant::now().checked_sub(Duration::from_secs(2)).unwrap();
        assert_eq!(r.poll(Some(&b), None), None, "new file, new baseline");
    }

    /// The baseline is the file as it was READ. Taking the first look as the
    /// baseline lost any write that landed between the read and the look.
    #[test]
    fn a_write_between_opening_and_the_first_look_is_a_change() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("live.md");
        std::fs::write(&f, "old").unwrap();
        let loaded = std::fs::metadata(&f).unwrap().modified().unwrap();
        let later = loaded + Duration::from_secs(5);
        std::fs::File::options()
            .write(true)
            .open(&f)
            .unwrap()
            .set_modified(later)
            .unwrap();
        let mut r = Reloader::new();
        r.last_check = Instant::now().checked_sub(Duration::from_secs(2)).unwrap();
        assert_eq!(r.poll(Some(&f), Some(loaded)), Some(Reload::Changed));
        // The control: untouched since it was read, the first look is quiet.
        let mut r = Reloader::new();
        r.last_check = Instant::now().checked_sub(Duration::from_secs(2)).unwrap();
        assert_eq!(r.poll(Some(&f), Some(later)), None);
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

#[cfg(test)]
mod lightbox_pipeline_tests {
    use super::*;

    #[test]
    fn pathless_documents_start_decoding_absolute_images() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("image.png");
        image::DynamicImage::new_rgb8(2, 2).save(&path).unwrap();
        let source = format!("![local]({})", path.display());
        let mut app = App::new(
            "stdin".into(),
            carrel_core::Document::parse(&source),
            80,
            24,
        );
        assert!(app.file.is_none());
        let mut pipeline = Images::detect();
        pipeline.sync(&mut app);
        assert!(
            pipeline.rx.is_some(),
            "None filename must not suppress initial decode"
        );
        assert!(pipeline.initialized);
    }
}
