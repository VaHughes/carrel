//! stdin-mode paths that need no terminal: plain render and the report.
//! The TUI half lives in the pty suite; these drive the binary with both
//! ends piped, which must never try to enter the alternate screen.

use std::io::Write;
use std::process::{Command, Stdio};

fn run_with_stdin(args: &[&str], input: &str) -> (String, bool) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_carrel"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("binary runs");
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(input.as_bytes())
        .expect("write the document");
    let out = child.wait_with_output().expect("binary exits");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.success(),
    )
}

#[test]
fn piped_in_and_out_renders_plain() {
    let (out, ok) = run_with_stdin(&[], "# Title\n\nbody text\n");
    assert!(ok);
    assert!(out.contains("Title") && out.contains("body text"), "{out}");
    assert!(!out.contains('\u{1b}'), "plain output carries no escapes");
}

#[test]
fn dash_is_the_same_document_explicitly() {
    let (a, _) = run_with_stdin(&[], "# Same\n\ntext\n");
    let (b, ok) = run_with_stdin(&["-"], "# Same\n\ntext\n");
    assert!(ok);
    assert_eq!(a, b);
}

#[test]
fn a_pattern_with_dash_prints_the_match_report() {
    let (out, ok) = run_with_stdin(&["-", "beta"], "alpha\n\nbeta gamma beta\n");
    assert!(ok);
    assert!(out.contains("beta"), "the report names the hits: {out}");
}

#[test]
fn plain_dash_reads_stdin_at_a_width() {
    let (out, ok) = run_with_stdin(
        &["--plain", "-", "30"],
        "# W\n\nword word word word word word word\n",
    );
    assert!(ok && out.contains("word"), "{out}");
    assert!(
        out.lines().all(|l| l.chars().count() <= 30),
        "respects the width: {out}"
    );
}

/// The binary on `args`, with `input` piped in, `NO_COLOR` as given, and
/// every XDG directory in a scratch dir.
fn run_linear(args: &[&str], input: &str, no_color: bool) -> String {
    let scratch = tempfile::tempdir().expect("scratch dir");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_carrel"));
    cmd.args(args)
        .env("XDG_CONFIG_HOME", scratch.path())
        .env("XDG_STATE_HOME", scratch.path())
        .env("XDG_CACHE_HOME", scratch.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if no_color {
        cmd.env("NO_COLOR", "1");
    } else {
        cmd.env_remove("NO_COLOR");
    }
    let mut child = cmd.spawn().expect("binary runs");
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(input.as_bytes())
        .expect("write the document");
    let out = child.wait_with_output().expect("binary exits");
    assert!(out.status.success(), "{args:?} failed");
    String::from_utf8(out.stdout).expect("utf-8 output")
}

/// `--plain -` and `--render -` lay out with the same code a file gets: an
/// over-long run stands alone, unmarked in plain and unstyled output, cut
/// with a faint mark in styled output — read from a pipe or from a path.
#[test]
fn piped_linear_output_keeps_the_width_contract_a_file_gets() {
    let hash = "0123456789abcdef0123456789abcdef0123456789abcdef";
    let src = format!("A hash {hash} and more words after it.\n");
    let doc = carrel_core::Document::parse(&src);
    let dir = tempfile::tempdir().expect("scratch dir");
    let file = dir.path().join("h.md");
    std::fs::write(&file, &src).expect("write the file");
    let file = file.to_str().expect("utf-8 path");

    let want = carrel::plain::render(&doc, 30);
    assert_eq!(want, format!("A hash\n{hash}\nand more words after it.\n"));
    for args in [
        &["--plain", "-", "30"][..],
        &["--plain", file, "30"],
        &["--render", "-", "30"],
        &["--render", file, "30"],
    ] {
        assert_eq!(
            run_linear(args, &src, true),
            want,
            "{args:?} under NO_COLOR"
        );
    }
    assert_eq!(run_linear(&["--plain", "-", "30"], &src, false), want);

    let styled = carrel::ansi::render_with(&doc, 30, true);
    assert!(styled.contains("\u{1b}[2m↩\u{1b}[0m"), "{styled:?}");
    for args in [&["--render", "-", "30"][..], &["--render", file, "30"]] {
        assert_eq!(run_linear(args, &src, false), styled, "{args:?}");
    }
}
