//! Tier 4 of the testing strategy (testing-strategy research, Q34 in the notes repo): the automated pty smoke.
//!
//! `TestBackend` cannot reach PTY behaviour — TTY detection, the alternate
//! screen, raw-mode entry/exit, the real event loop. Those failures have
//! bitten before (`Picker::from_query_stdio` hung the binary; a detached pty
//! reported 0×0 and rendered nothing), and until 2026-08-12 the only guard
//! was a manual `script` invocation in a developer's shell. This test IS that
//! smoke, checked in: it runs the real binary inside a pty that `script`(1)
//! allocates, feeds it keystrokes, and asserts on the raw byte stream.
//!
//! Unix-only by nature; CI's ubuntu runner has `script` (util-linux). If the
//! host lacks it, the test skips loudly rather than failing falsely.
#![cfg(unix)]

use std::path::Path;
use std::process::Command;

/// Run the built binary inside a real pty, typing `keys` after a beat.
/// Returns the raw captured terminal stream (escapes and all).
fn pty_run(args: &str, keys: &str, dir: &Path) -> String {
    let bin = env!("CARGO_BIN_EXE_carrel");
    let out = dir.join("pty-capture");
    let cfg = dir.join("cfg");
    let state = dir.join("state");
    // `script` gives the child a real pty; the subshell delays the keys so
    // the app is up before they arrive. XDG dirs point at the scratch dir —
    // a test must never be able to touch the real config or positions.
    let cmd = format!(
        "( sleep 1; printf '{keys}' ) | \
         XDG_CONFIG_HOME='{}' XDG_STATE_HOME='{}' \
         script -qec 'stty rows 20 cols 76; {bin} {args}' '{}' >/dev/null 2>&1",
        cfg.display(),
        state.display(),
        out.display(),
    );
    let status = Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .current_dir(dir)
        .status()
        .expect("sh must run");
    assert!(status.success(), "the binary must exit cleanly");
    std::fs::read_to_string(&out).unwrap_or_default()
}

fn script_available() -> bool {
    Command::new("script")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Like [`pty_run`], but the whole shell command inside the pty is given —
/// so a document pipe can feed the binary while keys arrive via the pty
/// (crossterm falls back to `/dev/tty` when stdin is not a terminal).
fn pty_run_cmd(cmd_in_pty: &str, keys: &str, delay: &str, dir: &Path) -> String {
    let out = dir.join("pty-capture");
    let cfg = dir.join("cfg");
    let state = dir.join("state");
    let cmd = format!(
        "( sleep {delay}; printf '{keys}' ) | \
         XDG_CONFIG_HOME='{}' XDG_STATE_HOME='{}' \
         script -qec 'stty rows 20 cols 76; {cmd_in_pty}' '{}' >/dev/null 2>&1",
        cfg.display(),
        state.display(),
        out.display(),
    );
    let status = Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .current_dir(dir)
        .status()
        .expect("sh must run");
    assert!(status.success(), "the binary must exit cleanly");
    std::fs::read_to_string(&out).unwrap_or_default()
}

#[test]
fn a_piped_document_enters_the_tui_and_leaves_it() {
    if !script_available() {
        eprintln!("skipping: script(1) not available");
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let bin = env!("CARGO_BIN_EXE_carrel");
    let cap = pty_run_cmd(
        &format!("sh -c \"printf \\\"# Piped\\n\\nhello stream\\n\\\" | {bin}\""),
        "q",
        "1",
        d.path(),
    );
    assert!(cap.contains("\u{1b}[?1049h"), "enters the alternate screen");
    assert!(cap.contains("\u{1b}[?1049l"), "and leaves it");
    assert!(cap.contains("Piped"), "renders the piped heading");
    assert!(cap.contains("(stdin"), "the label says where it came from");
}

#[test]
fn a_slow_producer_streams_into_a_live_reader() {
    if !script_available() {
        eprintln!("skipping: script(1) not available");
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let bin = env!("CARGO_BIN_EXE_carrel");
    let cap = pty_run_cmd(
        &format!(
            "sh -c \"( printf \\\"# One\\n\\n\\\"; sleep 2; printf \\\"two arrived\\n\\\" ) | {bin}\""
        ),
        "q",
        "3",
        d.path(),
    );
    assert!(cap.contains("One"), "the first chunk painted");
    assert!(
        cap.contains("two arrived"),
        "the second chunk landed in the live reader"
    );
    assert!(cap.contains("\u{1b}[?1049l"), "and it exits cleanly");
}

#[test]
fn dash_on_a_terminal_refuses_with_a_note() {
    if !script_available() {
        eprintln!("skipping: script(1) not available");
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let bin = env!("CARGO_BIN_EXE_carrel");
    // stdin IS the pty here: `carrel -` must refuse rather than wait on a
    // pipe that is really a keyboard. The pty merges stderr into the
    // capture, and `script` propagates nothing useful, so the exit code is
    // echoed into the stream instead of asserted on the harness.
    let out = d.path().join("cap");
    let cmd = format!(
        "XDG_CONFIG_HOME='{0}' XDG_STATE_HOME='{0}' \
         script -qec 'stty rows 20 cols 76; {bin} - ; echo RC=$?' '{1}' >/dev/null 2>&1",
        d.path().display(),
        out.display(),
    );
    Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .status()
        .expect("sh runs");
    let cap = std::fs::read_to_string(&out).unwrap_or_default();
    assert!(
        cap.contains("stdin is a terminal"),
        "the refusal names the reason: {cap}"
    );
    assert!(cap.contains("RC=1"), "and exits nonzero: {cap}");
}

#[test]
fn the_reader_enters_the_alternate_screen_paints_and_leaves() {
    if !script_available() {
        eprintln!("SKIP: `script`(1) not available — pty smoke not run");
        return;
    }
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("doc.md"), "# Title\n\nbody text\n").unwrap();

    let raw = pty_run("doc.md", "q", d.path());

    // In, painted, out — the exact trio the manual smoke always checked.
    assert!(
        raw.contains("\x1b[?1049h"),
        "must ENTER the alternate screen"
    );
    assert!(
        raw.contains("\x1b[?1049l"),
        "must LEAVE the alternate screen — a stuck terminal is the worst bug a TUI can ship"
    );
    assert!(raw.contains("Title"), "the document must actually paint");
    assert!(
        raw.contains("╭●"),
        "the lamplight footer must be lit on a fresh config"
    );
}

#[test]
fn auto_read_drifts_to_the_end_without_a_resize() {
    if !script_available() {
        eprintln!("SKIP: `script`(1) not available — pty smoke not run");
        return;
    }
    let d = tempfile::tempdir().unwrap();
    // A little longer than the 20-row pty, so reaching the end takes real
    // ticks (AUTO_READ_MS is 300) but not many of them.
    let body = "a paragraph to scroll past\n\n".repeat(12);
    std::fs::write(d.path().join("doc.md"), format!("# Drift\n\n{body}")).unwrap();

    // Press `A`, wait for the drift, then quit. No resize event is ever
    // delivered — which is the whole point: the tick used to be nested inside
    // the debounced-resize branch, so auto-read advanced only while a window
    // was being dragged, and the end-of-document note never arrived at all.
    let bin = env!("CARGO_BIN_EXE_carrel");
    let out = d.path().join("pty-capture");
    let cfg = d.path().join("cfg");
    let state = d.path().join("state");
    let cache = d.path().join("cache");
    let cmd = format!(
        "( sleep 1; printf 'A'; sleep 6; printf 'q' ) | \
         XDG_CONFIG_HOME='{}' XDG_STATE_HOME='{}' XDG_CACHE_HOME='{}' HOME='{}' \
         timeout 40 script -qec 'stty rows 20 cols 76; {bin} doc.md' '{}' >/dev/null 2>&1",
        cfg.display(),
        state.display(),
        cache.display(),
        d.path().display(),
        out.display(),
    );
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .current_dir(d.path())
        .status()
        .expect("sh must run");
    assert!(status.success(), "the binary must exit cleanly");
    let raw = std::fs::read_to_string(&out).unwrap_or_default();

    assert!(
        raw.contains("auto-read"),
        "pressing A must announce itself: {raw:?}"
    );
    assert!(
        raw.contains("the end"),
        "auto-read must reach the end on its own clock, with no resize to ride"
    );
}

#[test]
fn every_frame_is_bracketed_in_a_synchronized_update() {
    if !script_available() {
        eprintln!("SKIP: `script`(1) not available — pty smoke not run");
        return;
    }
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("doc.md"), "# Title\n\nbody text\n").unwrap();

    let raw = pty_run("doc.md", "jjjq", d.path());

    let begins = raw.matches("\x1b[?2026h").count();
    let ends = raw.matches("\x1b[?2026l").count();

    // Without a frame boundary the terminal is free to render a half-applied
    // update: a fast scroll rewrites most of the screen, the write spans a
    // refresh, and characters from the previous frame stay on screen where the
    // new one has not landed. Reproduced on both ghostty and foot.
    assert!(begins > 0, "frames must open a synchronized update");
    assert_eq!(
        begins, ends,
        "every synchronized update must be closed — an unbalanced one freezes the screen"
    );

    // The teeth: the paint has to be INSIDE the update, not merely near one.
    let open = raw.find("\x1b[?2026h").unwrap();
    let close = raw[open..]
        .find("\x1b[?2026l")
        .expect("an opened synchronized update must close");
    assert!(
        raw[open..open + close].contains("Title"),
        "the document must paint between begin and end, not outside them"
    );
}

#[test]
fn the_home_screen_survives_a_pty_round_trip_too() {
    if !script_available() {
        eprintln!("SKIP: `script`(1) not available — pty smoke not run");
        return;
    }
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("a.md"), "# A\n").unwrap();

    // No file argument: the home screen. Ctrl-C quits from any mode.
    let raw = pty_run("", "\\003", d.path());

    assert!(raw.contains("\x1b[?1049h"), "enters the alternate screen");
    assert!(raw.contains("\x1b[?1049l"), "leaves it again");
    assert!(
        raw.contains("carrel"),
        "the wordmark (splash or collapsed) must paint"
    );
}

/// The home screen's list must pick up a file written while it is up.
///
/// The unit tests cover the reconciliation ([`carrel::home::Home::begin_rescan`]
/// and friends); nothing but a real run covers the *timer* that fires it, which
/// is the half that was missing — the walk ran once at startup and never again,
/// so a new document needed a restart to appear (maintainer report, 2026-08-29).
#[test]
fn a_file_created_while_the_home_screen_is_up_appears_on_it() {
    if !script_available() {
        eprintln!("SKIP: `script`(1) not available — pty smoke not run");
        return;
    }
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("already-here.md"), "# Already\n").unwrap();

    let bin = env!("CARGO_BIN_EXE_carrel");
    let out = d.path().join("pty-capture");
    let cfg = d.path().join("cfg");
    let state = d.path().join("state");
    // The same subshell that eventually types Ctrl-C writes the file first, a
    // beat after the startup walk has finished — so the only thing that can
    // put it on screen is a later walk.
    let cmd = format!(
        "( sleep 2; : > written-later.md; sleep 3; printf '\\003' ) | \
         XDG_CONFIG_HOME='{}' XDG_STATE_HOME='{}' \
         script -qec 'stty rows 20 cols 76; {bin}' '{}' >/dev/null 2>&1",
        cfg.display(),
        state.display(),
        out.display(),
    );
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .current_dir(d.path())
        .status()
        .expect("sh must run");
    assert!(status.success(), "the binary must exit cleanly");
    let raw = std::fs::read_to_string(&out).unwrap_or_default();

    assert!(
        raw.contains("already-here"),
        "the startup walk found the file that was there"
    );
    assert!(
        raw.contains("written-later"),
        "a file created while the list was up must arrive without a restart"
    );
    assert!(raw.contains("\x1b[?1049l"), "and it exits cleanly");
}

#[test]
fn the_reader_wears_the_desktop_palette_and_follows_it_when_it_changes() {
    if !script_available() {
        eprintln!("SKIP: `script`(1) not available — pty smoke not run");
        return;
    }
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("doc.md"), "# Heading\n\nbody\n").unwrap();

    // A desktop palette in the scratch state dir. `XDG_STATE_HOME` is what
    // the harness already redirects, so this can never see — or disturb —
    // the real one.
    let theme_dir = d.path().join("state/omarchy/current/theme");
    std::fs::create_dir_all(&theme_dir).unwrap();
    let colors = theme_dir.join("colors.toml");
    std::fs::write(
        &colors,
        "background = \"#0e091d\"\nforeground = \"#dc8f7c\"\naccent = \"#6e6080\"\n",
    )
    .unwrap();
    // What `omarchy theme set` will look like from in here.
    std::fs::write(
        d.path().join("next.toml"),
        "background = \"#102030\"\nforeground = \"#e0e0e0\"\naccent = \"#40c0a0\"\n",
    )
    .unwrap();

    let bin = env!("CARGO_BIN_EXE_carrel");
    let out = d.path().join("pty-capture");
    let cfg = d.path().join("cfg");
    let state = d.path().join("state");
    // The subshell swaps the palette a beat after the reader is up, then
    // gives the once-a-second poll time to notice before quitting.
    let cmd = format!(
        "( sleep 2; cp next.toml '{}'; sleep 3; printf 'q' ) | \
         XDG_CONFIG_HOME='{}' XDG_STATE_HOME='{}' \
         script -qec 'stty rows 20 cols 76; {bin} doc.md' '{}' >/dev/null 2>&1",
        colors.display(),
        cfg.display(),
        state.display(),
        out.display(),
    );
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .current_dir(d.path())
        .status()
        .expect("sh must run");
    assert!(status.success(), "the binary must exit cleanly");
    let raw = std::fs::read_to_string(&out).unwrap_or_default();

    // No `theme` in the (scratch, empty) config, so the desktop's palette is
    // what a fresh reader opens wearing.
    assert!(
        raw.contains("48;2;14;9;29"),
        "the page takes the desktop's background (#0e091d)"
    );
    assert!(
        raw.contains("38;2;110;96;128"),
        "and the heading its accent (#6e6080)"
    );
    assert!(
        !raw.contains("38;2;122;168;116"),
        "carrel's house green must not be painted over a desktop that never \
         asked for it"
    );

    // And it followed the swap without a restart.
    assert!(
        raw.contains("48;2;16;32;48"),
        "the new background (#102030) arrived while the reader was up"
    );
    assert!(
        raw.contains("38;2;64;192;160"),
        "and the new accent (#40c0a0) with it"
    );
    assert!(raw.contains("\x1b[?1049l"), "and it exits cleanly");
}

// --- CLI surface (2026-08-15) ---

#[test]
fn version_flag_prints_the_version_and_exits_zero() {
    for flag in ["--version", "-V"] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_carrel"))
            .arg(flag)
            .output()
            .expect("run carrel");
        assert!(
            out.status.success(),
            "{flag} must exit 0, got {:?}",
            out.status
        );
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(
            text.contains(env!("CARGO_PKG_VERSION")),
            "{flag} prints the version, got {text:?}"
        );
    }
}

/// The man page names every reader key by hand, so it rots the same way the
/// flags would.
///
/// **The flag guard below did not cover this**, and four keys — `F`, `y`,
/// `] [` and `L` — reached a release undocumented before anyone noticed
/// (2026-08-21). The help overlay had them, because a compile-time exhaustive
/// match forces that; the man page has no such compiler.
#[test]
fn the_man_page_documents_every_key_the_help_overlay_does() {
    // Rows the man page deliberately does not carry: the mouse gestures, which
    // it covers as prose under MOUSE. A NEW key is in neither list and fails
    // until someone decides which it is.
    const NOT_IN_MAN: &[&str] = &["drag", "2× click", "wheel", "click", "double-click"];

    let man = std::fs::read_to_string("../../contrib/carrel.1").expect("man page");
    // Section headers (`§`) are grouping, and prose rows like "double-click"
    // are gestures, not keys — the same exemption the honesty test makes.
    // Keys appear as `.B x` or, for a row of them, `.BR j ", " k ", " …`.
    // Strip roff and collect every token either macro names.
    let mut named: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for line in man.lines() {
        let Some(rest) = line
            .strip_prefix(".B ")
            .or_else(|| line.strip_prefix(".BR "))
        else {
            continue;
        };
        // The `", "` runs separate keys inside one .BR line; a lone quote is
        // now also a key (`"` lists bookmarks), so only the separator form
        // may be eaten.
        let cleaned = rest
            .replace("\\-", "-")
            .replace('\\', "")
            .replace("\", \"", " ");
        for tok in cleaned.split([' ', ',']) {
            let t = tok.trim();
            if !t.is_empty() {
                named.insert(t.to_string());
            }
        }
        named.insert(cleaned.trim().to_string());
    }

    let mut missing: Vec<&str> = Vec::new();
    for (key, _) in carrel::keys::READER_HELP {
        if *key == "§" || NOT_IN_MAN.contains(key) {
            continue;
        }
        // A row like "gg G Home End" is documented if any of its keys is.
        if key.split_whitespace().any(|k| named.contains(k)) || named.contains(*key) {
            continue;
        }
        missing.push(key);
    }
    assert!(
        missing.is_empty(),
        "contrib/carrel.1 documents no entry for: {missing:?}"
    );
}

/// The completions and the man page name every flag by hand. If a flag is
/// added to USAGE and not to them they rot silently, so assert the sets match.
#[test]
fn completions_and_man_page_cover_every_flag_in_usage() {
    let main_rs = std::fs::read_to_string("src/main.rs").expect("main.rs");
    let usage = main_rs
        .split("const USAGE: &str = \"\\\n")
        .nth(1)
        .and_then(|s| s.split("\";").next())
        .expect("USAGE literal");
    let flags: std::collections::BTreeSet<String> = usage
        .split_whitespace()
        .filter(|w| w.starts_with("--") && w.len() > 2)
        .map(|w| {
            w.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-')
                .to_string()
        })
        .collect();
    assert!(
        flags.contains("--version") && flags.contains("--plain") && flags.contains("--help"),
        "the scraper found the real flag set, got {flags:?}"
    );
    for shell in ["bash", "zsh", "fish"] {
        let path = format!("../../contrib/completions/carrel.{shell}");
        let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        for flag in &flags {
            // fish declares long options bare: `complete -l help`, no dashes.
            let bare = flag.trim_start_matches('-');
            let found = if shell == "fish" {
                src.contains(&format!("-l {bare}"))
            } else {
                src.contains(flag)
            };
            assert!(found, "{shell} completion is missing {flag}");
        }
    }
    let man = std::fs::read_to_string("../../contrib/carrel.1").expect("man page");
    for flag in &flags {
        // roff escapes the leading dashes as `\-\-`.
        let escaped = flag.replace('-', "\\-");
        assert!(
            man.contains(flag) || man.contains(&escaped),
            "man page is missing {flag}"
        );
    }
}

/// The whole conformance corpus through a real terminal.
///
/// The unit tests reach the renderer directly; this proves the binary can
/// open a document containing every construct at once — frontmatter, tables,
/// alerts, math art, wikilinks — and still leave the alternate screen. A
/// stuck terminal is the worst bug a TUI can ship.
#[test]
fn the_conformance_corpus_survives_a_pty_round_trip() {
    if !script_available() {
        eprintln!("SKIP: `script`(1) not available — pty smoke not run");
        return;
    }
    let d = tempfile::tempdir().unwrap();
    std::fs::copy(
        "tests/corpus/conformance.md",
        d.path().join("conformance.md"),
    )
    .expect("copy corpus");

    let raw = pty_run("conformance.md", "q", d.path());

    assert!(
        raw.contains("\x1b[?1049h"),
        "must ENTER the alternate screen"
    );
    assert!(
        raw.contains("\x1b[?1049l"),
        "must LEAVE the alternate screen"
    );
    assert!(
        raw.contains("Conformance"),
        "the corpus must actually paint"
    );
    assert!(
        raw.contains('╭'),
        "the frontmatter card paints on the first screen"
    );
}
