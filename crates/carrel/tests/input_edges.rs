//! What the binary does with awkward input: bytes that are not quite UTF-8,
//! and command lines at the edge of what `--help` promises. Every case here
//! was reproduced as a real failure before it was fixed.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

struct Run {
    stdout: String,
    stderr: String,
    ok: bool,
}

/// The binary on `args` in `dir`, `input` piped in, every XDG directory and
/// `HOME` in a scratch dir.
fn run(dir: &Path, args: &[&str], input: &[u8]) -> Run {
    let scratch = tempfile::tempdir().expect("scratch dir");
    let mut child = Command::new(env!("CARGO_BIN_EXE_carrel"))
        .args(args)
        .current_dir(dir)
        .env("HOME", scratch.path())
        .env("XDG_CONFIG_HOME", scratch.path())
        .env("XDG_STATE_HOME", scratch.path())
        .env("XDG_CACHE_HOME", scratch.path())
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("binary runs");
    // A closed pipe is fine: some modes never read stdin.
    let _ = child.stdin.take().expect("piped stdin").write_all(input);
    let out = child.wait_with_output().expect("binary exits");
    Run {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        ok: out.status.success(),
    }
}

const LATIN1: &[u8] = b"# caf\xe9 latin1\n\nna\xefve text\n";

/// Rust's own words for `read_to_string` on bad bytes, which reached the
/// screen verbatim from every non-TUI path.
const RAW_IO: &str = "stream did not contain valid UTF-8";

#[test]
fn a_stray_latin1_byte_reads_through_every_piped_mode() {
    let d = tempfile::tempdir().unwrap();
    for args in [
        &["--plain", "-"][..],
        &["-"][..],
        &[][..],
        &["--render", "-"][..],
        &["-", "caf"][..],
    ] {
        let r = run(d.path(), args, LATIN1);
        assert!(r.ok, "{args:?} failed: {}", r.stderr);
        assert!(!r.stderr.contains(RAW_IO), "{args:?}: {}", r.stderr);
        assert!(
            r.stdout.contains('\u{FFFD}') || args.contains(&"caf"),
            "{args:?} shows the stray byte as U+FFFD: {}",
            r.stdout
        );
    }
}

#[test]
fn a_stray_latin1_byte_in_a_file_reads_and_a_binary_is_refused_plainly() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("old.md"), LATIN1).unwrap();
    for args in [
        &["--plain", "old.md"][..],
        &["--render", "old.md"][..],
        &["old.md"][..],
    ] {
        let r = run(d.path(), args, b"");
        assert!(r.ok, "{args:?} failed: {}", r.stderr);
        assert!(r.stdout.contains("caf\u{FFFD}"), "{args:?}: {}", r.stdout);
    }
    let r = run(d.path(), &["old.md", "text"], b"");
    assert!(r.ok, "the report searches it too: {}", r.stderr);

    std::fs::write(
        d.path().join("pic.md"),
        b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\xff",
    )
    .unwrap();
    for args in [&["--plain", "pic.md"][..], &["--render", "pic.md"][..]] {
        let r = run(d.path(), args, b"");
        assert!(!r.ok, "{args:?} must refuse a binary");
        assert!(r.stderr.contains("not text"), "{args:?}: {}", r.stderr);
        assert!(!r.stderr.contains(RAW_IO), "{args:?}: {}", r.stderr);
    }
}

/// `--help` promises exit 1 for a `[W]` that is not a column count. A
/// negative one was taken for a pager flag git might pass, dropped, and
/// the document rendered at 80 with exit 0.
#[test]
fn a_negative_width_is_refused_not_dropped() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("t.md"), "# T\n\nhello\n").unwrap();
    for w in ["-5", "-0"] {
        let r = run(d.path(), &["--plain", "t.md", w], b"");
        assert!(!r.ok, "{w} was accepted: {}", r.stdout);
        assert!(r.stderr.contains("not a width"), "{w}: {}", r.stderr);
    }
    // git's own pager flags are still ignored when stdin is a pipe.
    let r = run(d.path(), &["-R"], b"# piped\n");
    assert!(r.ok && r.stdout.contains("piped"), "{}", r.stderr);
}

#[test]
fn line_zero_is_the_top_of_a_file_that_exists() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("t.md"), "# T\n\nhello\n").unwrap();
    let r = run(d.path(), &["t.md:0", "hello"], b"");
    assert!(r.ok, "{}", r.stderr);
    assert!(!r.stderr.contains("no such file"), "{}", r.stderr);
}

#[test]
fn a_folder_with_a_pattern_is_called_a_folder() {
    let d = tempfile::tempdir().unwrap();
    let r = run(d.path(), &[".", "TODO"], b"");
    assert!(!r.ok);
    assert!(r.stderr.contains("folder"), "{}", r.stderr);
    assert!(!r.stderr.contains("pipe or device"), "{}", r.stderr);
}
