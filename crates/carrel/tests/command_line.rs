//! The command line at its edges: arguments that are not UTF-8, what `--`
//! protects, which pager flags are tolerated, where config and state live,
//! and files that are not files where carrel expects its own. Every case
//! here was reproduced as a real failure before it was fixed.
//!
//! Half of it needs no terminal and drives the binary with pipes; the rest
//! runs it in a pty `script`(1) allocates, as `tests/pty.rs` does.
#![cfg(unix)]

use std::ffi::{OsStr, OsString};
use std::fmt::Write as _;
use std::io::Write as _;
use std::os::unix::ffi::OsStrExt as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

/// The binary on `args` in `dir`, `input` piped in, `HOME` and every XDG
/// directory under `dir/home` unless `env` says otherwise. Killed, with
/// `code` left `None`, if it has not exited in twenty seconds.
fn run_env(dir: &Path, args: &[&OsStr], input: &[u8], env: &[(&str, &OsStr)]) -> Run {
    let home = dir.join("home");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_carrel"));
    cmd.args(args)
        .current_dir(dir)
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_STATE_HOME", home.join("state"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("binary runs");
    // A closed pipe is fine: some modes never read stdin.
    let _ = child.stdin.take().expect("piped stdin").write_all(input);
    let started = Instant::now();
    let mut hung = false;
    while child.try_wait().expect("wait").is_none() {
        if started.elapsed() > Duration::from_secs(20) {
            hung = true;
            let _ = child.kill();
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let out = child.wait_with_output().expect("binary exits");
    Run {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        code: if hung { None } else { out.status.code() },
    }
}

fn run(dir: &Path, args: &[&OsStr], input: &[u8]) -> Run {
    run_env(dir, args, input, &[])
}

fn os(s: &str) -> &OsStr {
    OsStr::new(s)
}

fn raw(b: &[u8]) -> &OsStr {
    OsStr::from_bytes(b)
}

/// `std::env::args` panics on an argument that is not UTF-8, so a file with
/// such a name could not be named at all: exit 101 and a Rust backtrace.
#[test]
fn a_file_name_that_is_not_utf8_opens_and_is_named_escaped() {
    let d = tempfile::tempdir().unwrap();
    let name = raw(b"caf\xe9.md");
    if std::fs::write(d.path().join(name), "# Latin\n\nhello there\n").is_err() {
        return; // a filesystem that insists on UTF-8 names
    }
    let r = run(d.path(), &[name], b"");
    assert_eq!(r.code, Some(0), "{}", r.stderr);
    assert!(r.stdout.contains("hello there"), "{}", r.stdout);

    // A place in it is still a place, and the report names it escaped.
    let r = run(d.path(), &[raw(b"caf\xe9.md:3"), os("hello")], b"");
    assert_eq!(r.code, Some(0), "{}", r.stderr);
    assert!(r.stdout.contains(r"caf\xe9.md"), "{}", r.stdout);

    for mode in ["--plain", "--render"] {
        let r = run(d.path(), &[os(mode), name], b"");
        assert_eq!(r.code, Some(0), "{mode}: {}", r.stderr);
        assert!(r.stdout.contains("hello there"), "{mode}: {}", r.stdout);
    }

    // One that is not there is an ordinary "no such file".
    let r = run(d.path(), &[raw(b"gone\xff.md")], b"");
    assert_eq!(r.code, Some(1), "{}", r.stderr);
    assert!(
        r.stderr.contains(r"gone\xff.md: there is no such file"),
        "{}",
        r.stderr
    );
    let r = run(d.path(), &[os("--latest"), raw(b"gone\xff")], b"");
    assert_eq!(r.code, Some(1), "{}", r.stderr);
    assert!(
        r.stderr.contains(r"gone\xff is not a folder"),
        "{}",
        r.stderr
    );
}

#[test]
fn a_pattern_width_or_option_that_is_not_utf8_is_refused_by_name() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("t.md"), "# T\n\nhello\n").unwrap();
    for (args, says) in [
        (&[os("t.md"), raw(b"\xff")][..], "not a pattern"),
        (&[os("-"), raw(b"h\xff")][..], "not a pattern"),
        (
            &[os("--plain"), os("t.md"), raw(b"7\xff")][..],
            "not a width",
        ),
        (&[raw(b"--pl\xffain")][..], "unknown option"),
    ] {
        let r = run(d.path(), args, b"# piped\n");
        assert_eq!(r.code, Some(1), "{args:?}: {}", r.stderr);
        assert!(r.stderr.contains(says), "{args:?}: {}", r.stderr);
        assert!(r.stderr.contains(r"\xff"), "{args:?}: {}", r.stderr);
        assert!(!r.stderr.contains("panicked"), "{args:?}: {}", r.stderr);
    }
}

/// An unknown option was echoed as typed, so an ESC in it reached the
/// terminal: `carrel $'--x\e[2J'` cleared the screen.
#[test]
fn an_argument_in_an_error_is_never_echoed_raw() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("t.md"), "# T\n\nhello\n").unwrap();
    for args in [
        &[os("--x\u{1b}]0;T\u{7}\u{1b}[2J")][..],
        // Near enough to `--plain` to be offered it.
        &[os("--plain\u{1b}")][..],
        &[os("--plain"), os("t.md"), os("7\u{1b}[2J")][..],
        &[os("t.md"), os("\u{1b}[2Jnothing")][..],
        &[os("no\u{1b}[2J.md")][..],
        &[os("--latest"), os("no\u{1b}[2J")][..],
    ] {
        let r = run(d.path(), args, b"");
        assert_eq!(r.code, Some(1), "{args:?}: {}", r.stderr);
        for (what, text) in [("stderr", &r.stderr), ("stdout", &r.stdout)] {
            assert!(
                !text.contains(['\u{1b}', '\u{7}']),
                "{args:?} wrote a control character to {what}: {text:?}"
            );
        }
        // Stripping the argument would pass the check above too.
        assert!(
            format!("{}{}", r.stderr, r.stdout).contains(r"\x1b")
                || format!("{}{}", r.stderr, r.stdout).contains(r"\u{1b}"),
            "{args:?} must still be named: {}{}",
            r.stderr,
            r.stdout
        );
    }
    let r = run(d.path(), &[os("--plain\u{1b}")], b"");
    assert!(r.stderr.contains("did you mean --plain?"), "{}", r.stderr);
}

/// The flags pulled out wherever they stand, and the pager flags dropped on
/// a pipe, were both taken from the whole line — before `--` was looked for.
#[test]
fn nothing_after_a_double_dash_is_an_option_without_a_terminal() {
    let d = tempfile::tempdir().unwrap();
    for name in ["-weird.md", "--no-mouse", "--diff", "-R", "-", "--help"] {
        std::fs::write(
            d.path().join(name),
            format!("# name\n\nfile{} w\n", name.len()),
        )
        .unwrap();
    }
    for name in ["-weird.md", "--no-mouse", "--diff", "-R", "-", "--help"] {
        // Not the name itself: smart punctuation respells `--`.
        let body = format!("file{} w", name.len());
        let r = run(d.path(), &[os("--"), os(name)], b"");
        assert_eq!(r.code, Some(0), "-- {name}: {}", r.stderr);
        assert!(r.stdout.contains(&body), "-- {name}: {}", r.stdout);

        let r = run(d.path(), &[os("--"), os(name), os("w")], b"");
        assert_eq!(r.code, Some(0), "-- {name} w: {}", r.stderr);
        assert!(r.stdout.contains("match(es)"), "-- {name} w: {}", r.stdout);

        for mode in ["--plain", "--render"] {
            let r = run(d.path(), &[os(mode), os("--"), os(name)], b"# stdin\n");
            assert_eq!(r.code, Some(0), "{mode} -- {name}: {}", r.stderr);
            assert!(r.stdout.contains(&body), "{mode} -- {name}: {}", r.stdout);
            let r = run(d.path(), &[os(mode), os("--"), os(name), os("30")], b"");
            assert_eq!(r.code, Some(0), "{mode} -- {name} 30: {}", r.stderr);
            let r = run(d.path(), &[os(mode), os("--"), os(name), os("x")], b"");
            assert_eq!(r.code, Some(1), "{mode} -- {name} x");
            assert!(r.stderr.contains("not a width"), "{}", r.stderr);
        }
    }
    // An option still has to be one, and still comes before the `--`.
    let r = run(d.path(), &[os("--plian"), os("--"), os("-weird.md")], b"");
    assert_eq!(r.code, Some(1));
    assert!(r.stderr.contains("did you mean --plain?"), "{}", r.stderr);
    // `--diff` before it is still the option.
    let r = run(d.path(), &[os("--diff"), os("--"), os("-weird.md")], b"");
    assert_eq!(r.code, Some(0), "{}", r.stderr);
}

/// Whatever a tool hands its pager is ignored on a pipe: a pager that
/// refuses a flag shows nothing of what was piped to it. Only single-dash
/// flags; a `--long` option is carrel's own and a typo in one is an error.
#[test]
fn a_pagers_flags_are_ignored_on_a_pipe_and_a_long_option_is_not() {
    let d = tempfile::tempdir().unwrap();
    for flags in [
        &["-R"][..],
        &["-FRX"][..],
        &["-F", "-R", "-X"][..],
        &["-FRSXMK"][..],
        &["-N"][..],
        &["-x4"][..],
        &["-+F"][..],
        &["-z"][..],
    ] {
        let args: Vec<&OsStr> = flags.iter().map(|f| os(f)).collect();
        let r = run(d.path(), &args, b"# piped\n\ntext\n");
        assert_eq!(r.code, Some(0), "{flags:?}: {}", r.stderr);
        assert!(r.stdout.contains("piped"), "{flags:?}: {}", r.stdout);
    }
    for flag in ["--plian", "--zzz"] {
        let r = run(d.path(), &[os(flag)], b"# piped\n\ntext\n");
        assert_eq!(r.code, Some(1), "{flag} was accepted: {}", r.stdout);
        assert!(r.stderr.contains("unknown option"), "{flag}: {}", r.stderr);
    }
    // Carrel's own two short options are not a pager's.
    let r = run(d.path(), &[os("-V")], b"# piped\n");
    assert!(r.stdout.starts_with("carrel "), "{}", r.stdout);
    let r = run(d.path(), &[os("-h")], b"# piped\n");
    assert!(r.stdout.contains("USAGE:"), "{}", r.stdout);
    // A negative number is a width typed wrong, never a pager's flag.
    std::fs::write(d.path().join("t.md"), "# t\n").unwrap();
    let r = run(d.path(), &[os("--plain"), os("t.md"), os("-5")], b"");
    assert_eq!(r.code, Some(1), "{}", r.stdout);
}

/// The modes that print a pipe and exit read all of it, whatever its size,
/// while the reader stopped at its limit: `yes '# x' | carrel --plain -` ran
/// out of memory. Thirty million headings is far past what fits in the
/// memory allowed here, and a bounded stand-in for the endless pipe.
#[test]
fn a_pipe_past_the_limit_is_cut_in_the_modes_that_print() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path().join("home");
    let out = Command::new("sh")
        .arg("-c")
        .arg(format!(
            "ulimit -c 1; ulimit -v 4000000; yes '# x' | head -n 30000000 | \
             timeout 300 '{}' --plain - | wc -l",
            env!("CARGO_BIN_EXE_carrel")
        ))
        .current_dir(d.path())
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_STATE_HOME", home.join("state"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .output()
        .expect("sh runs");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let lines: usize = String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(0);
    assert!(!stderr.contains("memory"), "{stderr}");
    assert!(
        stderr.contains("carrel: stopped reading at 2,000,000 lines"),
        "the cut must be said on stderr: {stderr}"
    );
    // One heading and one blank line for each line read.
    assert!((1_999_999..=4_000_001).contains(&lines), "{lines} lines");
}

/// The same cut, said the same way, for the search report.
#[test]
fn a_cut_pipe_is_reported_by_every_mode_that_prints() {
    let d = tempfile::tempdir().unwrap();
    let input = "word\n".repeat(2_000_500);
    let r = run(
        d.path(),
        &[os("-"), os("nothing-matches")],
        input.as_bytes(),
    );
    assert!(
        r.stderr
            .contains("carrel: stopped reading at 2,000,000 lines"),
        "{}",
        r.stderr
    );
    assert_eq!(r.code, Some(1), "no match is still exit 1");
    // Under the limit nothing is said.
    let r = run(d.path(), &[], b"word\n");
    assert_eq!((r.code, r.stderr.as_str()), (Some(0), ""));
}

fn mkfifo(path: &Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    assert!(
        Command::new("mkfifo")
            .arg(path)
            .status()
            .expect("mkfifo runs")
            .success()
    );
}

fn script_available() -> bool {
    Command::new("script")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// What the program wrote to the pty, without `script`'s own header line —
/// which echoes the command, and so anything the test typed into it.
fn captured(out: &Path) -> String {
    let raw = String::from_utf8_lossy(&std::fs::read(out).unwrap_or_default()).into_owned();
    match raw.strip_prefix("Script started") {
        Some(rest) => rest
            .split_once('\n')
            .map_or(String::new(), |(_, r)| r.to_string()),
        None => raw,
    }
}

/// Run `carrel ARGS` in a real pty, 20 rows by 76 columns, in `dir`. `keys`
/// are typed after a beat, and `then` once `until` has appeared in what the
/// program wrote (or ten seconds have passed). Returns whether it exited 0
/// inside twenty seconds, and what it wrote.
fn pty(dir: &Path, args: &str, keys: &str, until: &str, then: &str) -> (bool, String) {
    pty_env(dir, "", args, keys, until, then)
}

/// [`pty`], with `env` (`A=b C=d`) set after the scratch directories.
fn pty_env(
    dir: &Path,
    env: &str,
    args: &str,
    keys: &str,
    until: &str,
    then: &str,
) -> (bool, String) {
    let bin = env!("CARGO_BIN_EXE_carrel");
    let out = dir.join("pty-capture");
    let home = dir.join("home");
    let cmd = format!(
        "( sleep 1; printf '{keys}'; i=0; \
           while [ $i -lt 100 ] && ! grep -qF '{until}' '{out}' 2>/dev/null; do \
             sleep 0.1; i=$((i+1)); done; \
           printf '{then}' ) | \
         XDG_CONFIG_HOME='{h}/config' XDG_STATE_HOME='{h}/state' XDG_CACHE_HOME='{h}/cache' \
         HOME='{h}' {env} \
         timeout 20 script -fqec 'stty rows 20 cols 76; {bin} {args}' '{out}' >/dev/null 2>&1",
        out = out.display(),
        h = home.display(),
    );
    let status = Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .current_dir(dir)
        .status()
        .expect("sh must run");
    (status.success(), captured(&out))
}

/// The XDG specification calls a relative base directory invalid. Honored,
/// it made the folder being read the place carrel keeps its own files: a
/// config planted there was obeyed, and positions and the index were
/// written into it.
#[test]
fn a_relative_xdg_directory_is_ignored_not_resolved_against_the_folder() {
    if !script_available() {
        eprintln!("SKIP: `script` (util-linux) not found");
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let mut doc = String::from("# Long\n\n");
    for i in 0..200 {
        let _ = write!(doc, "paragraph {i}\n\n");
    }
    std::fs::write(d.path().join("a.md"), doc).unwrap();
    let relative = "XDG_CONFIG_HOME=planted XDG_STATE_HOME=planted XDG_CACHE_HOME=./planted";
    // Read far enough for a position to be worth remembering.
    let (ok, screen) = pty_env(d.path(), relative, "a.md", " ", "paragraph 20", "q");
    assert!(ok, "{screen}");
    // The file list writes the index cache, and `T` the config.
    let (ok, screen) = pty_env(d.path(), relative, ".", "T", "theme:", "q");
    assert!(ok, "{screen}");
    assert!(
        !d.path().join("planted").exists(),
        "carrel wrote into the folder it was reading"
    );
    let home = d.path().join("home");
    assert!(home.join(".local/state/carrel/positions").is_file());
    assert!(home.join(".config/carrel/config").is_file());
    assert!(home.join(".cache/carrel").is_dir());
}

/// At a terminal the same rules hold: `--` protects a name, an unknown
/// option is an error whether or not it looks like a pager's.
#[test]
fn the_double_dash_and_unknown_options_behave_the_same_at_a_terminal() {
    if !script_available() {
        eprintln!("SKIP: `script` (util-linux) not found");
        return;
    }
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("-weird.md"), "# Dashed\n\nbodytext\n").unwrap();

    let (ok, screen) = pty(d.path(), "-- -weird.md", "", "bodytext", "q");
    assert!(ok, "the reader must open and close: {screen}");
    assert!(screen.contains("\u{1b}[?1049h") && screen.contains("bodytext"));

    let (ok, screen) = pty(d.path(), "--plain -- -weird.md", "", "bodytext", "");
    assert!(ok && screen.contains("bodytext"), "{screen}");

    for flag in ["-R", "-z"] {
        let (ok, screen) = pty(d.path(), flag, "", "unknown option", "");
        assert!(!ok, "{flag} at a terminal is a typo, not a pager's flag");
        assert!(
            screen.contains(&format!("unknown option {flag}")),
            "{screen}"
        );
        assert!(!screen.contains("\u{1b}[?1049h"), "no reader opened");
    }
}

/// open(2) on a FIFO blocks until a writer comes. One at the config path, or
/// where a state file should be, held startup on a blank screen that no key
/// could leave.
#[test]
fn a_fifo_where_config_or_state_should_be_does_not_hang_startup() {
    if !script_available() {
        eprintln!("SKIP: `script` (util-linux) not found");
        return;
    }
    for place in [
        "config/carrel/config",
        "state/carrel/positions",
        "state/carrel/bookmarks",
        "state/carrel/folds",
    ] {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.md"), "# Fifo\n\nreadable\n").unwrap();
        mkfifo(&d.path().join("home").join(place));
        let (ok, screen) = pty(d.path(), "a.md", "", "readable", "jq");
        assert!(ok, "{place}: the reader hung or failed: {screen}");
        assert!(screen.contains("readable"), "{place}: {screen}");

        // The file list reads config, positions and the index cache.
        let (ok, screen) = pty(d.path(), ".", "", "a.md", "q");
        assert!(ok, "{place}: the file list hung or failed: {screen}");
        assert!(screen.contains("a.md"), "{place}: {screen}");
    }

    // And without a terminal: the listing reads the saved root.
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("a.md"), "# Fifo\n").unwrap();
    mkfifo(&d.path().join("home/config/carrel/config"));
    let r = run(d.path(), &[os(".")], b"");
    assert_eq!(r.code, Some(0), "hung or failed: {}", r.stderr);
}

/// The settings pane read `Theme  terminal` under every theme: the name it
/// shows was a field nothing ever assigned.
#[test]
fn the_settings_pane_names_the_theme_that_is_showing() {
    if !script_available() {
        eprintln!("SKIP: `script` (util-linux) not found");
        return;
    }
    let names: Vec<&str> = carrel::theme::PALETTES.iter().map(|p| p.name).collect();
    let nord = names.iter().position(|n| *n == "nord").expect("nord");
    let next = names[(nord + 1) % names.len()];
    let config = |d: &Path| {
        std::fs::create_dir_all(d.join("home/config/carrel")).unwrap();
        std::fs::write(d.join("home/config/carrel/config"), "theme = nord\n").unwrap();
        std::fs::write(d.join("a.md"), "# Themed\n\nbody text\n").unwrap();
    };

    // From the config, at startup.
    let d = tempfile::tempdir().unwrap();
    config(d.path());
    let (ok, screen) = pty(d.path(), "a.md", ",", "Theme", "qq");
    assert!(ok, "{screen}");
    assert!(screen.contains("Theme"), "the pane must open: {screen}");
    assert!(screen.contains("nord"), "the row must name nord: {screen}");

    // After `T`: once in the status note, and once on the row.
    let d = tempfile::tempdir().unwrap();
    config(d.path());
    let (ok, screen) = pty(d.path(), "a.md", "T,", "Theme", "qq");
    assert!(ok, "{screen}");
    assert_eq!(
        screen.matches(next).count(),
        2,
        "`{next}` in the note and on the row: {screen}"
    );

    // From the row itself: Enter on it changes the theme and the row.
    let d = tempfile::tempdir().unwrap();
    config(d.path());
    let (ok, screen) = pty(d.path(), "a.md", ",\\r", next, "qq");
    assert!(ok, "{screen}");
    assert_eq!(
        screen.matches(next).count(),
        2,
        "`{next}` in the note and on the row: {screen}"
    );

    // And on the file list.
    let d = tempfile::tempdir().unwrap();
    config(d.path());
    let (ok, screen) = pty(d.path(), ".", ",", "Theme", "qq");
    assert!(ok, "{screen}");
    assert!(screen.contains("nord"), "the row must name nord: {screen}");
}

/// A non-UTF-8 argument must not take the reader down either.
#[test]
fn a_file_name_that_is_not_utf8_opens_in_the_reader() {
    if !script_available() {
        eprintln!("SKIP: `script` (util-linux) not found");
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let name = OsString::from(raw(b"caf\xe9.md"));
    if std::fs::write(d.path().join(&name), "# Latin\n\nhello there\n").is_err() {
        return;
    }
    // The shell expands the glob to the one name, bytes intact.
    let (ok, screen) = pty(d.path(), "caf*.md", "", "hello", "q");
    assert!(ok, "{screen}");
    assert!(screen.contains("hello"), "{screen}");
}

/// The man page listed "nine keys" above twelve, and its synopsis lacked an
/// option the binary had. Prose has no compiler, so: every key the config
/// parser reads has an entry in the man page's list and a row in the
/// README's table, and every option has a line in the synopsis.
#[test]
fn the_man_page_and_readme_name_every_config_key_and_option() {
    let parser = std::fs::read_to_string("../carrel-state/src/config.rs").expect("config.rs");
    let body = parser
        .split("fn parse_all(")
        .nth(1)
        .and_then(|s| s.split("\nfn ").next())
        .expect("parse_all");
    // `"theme" if c.theme.is_none() =>` for most, `k == "place"` for the
    // one that repeats.
    let mut keys: Vec<&str> = body
        .split('"')
        .skip(1)
        .step_by(2)
        .zip(body.split('"').skip(2).step_by(2))
        .filter(|(_, after)| after.starts_with(" if c.") || after.starts_with(" {"))
        .map(|(key, _)| key)
        .collect();
    keys.sort_unstable();
    keys.dedup();
    assert!(
        keys.len() >= 12 && keys.contains(&"place") && keys.contains(&"max_width"),
        "the scraper found the real key set, got {keys:?}"
    );
    let man = std::fs::read_to_string("../../contrib/carrel.1").expect("man page");
    let listed = man
        .split("The keys are\n.RS\n")
        .nth(1)
        .and_then(|s| s.split("\n.RE\n").next())
        .expect("the config key list in FILES");
    let readme = std::fs::read_to_string("../../README.md").expect("README");
    for key in &keys {
        assert!(
            listed.contains(&format!(".TP\n.B {key}\n")),
            "contrib/carrel.1 FILES has no entry for the config key `{key}`"
        );
        assert!(
            readme.contains(&format!("\n| `{key}` |")),
            "README's configuration table has no row for `{key}`"
        );
    }

    let main_rs = std::fs::read_to_string("src/main.rs").expect("main.rs");
    let flags: Vec<&str> = main_rs
        .split("const KNOWN_FLAGS: &[&str] = &[")
        .nth(1)
        .and_then(|s| s.split("];").next())
        .expect("KNOWN_FLAGS")
        .split('"')
        .skip(1)
        .step_by(2)
        .filter(|f| f.starts_with("--"))
        .collect();
    assert!(
        flags.contains(&"--plain") && flags.contains(&"--tutorial"),
        "{flags:?}"
    );
    let synopsis = man
        .split(".SH SYNOPSIS\n")
        .nth(1)
        .and_then(|s| s.split(".SH ").next())
        .expect("SYNOPSIS");
    for flag in flags {
        assert!(
            synopsis.contains(&flag.replace('-', "\\-")),
            "contrib/carrel.1 SYNOPSIS has no line for {flag}"
        );
    }
}
