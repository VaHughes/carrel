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

/// A path with its `.` and `..` components resolved by hand. No disk: the
/// point is to know where a path leads before deciding whether to look.
#[must_use]
pub fn fold_lexically(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// How far a document's links may make a reader look: the folder rule.
///
/// A document is untrusted. A link in it that leads out of the library —
/// `../outside/secret`, `/etc/hostname` — must not be looked at: a `stat`
/// of wherever a document points (a network mount, another user's files)
/// is a thing it should not be able to make a reader do by being opened or
/// searched. So a link's target is folded WITHOUT touching the disk, and
/// only one that stays inside is ever asked about.
///
/// "Inside" is the library root when there is one, and the document's own
/// folder when there is not.
#[derive(Debug, Clone)]
pub struct Bounds {
    here: PathBuf,
    within: PathBuf,
}

impl Bounds {
    /// For a document in `here`, in a library rooted at `root` (if any).
    /// Both are places the reader is already in, so making them absolute —
    /// which follows symlinks in THEM — asks nothing a document chose.
    #[must_use]
    pub fn new(here: &Path, root: Option<&Path>) -> Self {
        let absolute = |dir: &Path| {
            let dir = if dir.as_os_str().is_empty() {
                Path::new(".")
            } else {
                dir
            };
            dir.canonicalize()
                .or_else(|_| std::path::absolute(dir))
                .unwrap_or_else(|_| dir.to_path_buf())
        };
        let here = absolute(here);
        let within = root.map_or_else(|| here.clone(), absolute);
        Self { here, within }
    }

    /// Where `target` — a path as a link writes it, relative to the
    /// document — leads, when that is inside; `None` when it is absolute
    /// or leaves. No disk is touched either way.
    #[must_use]
    pub fn admit(&self, target: &Path) -> Option<PathBuf> {
        if target.is_absolute() {
            return None;
        }
        let folded = fold_lexically(&self.here.join(target));
        folded.starts_with(&self.within).then_some(folded)
    }
}

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
    /// The library root, when the caller has one: see [`Bounds`].
    root: Option<&'a Path>,
    /// Made on the first target, not before: a pass with no wikilink in it
    /// (most files a backlink scan reads) asks the disk nothing at all.
    bounds: Option<Bounds>,
    /// Every path this pass asked the disk about, for the tests of the
    /// folder rule.
    #[cfg(test)]
    touched: Vec<PathBuf>,
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
            root: None,
            bounds: None,
            #[cfg(test)]
            touched: Vec::new(),
            siblings: None,
            indexed: None,
            answers: HashMap::new(),
            listings: 0,
        }
    }

    /// The same pass, in a library rooted at `root`: a target may then
    /// lead anywhere inside the library, not only inside the document's
    /// own folder.
    #[must_use]
    pub fn within(mut self, root: Option<&'a Path>) -> Self {
        self.root = root;
        self.bounds = None;
        self
    }

    /// Is `path` a file? The one place a target's own path is asked about.
    fn is_file(&mut self, path: &Path) -> bool {
        #[cfg(test)]
        self.touched.push(path.to_path_buf());
        path.is_file()
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

        // 0. The folder rule, before anything is asked of the disk: a
        // target that is absolute or leads out of the library is not a
        // note here, and is not looked at to find out.
        let (here, root) = (self.here, self.root);
        self.bounds
            .get_or_insert_with(|| Bounds::new(here, root))
            .admit(Path::new(&want))?;

        // 1. Exact sibling.
        let exact = self.here.join(&want);
        if self.is_file(&exact) {
            return Some(exact);
        }

        // 2. Case-insensitive sibling: the first in path order that is a
        // file. Asked of the disk per candidate, not per entry — a folder
        // of thousands is not `stat`ed to find the one name that matters.
        let want_lower = want.to_lowercase();
        let here = self.here;
        let listings = &mut self.listings;
        #[cfg(test)]
        if self.siblings.is_none() {
            self.touched.push(here.to_path_buf());
        }
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

    /// **A wikilink cannot make the reader look outside the folder.**
    ///
    /// `[[../outside/secret]]` and `[[/etc/hostname]]` were joined onto the
    /// folder and `stat`ed as written, so a document chose which paths the
    /// reader asked the disk about — and a note that happened to be there
    /// became a link out of the library. A target is folded first, and one
    /// that leaves is refused with nothing asked: the resolver records
    /// every path it touches, and for these it records none.
    #[test]
    fn a_target_that_leaves_the_folder_is_refused_before_the_disk_is_asked() {
        let d = tempfile::tempdir().unwrap();
        let vault = d.path().join("vault");
        let outside = d.path().join("outside");
        std::fs::create_dir_all(vault.join("sub")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        // Really there, so a look would find them.
        std::fs::write(outside.join("secret.md"), "x").unwrap();
        std::fs::write(vault.join("inside.md"), "x").unwrap();
        std::fs::write(vault.join("sub/deep.md"), "x").unwrap();
        let absolute = outside.join("secret");
        let absolute = absolute.to_str().unwrap();

        let mut pass = Resolver::new(&vault, &[]);
        for target in [
            "../outside/secret",
            "sub/../../outside/secret",
            "../vault/../outside/secret",
            "/etc/hostname",
            absolute,
            "../..",
        ] {
            assert_eq!(pass.resolve(target), None, "{target}");
        }
        assert_eq!(pass.touched, Vec::<PathBuf>::new(), "nothing was looked at");
        assert_eq!(pass.listings(), 0, "not even the folder");

        // Inside, in any spelling, it is looked for — and only inside.
        assert_eq!(pass.resolve("inside"), Some(vault.join("inside.md")));
        assert_eq!(pass.resolve("sub/deep"), Some(vault.join("sub/deep.md")));
        assert_eq!(
            pass.resolve("sub/../inside"),
            Some(vault.join("sub/../inside.md"))
        );
        assert_eq!(
            pass.resolve("../vault/inside"),
            Some(vault.join("../vault/inside.md"))
        );
        assert_eq!(pass.resolve("gone"), None);
        let vault_real = vault.canonicalize().unwrap();
        for path in &pass.touched {
            let seen = fold_lexically(&vault_real.join(path.strip_prefix(&vault).unwrap()));
            assert!(seen.starts_with(&vault_real), "{path:?} was looked at");
        }

        // In a library rooted above, the neighbor folder is inside it; what
        // leaves the LIBRARY is still not looked at.
        let mut pass = Resolver::new(&vault, &[]).within(Some(d.path()));
        assert_eq!(
            pass.resolve("../outside/secret"),
            Some(vault.join("../outside/secret.md"))
        );
        let looked = pass.touched.len();
        for target in ["../../x", "/etc/hostname", absolute] {
            assert_eq!(pass.resolve(target), None, "{target}");
        }
        assert_eq!(pass.touched.len(), looked, "nothing more was looked at");
    }

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
