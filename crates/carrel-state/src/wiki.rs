//! `[[wikilink]]` target resolution — follow-time, frontend-side.
//!
//! The core never touches the filesystem; it hands over the raw target
//! ("Reflow Layer"). Resolution order (spec §3): exact sibling, then
//! case-insensitive sibling, then a basename match across the home screen's
//! scanned index — deterministic tie-break: shortest path, then lexicographic.
//! NO RATATUI — `scripts/check-discipline.sh` rule 6.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::scan::Entry;

/// Resolve a wikilink target to a markdown file, or `None` — never an error.
///
/// One target. A caller with a document's worth of them keeps a
/// [`Resolver`], which reads the folder once for all of them.
#[must_use]
pub fn resolve(target: &str, here_dir: &Path, index: &[Entry]) -> Option<PathBuf> {
    Resolver::new(here_dir, index).resolve(target)
}

/// Wikilink resolution for one pass over many targets: one document's
/// links, against one folder and one index.
///
/// The rule is [`resolve`]'s, step for step. What differs is the cost: the
/// folder is listed once, the first time a target is not an exact sibling,
/// and the index is folded once — where a listing per link made an index
/// note of two thousand links in a folder of eight thousand notes take
/// five seconds to open. It lives for a pass and no longer: a file created
/// later is seen by the next pass, never by this one.
#[derive(Debug)]
pub struct Resolver<'a> {
    here: &'a Path,
    index: &'a [Entry],
    /// The folder's entries by lowercased name, each list sorted. `None`
    /// until a target needs it.
    siblings: Option<HashMap<String, Vec<PathBuf>>>,
    /// The index's best path per lowercased file name: fewest components,
    /// then lexicographic.
    indexed: Option<HashMap<String, &'a PathBuf>>,
    /// Answers already given, so a link repeated down a page is asked once.
    answers: HashMap<String, Option<PathBuf>>,
    listings: u32,
}

impl<'a> Resolver<'a> {
    #[must_use]
    pub fn new(here_dir: &'a Path, index: &'a [Entry]) -> Self {
        Self {
            here: here_dir,
            index,
            siblings: None,
            indexed: None,
            answers: HashMap::new(),
            listings: 0,
        }
    }

    /// How many times the folder has been listed: at most once.
    #[must_use]
    pub fn listings(&self) -> u32 {
        self.listings
    }

    /// Resolve one target, as [`resolve`] does.
    pub fn resolve(&mut self, target: &str) -> Option<PathBuf> {
        if let Some(known) = self.answers.get(target) {
            return known.clone();
        }
        let found = self.look_up(target);
        self.answers.insert(target.to_string(), found.clone());
        found
    }

    fn look_up(&mut self, target: &str) -> Option<PathBuf> {
        let want = format!("{target}.md");

        // 1. Exact sibling.
        let exact = self.here.join(&want);
        if exact.is_file() {
            return Some(exact);
        }

        // 2. Case-insensitive sibling: the first in path order that is a
        // file. Asked of the disk per candidate, not per entry — a folder
        // of thousands is not `stat`ed to find the one name that matters.
        let want_lower = want.to_lowercase();
        let here = self.here;
        let listings = &mut self.listings;
        let siblings = self.siblings.get_or_insert_with(|| {
            *listings += 1;
            let mut by_name: HashMap<String, Vec<PathBuf>> = HashMap::new();
            if let Ok(rd) = std::fs::read_dir(here) {
                for path in rd.filter_map(Result::ok).map(|e| e.path()) {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        by_name.entry(name.to_lowercase()).or_default().push(path);
                    }
                }
            }
            for paths in by_name.values_mut() {
                paths.sort();
            }
            by_name
        });
        if let Some(p) = siblings
            .get(&want_lower)
            .and_then(|paths| paths.iter().find(|p| p.is_file()))
        {
            return Some(p.clone());
        }

        // 3. Basename match across the scanned index.
        let index = self.index;
        let indexed = self.indexed.get_or_insert_with(|| {
            let mut best: HashMap<String, (usize, &'a PathBuf)> = HashMap::new();
            for path in index.iter().map(|e| &e.path) {
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                let rank = (path.components().count(), path);
                best.entry(name.to_lowercase())
                    .and_modify(|held| {
                        if rank < *held {
                            *held = rank;
                        }
                    })
                    .or_insert(rank);
            }
            best.into_iter().map(|(name, (_, p))| (name, p)).collect()
        });
        indexed.get(&want_lower).map(|p| (*p).clone())
    }
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

    /// [`resolve`] as it was first written — a listing of the folder and a
    /// scan of the index per target. The rule itself, kept as the oracle.
    fn resolve_listing_every_time(
        target: &str,
        here_dir: &Path,
        index: &[Entry],
    ) -> Option<PathBuf> {
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

    /// Names a folder might hold: case collisions, names that only differ
    /// after Unicode case folding, a name with a `#`, and the bare `.md`.
    const NAMES: &[&str] = &[
        "note.md",
        "Note.md",
        "NOTE.md",
        "other.md",
        "Other.MD",
        "Émile.md",
        "émile.md",
        "ÉMILE.md",
        "straße.md",
        "İstanbul.md",
        "日本語.md",
        "a#b.md",
        ".md",
        "plain.txt",
    ];

    /// What a document might ask for, found or not.
    const TARGETS: &[&str] = &[
        "note",
        "Note",
        "nOTE",
        "other",
        "OTHER",
        "Émile",
        "émile",
        "éMILE",
        "STRASSE",
        "straße",
        "istanbul",
        "İstanbul",
        "日本語",
        "a#b",
        "",
        "gone",
        "sub/note",
        "sub/Deep",
        "Sub/deep",
        "deep",
        "../note",
        "folder",
        "dangling",
        "plain.txt",
    ];

    /// A scratch folder holding the picked `NAMES`, some as folders rather
    /// than files, a subfolder with notes of its own, a link to nothing and
    /// a name that is not UTF-8.
    fn scratch_folder(files: &[bool], folders: &[bool]) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        for (i, name) in NAMES.iter().enumerate() {
            if folders[i] {
                std::fs::create_dir(d.path().join(name)).unwrap();
            } else if files[i] {
                std::fs::write(d.path().join(name), "x").unwrap();
            }
        }
        std::fs::create_dir(d.path().join("sub")).unwrap();
        std::fs::write(d.path().join("sub").join("note.md"), "x").unwrap();
        std::fs::write(d.path().join("sub").join("deep.md"), "x").unwrap();
        std::fs::create_dir(d.path().join("folder.md")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt as _;
            std::os::unix::fs::symlink("nowhere.md", d.path().join("Dangling.md")).unwrap();
            let raw = std::ffi::OsStr::from_bytes(b"caf\xe9.md");
            std::fs::write(d.path().join(raw), "x").unwrap();
        }
        d
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(200))]

        /// One listing or one per link, the answers are the same — in any
        /// order of asking, with repeats.
        #[test]
        fn a_pass_that_lists_the_folder_once_resolves_as_one_that_lists_it_per_link(
            files in proptest::collection::vec(proptest::prelude::any::<bool>(), NAMES.len()),
            folders in proptest::collection::vec(proptest::bool::weighted(0.1), NAMES.len()),
            indexed in proptest::collection::vec(
                (0..NAMES.len(), proptest::sample::select(&["", "a", "b", "a/b", "z/y/x"][..])),
                0..12,
            ),
            asked in proptest::collection::vec(0..TARGETS.len(), 1..40),
        ) {
            let d = scratch_folder(&files, &folders);
            let index: Vec<Entry> = indexed
                .iter()
                .map(|&(name, under)| entry(&d.path().join("elsewhere").join(under).join(NAMES[name])))
                .collect();
            let mut pass = Resolver::new(d.path(), &index);
            for &t in &asked {
                let target = TARGETS[t];
                proptest::prop_assert_eq!(
                    pass.resolve(target),
                    resolve_listing_every_time(target, d.path(), &index),
                    "target {:?}", target
                );
            }
            proptest::prop_assert!(pass.listings() <= 1);
        }
    }

    /// **A document's links cost one listing of its folder, not one each.**
    #[test]
    fn two_thousand_links_list_the_folder_once() {
        let d = tempfile::tempdir().unwrap();
        for i in 0..50 {
            std::fs::write(d.path().join(format!("note{i}.md")), "x").unwrap();
        }
        let mut pass = Resolver::new(d.path(), &[]);
        let mut found = 0;
        for i in 0..2000 {
            // Wrong case, and missing: the two that cannot stop at a `stat`.
            found += usize::from(pass.resolve(&format!("Note{}", i % 100)).is_some());
            assert_eq!(pass.resolve(&format!("gone{i}")), None);
        }
        assert_eq!(found, 1000, "the fifty that exist, twenty times each");
        assert_eq!(pass.listings(), 1);
    }

    /// An exact sibling never needs the listing at all.
    #[test]
    fn exact_siblings_never_list_the_folder() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("note.md"), "x").unwrap();
        let mut pass = Resolver::new(d.path(), &[]);
        assert!(pass.resolve("note").is_some());
        assert_eq!(pass.listings(), 0);
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
