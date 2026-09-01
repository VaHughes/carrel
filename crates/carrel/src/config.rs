//! Persisted settings: the home screen's root, theme, footer and breadcrumb
//! preferences, the reading measure, titles — flat `key = value` lines —
//! plus `place`, the one key that legitimately REPEATS (favourite roots,
//! newest first; see [`add_place_in`]).
//!
//! NO RATATUI — `scripts/check-discipline.sh` rule 6.
//!
//! # Why this is not TOML
//!
//! `serde` + `toml` costs roughly 300 KB to store one string, in a project that
//! tracks `regex`'s 1.59 MB as its single largest binary cost. One
//! `key = value` per line needs thirty lines and no dependency.
//!
//! # The migration trigger, revised
//!
//! This used to read: **"past three or four keys, switch to TOML."** That
//! trigger has fired — there are eight — and it is still the wrong call,
//! because the number of keys was never what the cost depends on. What makes
//! a hand-rolled format ossify is *syntax*: nesting, lists, quoting, types,
//! comments that have to survive a rewrite. This format has none. Every key
//! is one line of `key = value`; values are a path, a name, a boolean or a
//! small integer; the only structure is that `place` may repeat, and that is
//! read as "the lines in order" rather than as a list literal. Eight of those
//! is not eight times the format, it is the same format eight times.
//!
//! So the trigger is restated as the shape it was always about. **Switch to
//! TOML the first time a value needs to be anything but a scalar** — a
//! per-theme override, a keybinding table, a list that must be written on one
//! line, or any value whose spelling needs quoting to survive. At that point
//! the 300 KB buys something. Until then it buys a dependency.
//!
//! Booleans accept `true`/`false`, `1`/`0` and `yes`/`no`, in any case; see
//! [`flag`], which is the ONE reading of a boolean this file has.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::state::write_atomic;

/// Every setting in the file, from ONE read of it.
///
/// `None` means the key is absent and the caller applies its own default; the
/// defaults are not baked in here because two of them (`max_width`, the
/// booleans) mean genuinely different things to different callers.
///
/// This exists because startup read the config file EIGHT times through six
/// near-identical wrappers, each one `read_to_string`-ing and re-scanning the
/// whole thing to answer a single key. That is not a measurable cost at this
/// size, and it is not the reason to fix it: six copies of "find a key, trim
/// it, decide what its value means" is six places for the readings to drift
/// apart, and they did — see [`flag`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Config {
    pub root: Option<PathBuf>,
    pub theme: Option<String>,
    pub hints: Option<bool>,
    pub breadcrumb: Option<bool>,
    pub outline_margin: Option<bool>,
    pub titles: Option<bool>,
    /// Already clamped by [`MIN_MEASURE`]; `Some(0)` still means OFF.
    pub max_width: Option<u16>,
    /// Newest first, deduped, capped at [`PLACE_CAP`].
    pub places: Vec<PathBuf>,
}

/// Read the whole config in one pass. A missing or unreadable file is
/// [`Config::default`] — every key absent, never an error.
#[must_use]
pub fn load_all_in(dir: &Path) -> Config {
    parse_all(&std::fs::read_to_string(dir.join("config")).unwrap_or_default())
}

/// `key = value` per line. Unknown keys and `#` comments are ignored, so a
/// newer version's file cannot break an older binary.
fn parse_all(text: &str) -> Config {
    let mut c = Config::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let (k, v) = (k.trim(), v.trim());
        if v.is_empty() {
            continue;
        }
        // `place` repeats and every line counts; every other key takes the
        // FIRST line that sets it, which is what `parse_key` did by returning
        // early and what an already-written file therefore expects.
        if k == "place" {
            let p = PathBuf::from(v);
            if c.places.len() < PLACE_CAP && !c.places.contains(&p) {
                c.places.push(p);
            }
            continue;
        }
        match k {
            "root" if c.root.is_none() => c.root = Some(PathBuf::from(v)),
            "theme" if c.theme.is_none() => c.theme = Some(v.to_string()),
            "hints" if c.hints.is_none() => c.hints = Some(flag(v, true)),
            "breadcrumb" if c.breadcrumb.is_none() => c.breadcrumb = Some(flag(v, true)),
            "outline_margin" if c.outline_margin.is_none() => {
                c.outline_margin = Some(flag(v, false));
            }
            "titles" if c.titles.is_none() => c.titles = Some(flag(v, false)),
            "max_width" if c.max_width.is_none() => {
                // Unparseable stays absent rather than becoming zero: zero is
                // a real setting here, meaning "no measure at all".
                c.max_width = v
                    .parse::<u16>()
                    .ok()
                    .map(|raw| if raw == 0 { 0 } else { raw.max(MIN_MEASURE) });
            }
            _ => {}
        }
    }
    c
}

/// The whole config from the real config directory, or all-absent when there
/// is no directory to read.
#[must_use]
pub fn load_all() -> Config {
    config_dir().map(|d| load_all_in(&d)).unwrap_or_default()
}

/// The ONE reading of a boolean value.
///
/// There were two. `hints` and `breadcrumb` took `v != "false"`, so anything
/// that was not the exact word `false` — including `0` — was ON. `titles` and
/// `outline_margin` took `v == "true"`, so anything that was not the exact
/// word `true` — including `1` — was OFF. The same file therefore answered
/// `breadcrumb = 1` and `titles = 1` differently, and neither spelling was
/// written down anywhere for a reader to check.
///
/// `true`/`false`, `1`/`0` and `yes`/`no` are accepted in any case. Anything
/// else is not an answer, so the key falls back to `default` — which is what
/// both old readings did by accident (`hints = wibble` was on, `titles =
/// wibble` was off, each its own default) and is worth keeping on purpose: a
/// typo should leave the reader's screen as they have always seen it.
fn flag(v: &str, default: bool) -> bool {
    match v.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" => true,
        "false" | "0" | "no" => false,
        _ => default,
    }
}

/// `$XDG_CONFIG_HOME/carrel`, else `~/.config/carrel`.
#[must_use]
pub fn config_dir() -> Option<PathBuf> {
    if let Some(x) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(x).join("carrel"));
    }
    std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(|h| PathBuf::from(h).join(".config").join("carrel"))
}

#[must_use]
pub fn load_root() -> Option<PathBuf> {
    load_root_in(&config_dir()?)
}

// There is deliberately no `save_root()` writing to the real directory: the
// picker persists through `App::config_dir`, which is `None` under test. The
// convenient wrapper was how `cargo test` once stomped the developer's real
// config with a tempdir on every run.

/// Read the saved root from an explicit config directory.
///
/// The `_in` variants exist so tests never touch environment variables or a
/// real home directory. Never fails: a missing, unreadable or malformed file
/// means "no saved root".
#[must_use]
pub fn load_root_in(dir: &Path) -> Option<PathBuf> {
    load_all_in(dir).root
}

pub fn save_root_in(dir: &Path, root: &Path) -> std::io::Result<()> {
    upsert_key_in(dir, "root", &root.display().to_string())
}

/// Write or replace ONE key, preserving every other line.
///
/// With more than one key, "write only mine" done naively destroys the
/// others — the original `save_root` rewrote the whole file.
/// Repeated keys (`place`) do not belong here; they have their own
/// append-and-promote path in [`add_place_in`].
fn upsert_key_in(dir: &Path, key: &str, value: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join("config");
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let mut out = String::new();
    let mut written = false;
    for line in existing.lines() {
        let is_this_key = line
            .trim()
            .split_once('=')
            .is_some_and(|(k, _)| k.trim() == key);
        if is_this_key {
            if !written {
                let _ = writeln!(out, "{key} = {value}");
                written = true;
            }
        } else if !line.trim().is_empty() {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !written {
        let _ = writeln!(out, "{key} = {value}");
    }
    write_atomic(&path, &out)
}

/// Saved favourite roots, newest first. The one key that legitimately
/// repeats: each visit to a directory appends a `place = …` line ahead of
/// its elders, duplicates collapse onto the newcomer, and the list caps at
/// eight so it stays a short menu rather than a history.
pub const PLACE_CAP: usize = 8;

#[must_use]
pub fn load_places_in(dir: &Path) -> Vec<PathBuf> {
    load_all_in(dir).places
}

fn place_value(line: &str) -> Option<&str> {
    let (k, v) = line.trim().split_once('=')?;
    (k.trim() == "place")
        .then(|| v.trim())
        .filter(|v| !v.is_empty())
}

pub fn add_place_in(dir: &Path, root: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join("config");
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let me = root.display().to_string();
    let mut kept_places = 0usize;
    let mut lines: Vec<String> = existing
        .lines()
        .filter(|l| match place_value(l) {
            // Dedupe: the newcomer replaces its elder.
            Some(v) if v == me => false,
            Some(_) => {
                kept_places += 1;
                // Drop the OLDEST once the menu would outgrow itself. The
                // newcomer inserted below needs the last slot.
                kept_places < PLACE_CAP
            }
            None => true,
        })
        .map(String::from)
        .collect();
    // Newest first: ahead of every elder place line, or at the end when the
    // file has none yet.
    let first_place = lines.iter().position(|l| place_value(l).is_some());
    match first_place {
        Some(i) => lines.insert(i, format!("place = {me}")),
        None => lines.push(format!("place = {me}")),
    }
    let mut out = lines.join("\n");
    out.push('\n');
    write_atomic(&path, &out)
}

/// The saved theme name, if any.
#[must_use]
pub fn load_theme() -> Option<String> {
    load_theme_in(&config_dir()?)
}

#[must_use]
pub fn load_theme_in(dir: &Path) -> Option<String> {
    load_all_in(dir).theme
}

pub fn save_theme(name: &str) -> std::io::Result<()> {
    let dir =
        config_dir().ok_or_else(|| std::io::Error::other("no XDG_CONFIG_HOME and no HOME"))?;
    upsert_key_in(&dir, "theme", name)
}

pub fn save_theme_in(dir: &Path, name: &str) -> std::io::Result<()> {
    upsert_key_in(dir, "theme", name)
}

/// The saved hint-bar visibility. Absent means on.
#[must_use]
pub fn load_hints() -> Option<bool> {
    load_hints_in(&config_dir()?)
}

#[must_use]
pub fn load_hints_in(dir: &Path) -> Option<bool> {
    load_all_in(dir).hints
}

pub fn save_hints_in(dir: &Path, on: bool) -> std::io::Result<()> {
    upsert_key_in(dir, "hints", if on { "true" } else { "false" })
}

/// The saved breadcrumb-band visibility. Absent means on.
#[must_use]
pub fn load_breadcrumb() -> Option<bool> {
    load_breadcrumb_in(&config_dir()?)
}

#[must_use]
pub fn load_breadcrumb_in(dir: &Path) -> Option<bool> {
    load_all_in(dir).breadcrumb
}

pub fn save_breadcrumb_in(dir: &Path, on: bool) -> std::io::Result<()> {
    upsert_key_in(dir, "breadcrumb", if on { "true" } else { "false" })
}

/// The saved margin-outline setting. Absent means **off**: it changes the
/// text column's geometry, and nobody's screen should move on upgrade.
#[must_use]
pub fn load_outline_margin() -> Option<bool> {
    load_outline_margin_in(&config_dir()?)
}

#[must_use]
pub fn load_outline_margin_in(dir: &Path) -> Option<bool> {
    load_all_in(dir).outline_margin
}

pub fn save_outline_margin_in(dir: &Path, on: bool) -> std::io::Result<()> {
    upsert_key_in(dir, "outline_margin", if on { "true" } else { "false" })
}

/// The saved title setting. Absent means **off**: a file list that shows
/// something other than file names is a different product, and that should
/// be asked for.
#[must_use]
pub fn load_titles() -> Option<bool> {
    load_titles_in(&config_dir()?)
}

#[must_use]
pub fn load_titles_in(dir: &Path) -> Option<bool> {
    load_all_in(dir).titles
}

pub fn save_titles_in(dir: &Path, on: bool) -> std::io::Result<()> {
    upsert_key_in(dir, "titles", if on { "true" } else { "false" })
}

/// The default reading measure, in columns.
///
/// Typographic practice puts the comfortable measure at roughly 45–90
/// characters; past that the eye loses the line return. 90 is the top of that
/// range, chosen so the default changes as little as possible while still
/// fixing the 200-column paragraph — a terminal at or under 90 usable columns
/// sees no difference at all.
pub const DEFAULT_MEASURE: u16 = 90;

/// Below this a "measure" is not a reading experience, it is a typo in a
/// config file. Clamp rather than obey.
pub const MIN_MEASURE: u16 = 20;

/// The saved reading measure. `Some(0)` means explicitly OFF — full bleed,
/// the pre-measure behaviour — while `None` means absent, and the caller uses
/// [`DEFAULT_MEASURE`]. The two are deliberately different.
#[must_use]
pub fn load_max_width() -> Option<u16> {
    load_max_width_in(&config_dir()?)
}

#[must_use]
pub fn load_max_width_in(dir: &Path) -> Option<u16> {
    load_all_in(dir).max_width
}

pub fn save_max_width_in(dir: &Path, w: u16) -> std::io::Result<()> {
    upsert_key_in(dir, "max_width", &w.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_round_trip_newest_first_and_dedupe() {
        let d = tempfile::tempdir().unwrap();
        assert!(load_places_in(d.path()).is_empty());
        add_place_in(d.path(), Path::new("/a")).unwrap();
        add_place_in(d.path(), Path::new("/b")).unwrap();
        assert_eq!(
            load_places_in(d.path()),
            vec![PathBuf::from("/b"), PathBuf::from("/a")],
            "newest first"
        );
        // Re-visiting /a promotes it, not duplicates it.
        add_place_in(d.path(), Path::new("/a")).unwrap();
        assert_eq!(
            load_places_in(d.path()),
            vec![PathBuf::from("/a"), PathBuf::from("/b")]
        );
    }

    #[test]
    fn the_place_menu_caps_at_eight() {
        let d = tempfile::tempdir().unwrap();
        for i in 0..12 {
            add_place_in(d.path(), Path::new(&format!("/p{i}"))).unwrap();
        }
        let places = load_places_in(d.path());
        assert_eq!(places.len(), PLACE_CAP);
        assert_eq!(places[0], PathBuf::from("/p11"), "newest kept");
        // Other keys survive the rewriting.
        save_root_in(d.path(), Path::new("/root")).unwrap();
        add_place_in(d.path(), Path::new("/zz")).unwrap();
        assert_eq!(load_root_in(d.path()), Some(PathBuf::from("/root")));
    }

    #[test]
    fn breadcrumb_round_trip_and_default_on() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(
            load_breadcrumb_in(d.path()),
            None,
            "absent file: caller defaults on"
        );
        save_breadcrumb_in(d.path(), false).unwrap();
        assert_eq!(load_breadcrumb_in(d.path()), Some(false));
        save_breadcrumb_in(d.path(), true).unwrap();
        assert_eq!(load_breadcrumb_in(d.path()), Some(true));
    }

    #[test]
    fn hints_round_trip_and_default_on() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(
            load_hints_in(d.path()),
            None,
            "absent file: caller defaults on"
        );
        save_hints_in(d.path(), false).unwrap();
        assert_eq!(load_hints_in(d.path()), Some(false));
        save_hints_in(d.path(), true).unwrap();
        assert_eq!(load_hints_in(d.path()), Some(true));
    }

    #[test]
    fn max_width_round_trips_through_the_config_file() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(
            load_max_width_in(d.path()),
            None,
            "absent: caller uses DEFAULT_MEASURE"
        );
        save_max_width_in(d.path(), 72).unwrap();
        assert_eq!(load_max_width_in(d.path()), Some(72));
    }

    #[test]
    fn zero_is_preserved_because_it_means_off_not_absent() {
        let d = tempfile::tempdir().unwrap();
        save_max_width_in(d.path(), 0).unwrap();
        assert_eq!(load_max_width_in(d.path()), Some(0));
    }

    #[test]
    fn an_absurdly_narrow_measure_clamps_to_the_floor() {
        let d = tempfile::tempdir().unwrap();
        save_max_width_in(d.path(), 3).unwrap();
        assert_eq!(load_max_width_in(d.path()), Some(MIN_MEASURE));
    }

    #[test]
    fn garbage_reads_as_absent_rather_than_failing() {
        let d = tempfile::tempdir().unwrap();
        upsert_key_in(d.path(), "max_width", "wide please").unwrap();
        assert_eq!(load_max_width_in(d.path()), None);
    }

    #[test]
    fn max_width_preserves_the_other_keys() {
        let d = tempfile::tempdir().unwrap();
        save_root_in(d.path(), Path::new("/somewhere")).unwrap();
        save_theme_in(d.path(), "paper").unwrap();
        save_max_width_in(d.path(), 72).unwrap();
        assert_eq!(load_root_in(d.path()), Some(PathBuf::from("/somewhere")));
        assert_eq!(load_theme_in(d.path()), Some("paper".into()));
    }

    #[test]
    fn hints_key_preserves_root_and_theme_lines() {
        let d = tempfile::tempdir().unwrap();
        save_root_in(d.path(), Path::new("/somewhere")).unwrap();
        save_theme_in(d.path(), "paper").unwrap();
        save_hints_in(d.path(), false).unwrap();
        assert_eq!(load_root_in(d.path()), Some(PathBuf::from("/somewhere")));
        assert_eq!(load_theme_in(d.path()), Some("paper".into()));
    }

    #[test]
    fn parses_a_root_line() {
        assert_eq!(
            parse_all("root = /tmp/x\n").root,
            Some(PathBuf::from("/tmp/x"))
        );
    }

    #[test]
    fn tolerates_whitespace_and_missing_spaces() {
        assert_eq!(parse_all("root=/tmp/x").root, Some(PathBuf::from("/tmp/x")));
        assert_eq!(
            parse_all("  root   =   /tmp/x   ").root,
            Some(PathBuf::from("/tmp/x"))
        );
    }

    #[test]
    fn ignores_unknown_keys_and_comments() {
        // A newer version's file must not break an older binary.
        let text = "# a comment\ntheme = dark\nroot = /tmp/x\nfuture_key = 3\n";
        assert_eq!(parse_all(text).root, Some(PathBuf::from("/tmp/x")));
    }

    #[test]
    fn malformed_input_yields_no_root_rather_than_an_error() {
        assert_eq!(parse_all("").root, None);
        assert_eq!(parse_all("garbage").root, None);
        assert_eq!(parse_all("root =").root, None);
    }

    #[test]
    fn round_trips_through_a_real_directory() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_root_in(dir.path()), None, "nothing saved yet");
        save_root_in(dir.path(), Path::new("/tmp/somewhere")).unwrap();
        assert_eq!(
            load_root_in(dir.path()),
            Some(PathBuf::from("/tmp/somewhere"))
        );
    }

    #[test]
    fn saving_one_key_preserves_the_other() {
        let dir = tempfile::tempdir().unwrap();
        save_root_in(dir.path(), Path::new("/tmp/somewhere")).unwrap();
        save_theme_in(dir.path(), "gruvbox-dark").unwrap();
        assert_eq!(
            load_root_in(dir.path()),
            Some(PathBuf::from("/tmp/somewhere"))
        );
        assert_eq!(load_theme_in(dir.path()).as_deref(), Some("gruvbox-dark"));
        // And the other direction: re-saving the root keeps the theme.
        save_root_in(dir.path(), Path::new("/tmp/elsewhere")).unwrap();
        assert_eq!(load_theme_in(dir.path()).as_deref(), Some("gruvbox-dark"));
        assert_eq!(
            load_root_in(dir.path()),
            Some(PathBuf::from("/tmp/elsewhere"))
        );
    }

    #[test]
    fn a_missing_directory_is_not_an_error_on_load() {
        assert_eq!(load_root_in(Path::new("/nonexistent/xyzzy")), None);
    }

    /// One file, two readings of a boolean: `hints`/`breadcrumb` took
    /// `v != "false"` and `titles`/`outline_margin` took `v == "true"`, so
    /// `breadcrumb = 1` was on while `titles = 1` was off. Nothing documented
    /// either spelling, so both were guesses a reader had to make twice.
    #[test]
    fn every_boolean_key_reads_the_same_spellings() {
        let d = tempfile::tempdir().unwrap();
        for (on, off) in [
            ("true", "false"),
            ("1", "0"),
            ("yes", "no"),
            ("TRUE", "False"),
        ] {
            std::fs::write(
                d.path().join("config"),
                format!(
                    "hints = {on}\nbreadcrumb = {off}\ntitles = {on}\noutline_margin = {off}\n"
                ),
            )
            .unwrap();
            assert_eq!(load_hints_in(d.path()), Some(true), "hints = {on}");
            assert_eq!(
                load_breadcrumb_in(d.path()),
                Some(false),
                "breadcrumb = {off}"
            );
            assert_eq!(load_titles_in(d.path()), Some(true), "titles = {on}");
            assert_eq!(
                load_outline_margin_in(d.path()),
                Some(false),
                "outline_margin = {off}"
            );
        }
    }

    /// A value that is not a spelling of either is not an answer, so each key
    /// falls back to its OWN default — which is what both old readings did by
    /// accident, and is the behaviour worth keeping deliberately.
    #[test]
    fn an_unreadable_boolean_falls_back_to_that_keys_default() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(
            d.path().join("config"),
            "hints = wibble\nbreadcrumb = wibble\ntitles = wibble\noutline_margin = wibble\n",
        )
        .unwrap();
        assert_eq!(load_hints_in(d.path()), Some(true), "hints default on");
        assert_eq!(
            load_breadcrumb_in(d.path()),
            Some(true),
            "breadcrumb default on"
        );
        assert_eq!(load_titles_in(d.path()), Some(false), "titles default off");
        assert_eq!(
            load_outline_margin_in(d.path()),
            Some(false),
            "outline_margin default off"
        );
    }

    /// Startup read the file eight times through six near-identical wrappers,
    /// each `read_to_string`-ing and re-scanning all of it. One pass answers
    /// every key, and answers them from the same bytes — so a file rewritten
    /// mid-startup cannot be read half one way and half the other.
    #[test]
    fn one_read_answers_every_key() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(
            d.path().join("config"),
            "# mine\nroot = /r\ntheme = paper\nhints = false\nbreadcrumb = 0\n\
             outline_margin = yes\ntitles = 1\nmax_width = 72\nplace = /p1\nplace = /p2\n\
             future_key = 3\n",
        )
        .unwrap();
        let c = load_all_in(d.path());
        assert_eq!(c.root, Some(PathBuf::from("/r")));
        assert_eq!(c.theme.as_deref(), Some("paper"));
        assert_eq!(c.hints, Some(false));
        assert_eq!(c.breadcrumb, Some(false));
        assert_eq!(c.outline_margin, Some(true));
        assert_eq!(c.titles, Some(true));
        assert_eq!(c.max_width, Some(72));
        assert_eq!(c.places, vec![PathBuf::from("/p1"), PathBuf::from("/p2")]);

        // And an absent file answers "nothing set" for all of them at once.
        let empty = load_all_in(Path::new("/nonexistent/xyzzy"));
        assert_eq!(empty.root, None);
        assert_eq!(empty.hints, None);
        assert!(empty.places.is_empty());
    }
}
