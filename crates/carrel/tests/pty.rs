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
        let cleaned = rest.replace("\\-", "-").replace('\\', "").replace('"', " ");
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
