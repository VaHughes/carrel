//! Reading positions — the XDG *state* dir, not config.
//!
//! NO RATATUI — `scripts/check-discipline.sh` rule 6. And no reaching the
//! real directory from library code: everything flows through
//! `App::state_dir`, which is `None` in every constructor, for the same
//! reason `config.rs` works that way (a test once stomped the developer's
//! real config; see the comment there).
//!
//! Format: one entry per line,
//! `anchor<TAB>saved_at<TAB>permille<TAB>words<TAB>path`, the path
//! **escaped** by [`escape_field`]. **The old three-field form still
//! parses** — an entry written before 2026-08-21 simply has no progress to
//! show, rather than being dropped — and malformed lines are ignored, so a
//! newer version's file cannot break an older binary.
//!
//! `permille` and `words` exist so the home screen can say "64%, 18 min
//! left" without opening the file. Opening every remembered document to
//! paint a list of them is not affordable; both numbers are already
//! computed by the status bar at the moment a position is saved.
//!
//! # The on-disk helpers live here
//!
//! [`write_atomic`] and [`escape_field`] are `pub(crate)` and used by
//! `config.rs` and `scan.rs` as well. They belong to one module because two
//! spellings of "escape a path into a line" is exactly how the bookmarks
//! file came to be corruptible; this is the module whose whole subject is
//! records carrel writes to disk, so it is the one that owns them.

use std::path::{Path, PathBuf};

/// Replace `path`'s contents with `text` without ever truncating it in place.
///
/// `fs::write` opens with `O_TRUNC` and then writes, so the file is empty for
/// as long as the write takes. Every file this project keeps is a
/// read-modify-write of *everything* it holds — up to [`CAP`] reading
/// positions — so a crash, a SIGKILL or a full disk inside that window does
/// not cost the last line, it costs all of them, and a second carrel reading
/// at that instant sees a truncated file. Writing a sibling temp file and
/// `rename`-ing it onto the target closes the window: `rename(2)` within one
/// directory is atomic, so an observer sees either the whole old file or the
/// whole new one. The temp file is a sibling rather than in `/tmp` because
/// `rename` is only atomic — only *possible*, in fact — within a filesystem.
///
/// `sync_all` before the rename is what makes the crash case hold and not
/// just the concurrent-reader one: without it the rename can reach the disk
/// before the data does, and the survivor is a correctly-named empty file.
///
/// **This does not make concurrent writers safe, and is not meant to.** Two
/// carrels in two terminals still race — each reads the whole file, each
/// rewrites the whole file, and the later `rename` wins entire, so the loser's
/// session is discarded. That is last-writer-wins, unchanged from before;
/// what changes is that the loser's outcome is a coherent older file rather
/// than a shredded one. Doing better needs a lock file, which needs a policy
/// for the stale lock a killed reader leaves behind, and that is a worse
/// trade for state this size.
pub(crate) fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    // The pid keeps two carrels from writing the same temp file and renaming
    // each other's half-written bytes into place.
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let write = |tmp: &Path| -> std::io::Result<()> {
        let mut f = std::fs::File::create(tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()
    };
    if let Err(e) = write(&tmp) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// Escape a path so it survives as one field of one TAB-separated line.
///
/// Unix forbids only `/` and NUL in a filename, so a TAB or a NEWLINE in a
/// path is legal and does turn up. Unescaped, the tab splits the record
/// *inside* the path so no entry ever matches again, and the newline splits
/// one record into two — which also means a crafted filename can inject a
/// forged entry for another document.
///
/// The escape is the C one, `\\` `\t` `\n` (and `\r`, which is not a
/// separator here but would still be a nasty thing to write into a line
/// verbatim). It is chosen because it is its own inverse under
/// [`unescape_field`] and because **an already-written file reads back
/// unchanged**: a path with none of these characters escapes to itself, and
/// unescaping is identity on any string without a backslash. The one thing
/// it cannot recover is a path containing a literal backslash written by a
/// version older than this one — a `\t` on disk from then is read as a tab.
/// That is a strictly smaller set than the paths broken before the fix.
pub(crate) fn escape_field(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out
}

/// The inverse of [`escape_field`]. An unknown escape keeps both characters,
/// so a path this version never wrote is returned as it was found rather than
/// silently mangled.
pub(crate) fn unescape_field(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            // An escaped backslash, and a trailing one with nothing after it:
            // both are one literal backslash.
            Some('\\') | None => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    out
}

/// Keep the most recent N files. Positions are a convenience, not a
/// database; the cap stops the file growing forever.
const CAP: usize = 500;

/// The same cap for bookmarks, and for the same reason.
const MARK_CAP: usize = 500;

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

/// One remembered document.
#[derive(Debug, Clone)]
pub struct Entry {
    pub anchor: u32,
    pub saved_at: u64,
    /// How far in, in permille. `None` for an entry written before the
    /// field existed.
    pub permille: Option<u16>,
    /// The document's word count when it was last read, for the estimate.
    pub words: Option<u32>,
    pub path: String,
}

fn parse(text: &str) -> Vec<Entry> {
    text.lines()
        .filter_map(|line| {
            let mut it = line.splitn(5, '\t');
            let anchor = it.next()?.parse().ok()?;
            let saved_at = it.next()?.parse().ok()?;
            let third = it.next()?;
            // Three fields: the old form, where the third IS the path.
            let Some(fourth) = it.next() else {
                return (!third.is_empty()).then(|| Entry {
                    anchor,
                    saved_at,
                    permille: None,
                    words: None,
                    path: unescape_field(third),
                });
            };
            let path = it.next()?;
            (!path.is_empty()).then(|| Entry {
                anchor,
                saved_at,
                permille: third.parse().ok(),
                words: fourth.parse().ok(),
                path: unescape_field(path),
            })
        })
        .collect()
}

/// Every remembered document, most recently read first.
///
/// The home screen's "continue reading" list. Entries whose file has gone
/// are the caller's to drop — this layer does not touch the disk to check.
#[must_use]
pub fn recent_in(dir: &Path) -> Vec<Entry> {
    let Ok(text) = std::fs::read_to_string(dir.join("positions")) else {
        return Vec::new();
    };
    let mut v = parse(&text);
    v.sort_by_key(|e| std::cmp::Reverse(e.saved_at));
    v
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
pub fn save_position_in(
    dir: &Path,
    file: &Path,
    anchor: u32,
    permille: u16,
    words: u32,
) -> std::io::Result<()> {
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
            permille: Some(permille),
            words: Some(words),
            path: k,
        });
    }
    entries.sort_by_key(|e| std::cmp::Reverse(e.saved_at));
    entries.truncate(CAP);
    let mut out = String::new();
    for e in &entries {
        use std::fmt::Write as _;
        let _ = writeln!(
            out,
            "{}\t{}\t{}\t{}\t{}",
            e.anchor,
            e.saved_at,
            e.permille.unwrap_or(0),
            e.words.unwrap_or(0),
            escape_field(&e.path)
        );
    }
    write_atomic(&path, &out)
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
    let k = escape_field(&key(file));
    for line in text.lines() {
        let Some((path, marks)) = line.split_once('\t') else {
            continue;
        };
        // Compared escaped, so one `escape_field` answers the whole file
        // rather than one `unescape_field` per line.
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
/// **Path first here**, unlike `positions`, and the ordering is not what
/// makes the line safe. An earlier comment here argued the marks had to come
/// last because they are "the tab-safe field"; that is backwards. Whichever
/// field comes last is the one allowed to contain the separator, so putting
/// the marks there made the PATH the field that must not contain a tab — and
/// Unix paths may. [`escape_field`] is what makes the line safe, in both
/// files; the ordering is now free, and stays as it is only because changing
/// it would orphan every bookmark already on disk.
///
/// Keeps the newest [`MARK_CAP`] documents. Unlike `positions` there is no
/// timestamp to sort on, so "newest" is file order: this function appends the
/// document it just wrote and drops from the front, which is oldest-first as
/// long as every write comes through here.
pub fn save_marks_in(dir: &Path, file: &Path, marks: &[u32]) -> std::io::Result<()> {
    use std::fmt::Write as _;
    std::fs::create_dir_all(dir)?;
    let path = dir.join("bookmarks");
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let k = escape_field(&key(file));
    let mut kept: Vec<&str> = existing
        .lines()
        .filter(|l| l.split_once('\t').is_none_or(|(p, _)| p != k) && !l.trim().is_empty())
        .collect();
    // Room for the line about to be appended, so the cap is a cap on the file
    // and not on the file plus one.
    let room = if marks.is_empty() {
        MARK_CAP
    } else {
        MARK_CAP - 1
    };
    if kept.len() > room {
        kept.drain(..kept.len() - room);
    }
    let mut out = String::new();
    for line in kept {
        out.push_str(line);
        out.push('\n');
    }
    if !marks.is_empty() {
        let mut sorted: Vec<u32> = marks.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        let joined: Vec<String> = sorted.iter().map(ToString::to_string).collect();
        let _ = writeln!(out, "{k}\t{}", joined.join(","));
    }
    write_atomic(&path, &out)
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
        save_position_in(dir.path(), &f, 1234, 0, 0).unwrap();
        assert_eq!(load_position_in(dir.path(), &f), Some(1234));
        save_position_in(dir.path(), &f, 99, 0, 0).unwrap();
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
        save_position_in(dir.path(), &f, 1234, 0, 0).unwrap();
        save_position_in(dir.path(), &f, 0, 0, 0).unwrap();
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
        save_position_in(dir.path(), &a, 10, 0, 0).unwrap();
        save_position_in(dir.path(), &b, 20, 0, 0).unwrap();
        assert_eq!(load_position_in(dir.path(), &a), Some(10));
        assert_eq!(load_position_in(dir.path(), &b), Some(20));
    }

    #[test]
    fn the_cap_evicts_and_the_file_never_exceeds_it() {
        let dir = tempfile::tempdir().unwrap();
        // Paths need not exist: canonicalize falls back to the spelling.
        for i in 0..(CAP + 10) {
            let f = dir.path().join(format!("f{i}.md"));
            save_position_in(dir.path(), &f, 1, 0, 0).unwrap();
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

    /// A path may contain a TAB, a newline or a backslash — Unix forbids only
    /// `/` and NUL. Unescaped, the tab splits the record inside the path and
    /// the newline splits one record into two, so nothing ever matches again.
    #[test]
    fn a_path_with_a_tab_newline_or_backslash_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["with\ttab.md", "with\nnewline.md", "with\\slash.md"] {
            let f = dir.path().join(name);
            std::fs::write(&f, "x").unwrap();
            save_position_in(dir.path(), &f, 42, 500, 100).unwrap();
            assert_eq!(load_position_in(dir.path(), &f), Some(42), "{name:?}");
            save_marks_in(dir.path(), &f, &[5, 9]).unwrap();
            assert_eq!(load_marks_in(dir.path(), &f), vec![5, 9], "{name:?}");
        }
        // One record per document, not one per field the path was split into.
        let positions = std::fs::read_to_string(dir.path().join("positions")).unwrap();
        assert_eq!(positions.lines().count(), 3, "{positions:?}");
        let bookmarks = std::fs::read_to_string(dir.path().join("bookmarks")).unwrap();
        assert_eq!(bookmarks.lines().count(), 3, "{bookmarks:?}");
    }

    /// The failure that has no cap to stop it: `save_marks_in` decides which
    /// line is its own with the same comparison `load_marks_in` uses, so a
    /// path it cannot recognise is a line it never replaces — one more entry
    /// per save, forever.
    #[test]
    fn saving_the_same_document_repeatedly_writes_exactly_one_line() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a\tb.md");
        std::fs::write(&f, "x").unwrap();
        for i in 1..=20u32 {
            save_marks_in(dir.path(), &f, &[i]).unwrap();
        }
        let text = std::fs::read_to_string(dir.path().join("bookmarks")).unwrap();
        assert_eq!(text.lines().count(), 1, "{text:?}");
        assert_eq!(load_marks_in(dir.path(), &f), vec![20]);
    }

    #[test]
    fn the_bookmark_cap_evicts_rather_than_growing_without_bound() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..(MARK_CAP + 10) {
            let f = dir.path().join(format!("f{i}.md"));
            save_marks_in(dir.path(), &f, &[1]).unwrap();
        }
        let text = std::fs::read_to_string(dir.path().join("bookmarks")).unwrap();
        assert_eq!(text.lines().count(), MARK_CAP);
        // The oldest go; the newest is certainly still there.
        let newest = dir.path().join(format!("f{}.md", MARK_CAP + 9));
        assert_eq!(load_marks_in(dir.path(), &newest), vec![1]);
    }

    /// The reason `fs::write` is not good enough: it truncates the target and
    /// then writes it back, so anything reading concurrently — a second
    /// carrel, or a crash's postmortem — can see a half-written or empty file
    /// where a full one was. A `rename` onto the target cannot be observed
    /// half-done.
    #[test]
    fn a_concurrent_reader_never_sees_a_partial_file() {
        let dir = tempfile::tempdir().unwrap();
        let seed = dir.path().join("seed.md");
        for i in 0..CAP {
            save_position_in(dir.path(), &dir.path().join(format!("f{i}.md")), 1, 0, 0).unwrap();
        }
        std::fs::write(&seed, "x").unwrap();
        let positions = dir.path().join("positions");
        let full = std::fs::read_to_string(&positions).unwrap().lines().count();
        assert_eq!(full, CAP, "the file under test is a big one");

        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reader = {
            let (p, stop) = (positions.clone(), std::sync::Arc::clone(&stop));
            std::thread::spawn(move || {
                let mut worst = usize::MAX;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    if let Ok(t) = std::fs::read_to_string(&p) {
                        worst = worst.min(t.lines().count());
                    }
                }
                worst
            })
        };
        for i in 0..200u32 {
            let f = dir.path().join(format!("f{}.md", i % 50));
            save_position_in(dir.path(), &f, i + 1, 0, 0).unwrap();
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let worst = reader.join().unwrap();
        assert_eq!(
            worst, full,
            "a reader saw a {worst}-line file where {full} lines were on disk",
        );
    }
}
