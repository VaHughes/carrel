//! Every non-TUI path that prints text carrel did not write — a document's
//! lines, its task items, a file's NAME — must not hand the terminal a
//! control sequence. The TUI is safe because `ratatui::text::Span` drops
//! control characters; these paths write straight to stdout or stderr, and
//! each of the four below once let a markdown file (or a file merely sitting
//! in the folder) set the terminal title, clear the screen, or reset it.
//!
//! Driven through the binary with both ends piped: that is the shape an
//! agent or a script runs carrel in, and none of these paths needs a pty.

use std::path::Path;
use std::process::{Command, Output, Stdio};

/// The bytes that start or are a terminal control sequence: every C0
/// control but the line feed (ESC, BEL, BS, CR among them), DEL, and the
/// C1 range as UTF-8 (`U+0080..=U+009F` encodes as `C2 80..=C2 9F` — U+009B
/// is a one-character CSI to a terminal that honours C1).
fn controls_in(bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes);
    text.chars()
        .filter(|&c| c.is_control() && c != '\n')
        .map(|c| format!("U+{:04X}", c as u32))
        .collect()
}

fn run_in(dir: &Path, args: &[&str]) -> Output {
    let scratch = tempfile::tempdir().expect("scratch dir");
    Command::new(env!("CARGO_BIN_EXE_carrel"))
        .args(args)
        .current_dir(dir)
        .env("HOME", scratch.path())
        .env("XDG_CONFIG_HOME", scratch.path())
        .env("XDG_STATE_HOME", scratch.path())
        .env("XDG_CACHE_HOME", scratch.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("binary runs")
}

fn assert_clean(out: &Output, what: &str) {
    let o = controls_in(&out.stdout);
    let e = controls_in(&out.stderr);
    assert!(
        o.is_empty() && e.is_empty(),
        "{what}: control characters reached the terminal — stdout {o:?}, stderr {e:?}\n\
         stdout: {:?}\nstderr: {:?}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

const HOSTILE: &str = "# T\n\nHello world \u{1b}]0;PWNED\u{7} and \u{1b}[2J clear \u{9b}31m c1 \r\u{8} end\n\n- [ ] ta\u{1b}]0;X\u{7}sk \u{9b}2J\n- [x] do\u{1b}c\rne\n";

#[test]
fn the_pattern_report_strips_control_characters_from_document_lines() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("esc.md"), HOSTILE).unwrap();
    let out = run_in(dir.path(), &["esc.md", "world"]);
    assert!(out.status.success(), "{out:?}");
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(
        s.contains("Hello world"),
        "the hit line is still reported: {s}"
    );
    assert_clean(&out, "carrel FILE PATTERN");
}

#[test]
fn the_pattern_report_on_stdin_strips_control_characters() {
    use std::io::Write as _;
    let scratch = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_carrel"))
        .args(["-", "world"])
        .env("HOME", scratch.path())
        .env("XDG_CONFIG_HOME", scratch.path())
        .env("XDG_STATE_HOME", scratch.path())
        .env("XDG_CACHE_HOME", scratch.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(HOSTILE.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{out:?}");
    assert_clean(&out, "carrel - PATTERN");
}

#[test]
fn the_task_report_strips_control_characters_from_task_text() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("t.md"), HOSTILE).unwrap();
    let out = run_in(dir.path(), &["--tasks", "t.md"]);
    assert!(out.status.success(), "{out:?}");
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("- [ ] ta") && s.contains("- [x] do"), "{s}");
    assert_clean(&out, "carrel --tasks");
}

#[test]
fn the_piped_home_listing_escapes_hostile_file_names() {
    let dir = tempfile::tempdir().unwrap();
    for name in [
        "evil\u{1b}]0;OWNED\u{7}\u{1b}[31m.md",
        "notes\u{1b}[2J.md",
        "line\nbreak\tz.md",
        "c1\u{9b}2J.md",
        "bidi\u{202e}dm.txt.md",
    ] {
        std::fs::write(dir.path().join(name), "# x\n").unwrap();
    }
    let out = run_in(dir.path(), &["."]);
    assert!(out.status.success(), "{out:?}");
    let s = String::from_utf8_lossy(&out.stdout);
    assert_clean(&out, "carrel . | cat");
    // Escaped visibly, not dropped: two names that differ only in a control
    // character must still read as two names.
    assert!(s.contains(r"evil\x1b]0;OWNED\x07\x1b[31m.md"), "{s}");
    assert!(s.contains(r"line\x0abreak\x09z.md"), "{s}");
    assert!(s.contains(r"c1\u{9b}2J.md"), "{s}");
    assert!(
        !s.contains('\u{202e}') && s.contains(r"bidi\u{202e}dm.txt.md"),
        "a bidi override must not reorder the listing: {s}"
    );
}

#[test]
fn did_you_mean_escapes_the_name_it_suggests() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("README\u{1b}c.md"), "# x\n").unwrap();
    let out = run_in(dir.path(), &["README.md"]);
    assert!(!out.status.success());
    let e = String::from_utf8_lossy(&out.stderr);
    assert!(e.contains(r"did you mean README\x1bc.md?"), "{e}");
    assert_clean(&out, "did you mean");
}

#[test]
fn a_missing_hostile_name_is_escaped_in_the_error() {
    let dir = tempfile::tempdir().unwrap();
    let out = run_in(dir.path(), &["gone\u{1b}]0;T\u{7}.md"]);
    assert!(!out.status.success());
    assert_clean(&out, "the open error");
    let out = run_in(dir.path(), &["--tasks", "gone\u{1b}c.md"]);
    assert_clean(&out, "the --tasks error");
    let out = run_in(dir.path(), &["--plain", "gone\u{1b}c.md"]);
    assert_clean(&out, "the --plain error");
    let out = run_in(dir.path(), &["--latest", "gone\u{1b}c"]);
    assert_clean(&out, "the --latest error");
}
