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
