//! `[[wikilink]]` target resolution — follow-time, frontend-side.
//!
//! The core never touches the filesystem; it hands over the raw target
//! ("Reflow Layer"). Resolution order (spec §3): exact sibling, then
//! case-insensitive sibling, then a basename match across the home screen's
//! scanned index — deterministic tie-break: shortest path, then lexicographic.
//! NO RATATUI — `scripts/check-discipline.sh` rule 6.

use std::path::{Path, PathBuf};

use crate::scan::Entry;

/// Resolve a wikilink target to a markdown file, or `None` — never an error.
#[must_use]
pub fn resolve(target: &str, here_dir: &Path, index: &[Entry]) -> Option<PathBuf> {
    let want = format!("{target}.md");

    // 1. Exact sibling.
    let exact = here_dir.join(&want);
    if exact.is_file() {
        return Some(exact);
    }

    // 2. Case-insensitive sibling.
    let want_lower = want.to_lowercase();
    if let Ok(rd) = std::fs::read_dir(here_dir) {
        let mut hits: Vec<PathBuf> = rd
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.to_lowercase() == want_lower)
                    && p.is_file()
            })
            .collect();
        hits.sort();
        if let Some(p) = hits.into_iter().next() {
            return Some(p);
        }
    }

    // 3. Basename match across the scanned index.
    let mut hits: Vec<&PathBuf> = index
        .iter()
        .map(|e| &e.path)
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.to_lowercase() == want_lower)
        })
        .collect();
    hits.sort_by_key(|p| (p.components().count(), (*p).clone()));
    hits.into_iter().next().cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(p: &Path) -> Entry {
        Entry {
            path: p.to_path_buf(),
            mtime: std::time::SystemTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn an_exact_sibling_wins() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("Reflow Layer.md"), "x").unwrap();
        assert_eq!(
            resolve("Reflow Layer", d.path(), &[]),
            Some(d.path().join("Reflow Layer.md"))
        );
    }

    #[test]
    fn a_case_insensitive_sibling_is_found() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("reflow layer.md"), "x").unwrap();
        assert_eq!(
            resolve("Reflow Layer", d.path(), &[]),
            Some(d.path().join("reflow layer.md"))
        );
    }

    #[test]
    fn the_index_resolves_what_the_directory_cannot() {
        let d = tempfile::tempdir().unwrap();
        let far = d.path().join("notes").join("deep").join("Reflow Layer.md");
        assert_eq!(
            resolve("Reflow Layer", d.path(), &[entry(&far)]),
            Some(far),
            "index paths need not exist on disk — the scan found them"
        );
    }

    #[test]
    fn index_ties_break_shortest_path_then_lexicographic() {
        let d = tempfile::tempdir().unwrap();
        let deep = d.path().join("a").join("b").join("Note.md");
        let shallow_b = d.path().join("b").join("Note.md");
        let shallow_a = d.path().join("a").join("Note.md");
        let idx = [entry(&deep), entry(&shallow_b), entry(&shallow_a)];
        assert_eq!(resolve("Note", d.path(), &idx), Some(shallow_a));
    }

    #[test]
    fn an_unresolvable_target_is_none_not_an_error() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(resolve("No Such Note", d.path(), &[]), None);
    }
}
