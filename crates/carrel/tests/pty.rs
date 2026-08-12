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
