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
//! **Migration trigger: past three or four keys, switch to TOML.** Hand-rolled
//! config formats ossify — replace this before it grows a syntax.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

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
    parse(&std::fs::read_to_string(dir.join("config")).ok()?)
}

pub fn save_root_in(dir: &Path, root: &Path) -> std::io::Result<()> {
    upsert_key_in(dir, "root", &root.display().to_string())
}

/// `key = value` per line. Unknown keys and `#` comments are ignored, so a
/// newer version's file cannot break an older binary.
fn parse(text: &str) -> Option<PathBuf> {
    parse_key(text, "root").map(PathBuf::from)
}

fn parse_key(text: &str, key: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        if k.trim() == key {
            let v = v.trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
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
    std::fs::write(path, out)
}

/// Saved favourite roots, newest first. The one key that legitimately
/// repeats: each visit to a directory appends a `place = …` line ahead of
/// its elders, duplicates collapse onto the newcomer, and the list caps at
/// eight so it stays a short menu rather than a history.
pub const PLACE_CAP: usize = 8;

#[must_use]
pub fn load_places_in(dir: &Path) -> Vec<PathBuf> {
    let text = std::fs::read_to_string(dir.join("config")).unwrap_or_default();
    let mut out: Vec<PathBuf> = Vec::new();
    for line in text.lines() {
        let Some(v) = place_value(line) else { continue };
        let p = PathBuf::from(v);
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out.truncate(PLACE_CAP);
    out
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
        .filter(|l| {
            let is_mine = place_value(l).is_some_and(|v| v == me);
            let is_place = place_value(l).is_some();
            if is_mine {
                return false; // dedupe: the newcomer replaces its elder
            }
            if is_place {
                kept_places += 1;
                // Drop the OLDEST once the menu would outgrow itself.
                if kept_places > PLACE_CAP - 1 {
                    return false;
                }
            }
            true
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
    let mut out = lines.join(
        "
",
    );
    out.push('\n');
    std::fs::write(path, out)
}

/// The saved theme name, if any.
#[must_use]
pub fn load_theme() -> Option<String> {
    load_theme_in(&config_dir()?)
}

#[must_use]
pub fn load_theme_in(dir: &Path) -> Option<String> {
    parse_key(&std::fs::read_to_string(dir.join("config")).ok()?, "theme")
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
    parse_key(&std::fs::read_to_string(dir.join("config")).ok()?, "hints").map(|v| v != "false")
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
    parse_key(
        &std::fs::read_to_string(dir.join("config")).ok()?,
        "breadcrumb",
    )
    .map(|v| v != "false")
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
    parse_key(
        &std::fs::read_to_string(dir.join("config")).ok()?,
        "outline_margin",
    )
    .map(|v| v == "true")
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
    parse_key(&std::fs::read_to_string(dir.join("config")).ok()?, "titles").map(|v| v == "true")
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
    let raw: u16 = parse_key(
        &std::fs::read_to_string(dir.join("config")).ok()?,
        "max_width",
    )?
    .parse()
    .ok()?;
    Some(if raw == 0 { 0 } else { raw.max(MIN_MEASURE) })
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
        assert_eq!(parse("root = /tmp/x\n"), Some(PathBuf::from("/tmp/x")));
    }

    #[test]
    fn tolerates_whitespace_and_missing_spaces() {
        assert_eq!(parse("root=/tmp/x"), Some(PathBuf::from("/tmp/x")));
        assert_eq!(
            parse("  root   =   /tmp/x   "),
            Some(PathBuf::from("/tmp/x"))
        );
    }

    #[test]
    fn ignores_unknown_keys_and_comments() {
        // A newer version's file must not break an older binary.
        let text = "# a comment\ntheme = dark\nroot = /tmp/x\nfuture_key = 3\n";
        assert_eq!(parse(text), Some(PathBuf::from("/tmp/x")));
    }

    #[test]
    fn malformed_input_yields_no_root_rather_than_an_error() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("garbage"), None);
        assert_eq!(parse("root ="), None);
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
}
