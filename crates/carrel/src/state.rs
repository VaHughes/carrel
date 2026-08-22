//! Reading positions — the XDG *state* dir, not config.
//!
//! NO RATATUI — `scripts/check-discipline.sh` rule 6. And no reaching the
//! real directory from library code: everything flows through
//! `App::state_dir`, which is `None` in every constructor, for the same
//! reason `config.rs` works that way (a test once stomped the developer's
//! real config; see the comment there).
//!
//! Format: one entry per line, `anchor<TAB>saved_at<TAB>path`. The path is
//! last because it is the only field that could contain a tab; parsing
//! splits from the left exactly twice. Malformed lines are ignored, so a
//! newer version's file cannot break an older binary.

use std::path::{Path, PathBuf};

/// Keep the most recent N files. Positions are a convenience, not a
/// database; the cap stops the file growing forever.
const CAP: usize = 500;

/// `$XDG_STATE_HOME/carrel`, else `~/.local/state/carrel`.
#[must_use]
pub fn state_dir() -> Option<PathBuf> {
    if let Some(x) = std::env::var_os("XDG_STATE_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(x).join("carrel"));
    }
    std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(|h| PathBuf::from(h).join(".local").join("state").join("carrel"))
}

#[derive(Debug)]
struct Entry {
    anchor: u32,
    saved_at: u64,
    path: String,
}

fn parse(text: &str) -> Vec<Entry> {
    text.lines()
        .filter_map(|line| {
            let mut it = line.splitn(3, '\t');
            let anchor = it.next()?.parse().ok()?;
            let saved_at = it.next()?.parse().ok()?;
            let path = it.next()?;
            (!path.is_empty()).then(|| Entry {
                anchor,
                saved_at,
                path: path.to_string(),
            })
        })
        .collect()
}

/// The canonical key for a file: symlink-resolved when possible, so the same
/// document reached by two spellings shares one position.
fn key(file: &Path) -> String {
    std::fs::canonicalize(file)
        .unwrap_or_else(|_| file.to_path_buf())
        .display()
        .to_string()
}

/// The saved anchor for `file`, if any. A missing directory, file, or entry
/// is `None` — never an error.
#[must_use]
pub fn load_position_in(dir: &Path, file: &Path) -> Option<u32> {
    let text = std::fs::read_to_string(dir.join("positions")).ok()?;
    let k = key(file);
    parse(&text)
        .into_iter()
        .find(|e| e.path == k)
        .map(|e| e.anchor)
}

/// Upsert `file`'s anchor; `anchor == 0` removes the entry (top of the file
/// is the default — remembering it is noise). Keeps the newest [`CAP`]
/// entries by save time.
pub fn save_position_in(dir: &Path, file: &Path, anchor: u32) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join("positions");
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let k = key(file);
    let mut entries: Vec<Entry> = parse(&existing)
        .into_iter()
        .filter(|e| e.path != k)
        .collect();
    if anchor > 0 {
        let saved_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        entries.push(Entry {
            anchor,
            saved_at,
            path: k,
        });
    }
    entries.sort_by_key(|e| std::cmp::Reverse(e.saved_at));
    entries.truncate(CAP);
    let mut out = String::new();
    for e in &entries {
        use std::fmt::Write as _;
        let _ = writeln!(out, "{}\t{}\t{}", e.anchor, e.saved_at, e.path);
    }
    std::fs::write(path, out)
}

/// The bookmarks for `file`, in document order. Never an error.
///
/// Kept in their own file rather than as another column of `positions`:
/// a position is one number that is always overwritten, a bookmark list is
/// many and is edited. Same key function, so the two agree about what "the
/// same document reached two ways" means.
#[must_use]
pub fn load_marks_in(dir: &Path, file: &Path) -> Vec<u32> {
    let Ok(text) = std::fs::read_to_string(dir.join("bookmarks")) else {
        return Vec::new();
    };
    let k = key(file);
    for line in text.lines() {
        let Some((path, marks)) = line.split_once('\t') else {
            continue;
        };
        if path != k {
            continue;
        }
        let mut out: Vec<u32> = marks.split(',').filter_map(|m| m.parse().ok()).collect();
        out.sort_unstable();
        out.dedup();
        return out;
    }
    Vec::new()
}

/// Replace `file`'s bookmarks. An empty list removes the entry.
///
/// **Path first here**, unlike `positions`: the marks are a comma-joined
/// list of digits, so the tab-safe field is the one that has to come last —
/// and that is the marks, not the path.
pub fn save_marks_in(dir: &Path, file: &Path, marks: &[u32]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join("bookmarks");
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let k = key(file);
    let mut out = String::new();
    for line in existing.lines() {
        if line.split_once('\t').is_none_or(|(p, _)| p != k) && !line.trim().is_empty() {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !marks.is_empty() {
        let mut sorted: Vec<u32> = marks.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        let joined: Vec<String> = sorted.iter().map(ToString::to_string).collect();
        use std::fmt::Write as _;
        let _ = writeln!(out, "{k}\t{}", joined.join(","));
    }
    std::fs::write(path, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_bookmarks_and_keeps_other_documents() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.md");
        let b = dir.path().join("b.md");
        std::fs::write(&a, "x").unwrap();
        std::fs::write(&b, "y").unwrap();
        assert!(load_marks_in(dir.path(), &a).is_empty());

        save_marks_in(dir.path(), &a, &[300, 100, 100]).unwrap();
        assert_eq!(
            load_marks_in(dir.path(), &a),
            vec![100, 300],
            "sorted and deduped"
        );

        save_marks_in(dir.path(), &b, &[7]).unwrap();
        assert_eq!(load_marks_in(dir.path(), &a), vec![100, 300], "a survived");
        assert_eq!(load_marks_in(dir.path(), &b), vec![7]);

        save_marks_in(dir.path(), &a, &[]).unwrap();
        assert!(load_marks_in(dir.path(), &a).is_empty(), "cleared");
        assert_eq!(load_marks_in(dir.path(), &b), vec![7], "b still survived");
    }

    #[test]
    fn round_trips_a_position() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("doc.md");
        std::fs::write(&f, "x").unwrap();
        assert_eq!(load_position_in(dir.path(), &f), None, "nothing saved yet");
        save_position_in(dir.path(), &f, 1234).unwrap();
        assert_eq!(load_position_in(dir.path(), &f), Some(1234));
        save_position_in(dir.path(), &f, 99).unwrap();
        assert_eq!(
            load_position_in(dir.path(), &f),
            Some(99),
            "upsert, not append"
        );
    }

    #[test]
    fn anchor_zero_deletes_the_entry() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("doc.md");
        std::fs::write(&f, "x").unwrap();
        save_position_in(dir.path(), &f, 1234).unwrap();
        save_position_in(dir.path(), &f, 0).unwrap();
        assert_eq!(
            load_position_in(dir.path(), &f),
            None,
            "scrolled back to top"
        );
    }

    #[test]
    fn two_files_keep_independent_positions() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.md");
        let b = dir.path().join("b.md");
        std::fs::write(&a, "x").unwrap();
        std::fs::write(&b, "x").unwrap();
        save_position_in(dir.path(), &a, 10).unwrap();
        save_position_in(dir.path(), &b, 20).unwrap();
        assert_eq!(load_position_in(dir.path(), &a), Some(10));
        assert_eq!(load_position_in(dir.path(), &b), Some(20));
    }

    #[test]
    fn the_cap_evicts_and_the_file_never_exceeds_it() {
        let dir = tempfile::tempdir().unwrap();
        // Paths need not exist: canonicalize falls back to the spelling.
        for i in 0..(CAP + 10) {
            let f = dir.path().join(format!("f{i}.md"));
            save_position_in(dir.path(), &f, 1).unwrap();
        }
        let text = std::fs::read_to_string(dir.path().join("positions")).unwrap();
        assert_eq!(text.lines().count(), CAP);
    }

    #[test]
    fn malformed_lines_are_ignored_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("positions"), "garbage\n12\tnotanum\t/x\n").unwrap();
        let f = dir.path().join("doc.md");
        assert_eq!(load_position_in(dir.path(), &f), None);
    }

    #[test]
    fn a_missing_directory_is_not_an_error_on_load() {
        assert_eq!(
            load_position_in(Path::new("/nonexistent/xyzzy"), Path::new("/tmp/x.md")),
            None
        );
    }
}
