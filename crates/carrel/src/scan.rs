//! Finding markdown files, without ever blocking a frame.
//!
//! NO RATATUI — `scripts/check-discipline.sh` rule 6.
//!
//! # Measured 2026-08-10
//!
//! `ignore::WalkBuilder` over `~/Documents` — 109,857 files, 6,352 directories,
//! warm page cache — found every `.md` in **6 ms** parallel with `.gitignore`
//! on, 10 ms single-threaded, 16 ms with `.gitignore` off. Honouring
//! `.gitignore` is not a cost: it prunes `.git`, `node_modules` and `target`
//! before descending, and is most of the speedup. `find` over the same tree
//! takes 63 ms, which is why that figure does not apply here.
//!
//! **Cold page cache, spinning disks and network mounts are not measured.**
//! That is what the cache is for; do not delete it on the strength of 6 ms.

use std::fmt::Write as _;

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, SystemTime};

/// One discovered markdown file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    pub mtime: SystemTime,
}

/// What a background walk reports.
#[derive(Debug)]
pub enum Msg {
    Found(Entry),
    /// Always sent when the walk ends, so the caller can drop the indicator.
    Done {
        unreadable: usize,
    },
}

fn builder(root: &Path) -> ignore::WalkBuilder {
    let mut b = ignore::WalkBuilder::new(root);
    b.git_ignore(true)
        .git_exclude(true)
        // Honour `.gitignore` even outside a git repository. `ignore` defaults
        // to requiring one, which means a plain directory with a `.gitignore`
        // gets walked in full — surprising, and it loses most of the pruning
        // that makes the walk fast. `fd` exposes the same switch as
        // `--no-require-git`.
        .require_git(false)
        // **`parents` defaults to TRUE**, and that default was doing real
        // damage. It reads `.gitignore` and `.ignore` in every ancestor of the
        // chosen root, so scanning `~/notes` also obeyed `~/.gitignore` — and
        // a `*.md` or `notes/` rule up there emptied the home screen with no
        // note and nothing to look at to find out why. `git_global` was worse
        // in kind: to resolve `core.excludesFile` it reads `~/.gitconfig`,
        // `$XDG_CONFIG_HOME/git/config` and `/etc/gitconfig`, none of which is
        // inside the directory the reader pointed at. README.md:34 and
        // carrel.1 both promise carrel "reads only the directory you point it
        // at"; these two lines are what makes that true rather than nearly
        // true. `git_global` is simply gone — a default of false is what we
        // want and stating it would only invite someone to flip it back.
        .parents(false)
        // Stated because the promise above rests on it and a default is not a
        // decision until someone writes it down: one `ln -s` inside a notes
        // directory would otherwise walk a whole home directory. Tested by
        // `a_symlink_out_of_the_root_is_never_followed`.
        .follow_links(false)
        .hidden(true);
    b
}

fn is_markdown(p: &Path) -> bool {
    p.extension().is_some_and(|x| x == "md" || x == "markdown")
}

fn entry_of(p: &Path) -> Entry {
    let mtime = std::fs::metadata(p)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    Entry {
        path: p.to_path_buf(),
        mtime,
    }
}

/// Walk synchronously, returning the entries and a count of unreadable paths.
///
/// For tests and the non-interactive path only — **never call this from the
/// paint path.**
#[must_use]
pub fn walk_blocking(root: &Path) -> (Vec<Entry>, usize) {
    let mut out = Vec::new();
    let mut unreadable = 0usize;
    for res in builder(root).build() {
        match res {
            Ok(e) if e.file_type().is_some_and(|t| t.is_file()) && is_markdown(e.path()) => {
                out.push(entry_of(e.path()));
            }
            Ok(_) => {}
            Err(_) => unreadable += 1,
        }
    }
    (out, unreadable)
}

/// Start a background walk. Returns immediately.
///
/// The receiver yields entries as they are found and always ends with
/// [`Msg::Done`], so the UI can drop its indicator on any outcome.
#[must_use]
pub fn spawn(root: &Path) -> Receiver<Msg> {
    let (tx, rx) = mpsc::channel();
    let root = root.to_path_buf();
    std::thread::spawn(move || {
        let mut unreadable = 0usize;
        for res in builder(&root).build() {
            match res {
                Ok(e) if e.file_type().is_some_and(|t| t.is_file()) && is_markdown(e.path()) => {
                    // A send error means the UI dropped the receiver — the user
                    // moved on. Stop walking rather than finish into the void.
                    if tx.send(Msg::Found(entry_of(e.path()))).is_err() {
                        return;
                    }
                }
                Ok(_) => {}
                Err(_) => unreadable += 1,
            }
        }
        let _ = tx.send(Msg::Done { unreadable });
    });
    rx
}

/// `$XDG_CACHE_HOME/carrel`, else `~/.cache/carrel`.
#[must_use]
pub fn cache_dir() -> Option<PathBuf> {
    if let Some(x) = std::env::var_os("XDG_CACHE_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(x).join("carrel"));
    }
    std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(|h| PathBuf::from(h).join(".cache").join("carrel"))
}

/// The most `index-*` files kept. Same shape and the same reasoning as
/// [`crate::config::PLACE_CAP`]: one file per root ever opened, each up to
/// 110k lines, accumulating forever, is not state — it is a leak.
const CACHE_CAP: usize = 32;

/// The current cache format, in the file NAME.
///
/// Bumping it invalidates every cache deliberately and instantly: the new
/// build simply does not find the old files, and the eviction pass reclaims
/// them as it goes. A format change that reused the name would instead be
/// read as corruption, which is silent.
const CACHE_PREFIX: &str = "index-1-";

/// One cache file per root, named by a **stable** hash of the root path.
///
/// This was `DefaultHasher`, whose value std explicitly documents as
/// unspecified and free to change between releases. That is fine for a
/// `HashMap` and wrong for a filename that is written today and looked up by
/// a different build tomorrow: a toolchain upgrade silently orphaned every
/// cache on the machine and bought a full cold rescan with no way to
/// diagnose it. FNV-1a is written out here rather than pulled in, because
/// the whole point is that the algorithm cannot move under us, and because
/// this project counts its dependencies by kilobyte.
///
/// A 64-bit hash can collide; two roots sharing a name would show each other's
/// stale list for the fraction of a second before the live walk corrects it.
/// The cache is a hint, never truth — that is already the contract of
/// [`load_cache_in`].
fn cache_name(root: &Path) -> String {
    // FNV-1a, 64-bit: offset basis and prime as specified.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in root.as_os_str().as_encoded_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{CACHE_PREFIX}{h:016x}")
}

/// Delete all but the newest [`CACHE_CAP`] cache files, `keep` always among
/// them.
///
/// `keep` is passed explicitly rather than trusted to be newest: it was just
/// written, but several files written inside one mtime tick sort arbitrarily
/// against each other, and evicting the file the caller is about to read back
/// would be a fine way to make the cache useless on a coarse-grained
/// filesystem.
fn evict_caches(dir: &Path, keep: &str) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut caches: Vec<(SystemTime, PathBuf)> = rd
        .flatten()
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            name.starts_with(CACHE_PREFIX) && name != keep
        })
        .map(|e| {
            let mtime = e
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            (mtime, e.path())
        })
        .collect();
    // Newest first, by name where the clock cannot tell them apart.
    caches.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    for (_, p) in caches.into_iter().skip(CACHE_CAP - 1) {
        // A cache that will not delete is not worth a word to the reader; the
        // next save tries again.
        let _ = std::fs::remove_file(p);
    }
}

#[must_use]
pub fn load_cache(root: &Path) -> Vec<Entry> {
    cache_dir()
        .map(|d| load_cache_in(&d, root))
        .unwrap_or_default()
}

pub fn save_cache(root: &Path, entries: &[Entry]) {
    if let Some(d) = cache_dir() {
        save_cache_in(&d, root, entries);
    }
}

/// `<mtime seconds>\t<path>` per line, the path escaped by
/// [`crate::state::escape_field`] — same reason as the positions file, and
/// the same helper so there is only one answer to what an escaped path is.
///
/// A malformed line is skipped and a corrupt file is simply no cache. **The
/// cache is a hint, never truth** — the live walk is what decides what exists.
#[must_use]
pub fn load_cache_in(dir: &Path, root: &Path) -> Vec<Entry> {
    let Ok(text) = std::fs::read_to_string(dir.join(cache_name(root))) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|l| {
            let (secs, path) = l.split_once('\t')?;
            let secs: u64 = secs.parse().ok()?;
            Some(Entry {
                path: PathBuf::from(crate::state::unescape_field(path)),
                mtime: SystemTime::UNIX_EPOCH + Duration::from_secs(secs),
            })
        })
        .collect()
}

pub fn save_cache_in(dir: &Path, root: &Path, entries: &[Entry]) {
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let mut text = String::with_capacity(entries.len() * 64);
    for e in entries {
        let secs = e
            .mtime
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let _ = writeln!(
            text,
            "{secs}\t{}",
            crate::state::escape_field(&e.path.to_string_lossy())
        );
    }
    // A failed cache write is not worth surfacing: the next run just rescans.
    let name = cache_name(root);
    let _ = crate::state::write_atomic(&dir.join(&name), &text);
    evict_caches(dir, &name);
}

/// The document's own title: `title:` from frontmatter, else the first `#`
/// heading, else nothing.
///
/// **Reads only the head of the file.** A home screen paints fourteen rows;
/// reading 110k files to fill them is not affordable, so this takes the first
/// [`TITLE_BYTES`] and stops. Callers cache by `(path, mtime)`.
#[must_use]
pub fn title_of(path: &Path) -> Option<String> {
    use std::io::Read as _;
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; TITLE_BYTES];
    let n = f.read(&mut buf).ok()?;
    buf.truncate(n);
    let head = String::from_utf8_lossy(&buf);

    let mut in_front = false;
    for (i, line) in head.lines().enumerate() {
        let t = line.trim();
        if i == 0 && (t == "---" || t == "+++") {
            in_front = true;
            continue;
        }
        if in_front {
            if t == "---" || t == "+++" {
                in_front = false;
                continue;
            }
            if let Some(v) = t
                .strip_prefix("title:")
                .or_else(|| t.strip_prefix("title ="))
            {
                let v = v.trim().trim_matches(['"', '\'']).trim();
                if !v.is_empty() {
                    return Some(v.chars().take(120).collect());
                }
            }
            continue;
        }
        if let Some(h) = t.strip_prefix("# ") {
            let h = h.trim();
            if !h.is_empty() {
                return Some(h.chars().take(120).collect());
            }
        }
    }
    None
}

/// How much of a file to read looking for its title. A frontmatter block and
/// a first heading live well inside this.
const TITLE_BYTES: usize = 2048;

#[cfg(test)]
mod title_tests {
    use super::*;

    fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn frontmatter_title_wins_then_the_first_heading() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(
            title_of(&write(
                d.path(),
                "a.md",
                "---\ntitle: The Real Name\ntags: [x]\n---\n\n# Something Else\n"
            )),
            Some("The Real Name".into())
        );
        assert_eq!(
            title_of(&write(d.path(), "b.md", "# Just A Heading\n\nbody\n")),
            Some("Just A Heading".into())
        );
        assert_eq!(
            title_of(&write(d.path(), "c.md", "no title at all\n")),
            None
        );
        // Quoted and TOML forms.
        assert_eq!(
            title_of(&write(d.path(), "d.md", "---\ntitle: \"Quoted\"\n---\n")),
            Some("Quoted".into())
        );
        assert_eq!(
            title_of(&write(d.path(), "e.md", "+++\ntitle = \"Toml\"\n+++\n")),
            Some("Toml".into())
        );
    }

    #[test]
    fn a_heading_far_past_the_head_is_not_read() {
        let d = tempfile::tempdir().unwrap();
        let body = format!("{}\n# Too Late\n", "x".repeat(TITLE_BYTES * 2));
        assert_eq!(title_of(&write(d.path(), "big.md", &body)), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Two `.md` files, one `.txt`, one inside a gitignored directory, one nested.
    fn fixture() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("a.md"), "# a").unwrap();
        fs::write(d.path().join("b.md"), "# b").unwrap();
        fs::write(d.path().join("c.txt"), "not markdown").unwrap();
        fs::write(d.path().join(".gitignore"), "skipped/\n").unwrap();
        fs::create_dir(d.path().join("skipped")).unwrap();
        fs::write(d.path().join("skipped").join("d.md"), "# d").unwrap();
        fs::create_dir(d.path().join("sub")).unwrap();
        fs::write(d.path().join("sub").join("e.md"), "# e").unwrap();
        d
    }

    fn names(entries: &[Entry]) -> Vec<String> {
        let mut v: Vec<String> = entries
            .iter()
            .map(|e| e.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn finds_markdown_and_nothing_else() {
        let d = fixture();
        let (entries, _) = walk_blocking(d.path());
        assert_eq!(names(&entries), vec!["a.md", "b.md", "e.md"]);
    }

    #[test]
    fn honours_gitignore() {
        let d = fixture();
        let (entries, _) = walk_blocking(d.path());
        assert!(
            !names(&entries).contains(&"d.md".to_string()),
            "an ignored directory must not be walked",
        );
    }

    #[test]
    fn a_missing_root_yields_nothing_rather_than_panicking() {
        let (entries, _) = walk_blocking(Path::new("/nonexistent/xyzzy"));
        assert!(entries.is_empty());
    }

    #[test]
    fn the_background_walk_finds_the_same_files() {
        let d = fixture();
        let rx = spawn(d.path());
        let mut found = Vec::new();
        let mut done = false;
        for msg in rx {
            match msg {
                Msg::Found(e) => found.push(e),
                Msg::Done { .. } => done = true,
            }
        }
        assert!(done, "the walk must always report completion");
        assert_eq!(names(&found), vec!["a.md", "b.md", "e.md"]);
    }

    #[test]
    fn the_cache_round_trips() {
        let d = tempfile::tempdir().unwrap();
        let root = Path::new("/some/root");
        assert!(load_cache_in(d.path(), root).is_empty());

        let entries = vec![Entry {
            path: PathBuf::from("/some/root/a.md"),
            mtime: SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        }];
        save_cache_in(d.path(), root, &entries);
        assert_eq!(load_cache_in(d.path(), root), entries);
    }

    #[test]
    fn different_roots_do_not_share_a_cache() {
        let d = tempfile::tempdir().unwrap();
        let e = vec![Entry {
            path: PathBuf::from("/a/x.md"),
            mtime: SystemTime::UNIX_EPOCH,
        }];
        save_cache_in(d.path(), Path::new("/a"), &e);
        assert!(load_cache_in(d.path(), Path::new("/b")).is_empty());
    }

    /// The name is written to disk and read back by a *later build*, so it has
    /// to be a fixed function of the path and nothing else. `DefaultHasher` is
    /// documented by std as unspecified and free to change between releases:
    /// under it, a toolchain upgrade orphans every cache on the machine and
    /// costs a silent cold rescan. This literal is the guard — if it has to be
    /// edited, every existing cache has just been abandoned, and the format
    /// version in the prefix is what should have been bumped instead.
    #[test]
    fn the_cache_name_is_stable_across_builds() {
        assert_eq!(
            cache_name(Path::new("/some/root")),
            "index-1-ba0024f154c745cf"
        );
        assert_eq!(cache_name(Path::new("/a")), "index-1-07d66707b49cd92d");
        assert_eq!(cache_name(Path::new("/b")), "index-1-07d66407b49cd414");
    }

    /// One `index-*` file per root ever opened, each up to 110k lines, and
    /// nothing ever deleted them — the one piece of carrel's state with no
    /// cap, in a project where `PLACE_CAP` and the positions `CAP` are the
    /// house rule.
    #[test]
    fn the_cache_directory_is_capped_and_evicts_the_oldest() {
        let d = tempfile::tempdir().unwrap();
        let e = |n: &str| {
            vec![Entry {
                path: PathBuf::from(n),
                mtime: SystemTime::UNIX_EPOCH,
            }]
        };
        for i in 0..(CACHE_CAP + 12) {
            save_cache_in(d.path(), Path::new(&format!("/root{i}")), &e("/x.md"));
        }
        let files = fs::read_dir(d.path()).unwrap().count();
        assert_eq!(files, CACHE_CAP, "one file per root ever opened, forever");
        // The one just written is certainly not the one evicted.
        let newest = format!("/root{}", CACHE_CAP + 11);
        assert_eq!(load_cache_in(d.path(), Path::new(&newest)), e("/x.md"));
    }

    /// A path may contain a TAB or a NEWLINE; the cache is TAB-separated with
    /// the path last and one entry per line. See `state::escape_field`.
    #[test]
    fn a_cached_path_with_a_tab_or_a_newline_round_trips() {
        let d = tempfile::tempdir().unwrap();
        let root = Path::new("/some/root");
        let entries: Vec<Entry> = ["/r/with\ttab.md", "/r/with\nnewline.md", "/r/with\\a.md"]
            .iter()
            .map(|p| Entry {
                path: PathBuf::from(p),
                mtime: SystemTime::UNIX_EPOCH + Duration::from_secs(9),
            })
            .collect();
        save_cache_in(d.path(), root, &entries);
        assert_eq!(load_cache_in(d.path(), root), entries);
    }

    /// `ignore` defaults `parents` to TRUE, so a scan of `~/notes` also reads
    /// `~/.gitignore` and every `.gitignore` above it. A `*.md` rule up there
    /// empties the home screen with no note and no way to find out why — and
    /// README.md:34 claims carrel "reads only the directory you point it at".
    #[test]
    fn an_ignore_file_above_the_root_is_not_read() {
        let outer = tempfile::tempdir().unwrap();
        fs::write(outer.path().join(".gitignore"), "*.md\nnotes/\n").unwrap();
        let root = outer.path().join("vault");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("kept.md"), "# kept").unwrap();
        fs::create_dir(root.join("notes")).unwrap();
        fs::write(root.join("notes").join("also.md"), "# also").unwrap();

        let (entries, _) = walk_blocking(&root);
        assert_eq!(
            names(&entries),
            vec!["also.md", "kept.md"],
            "a rule outside the chosen root must not reach inside it",
        );
    }

    /// `follow_links` defaults to false, and the privacy claim in README.md:34
    /// and carrel.1 depends on that default rather than on anything either
    /// document says. A symlink is one `ln -s` away from making a scan of a
    /// notes directory read someone's whole home.
    #[test]
    fn a_symlink_out_of_the_root_is_never_followed() {
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("escaped.md"), "# secret").unwrap();
        let d = fixture();
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), d.path().join("away")).unwrap();

        let (entries, _) = walk_blocking(d.path());
        assert!(
            !names(&entries).contains(&"escaped.md".to_string()),
            "the walk left the root through a symlink: {:?}",
            names(&entries),
        );
    }

    #[test]
    fn a_corrupt_cache_is_no_cache_rather_than_an_error() {
        let d = tempfile::tempdir().unwrap();
        let root = Path::new("/some/root");
        save_cache_in(d.path(), root, &[]);
        fs::write(d.path().join(cache_name(root)), "\u{0}not a cache\nnope").unwrap();
        assert!(load_cache_in(d.path(), root).is_empty());
    }
}
