//! Backlinks: which documents link *to* the one you are reading.
//!
//! **There is no index, deliberately.** A backlink is a query, and the home
//! screen's scan already gives an entry list that `grep.rs` streams across at
//! a cost a keystroke tolerates. An inverse index would be a second source of
//! truth that has to be kept true across edits, renames and reloads — and the
//! only thing it would buy is a query that is already fast enough.
//!
//! Two stages, cheap first:
//!
//! 1. **Prefilter on the stem.** Any link to `notes/architecture.md` mentions
//!    `architecture` somewhere, whatever form it takes — `[[architecture]]`,
//!    `[[notes/architecture]]`, `](./architecture.md)`. A plain substring scan
//!    over the source rejects almost every file for the price of a read.
//! 2. **Resolve to be sure.** A surviving candidate is parsed and every link
//!    in it resolved the same way the reader would resolve it — wikilinks
//!    through [`crate::wiki::resolve`], relative links against the linking
//!    file's own directory. A file that merely says the word "architecture"
//!    does not become a backlink.
//!
//! Same streaming shape and generation stamping as `grep.rs`, for the same
//! reason: the filesystem never blocks a frame, and an abandoned query dies
//! when its receiver drops.
//!
//! NO RATATUI — `scripts/check-discipline.sh` rule 6.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use crate::scan::Entry;

/// A relative path written as a link destination that survives being read
/// back as one.
///
/// A destination is a URL on the way in: split at `#` for a fragment, at `?`
/// by the links pane, copied rather than opened if it seems to carry a
/// scheme, entity-decoded by the parser, and ended by a newline or a `>`.
/// A file may be called `C# notes.md`, `re: plan.md` or `a&amp;b.md`, so
/// every character that means something to any of those readers is
/// percent-encoded, and [`resolve_local`] decodes on the other side. For the
/// documents carrel generates itself — search results, the tags page —
/// whose targets come from a folder it did not write.
#[must_use]
pub fn encode_target(rel: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(rel.len() + 8);
    for c in rel.chars() {
        if c.is_control() || matches!(c, '%' | '#' | '?' | ':' | '&' | '<' | '>' | '\\') {
            let mut buf = [0u8; 4];
            for b in c.encode_utf8(&mut buf).bytes() {
                let _ = write!(out, "%{b:02X}");
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// `%XX` escapes decoded, or `None` when there are none or the result is
/// not text.
#[must_use]
pub fn percent_decoded(s: &str) -> Option<String> {
    if !s.contains('%') {
        return None;
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && let (Some(hi), Some(lo)) = (
                bytes.get(i + 1).copied().and_then(hex),
                bytes.get(i + 2).copied().and_then(hex),
            )
        {
            out.push(u8::try_from(hi * 16 + lo).unwrap_or(b'?'));
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The names a relative link destination might mean, most literal first.
///
/// **The name as written wins.** Failing that it is tried percent-decoded —
/// `[notes](my%20notes.md)` is how every other tool writes a link to a name
/// with a space — and then without a `?query`, which a web view reads and a
/// file system does not (`guide.md?plain=1`).
#[must_use]
pub fn candidates(bare: &str) -> Vec<String> {
    let mut out = vec![bare.to_string()];
    let mut add = |c: Option<String>| {
        if let Some(c) = c
            && !c.is_empty()
            && !out.contains(&c)
        {
            out.push(c);
        }
    };
    add(percent_decoded(bare));
    if let Some((path, _)) = bare.split_once('?') {
        add(Some(path.to_string()));
        add(percent_decoded(path));
    }
    out
}

/// The file a relative link destination names: the first of its
/// [`candidates`] that exists, so a file really called `50%20off.md` still
/// opens. When none does, the decoded form, which is the one worth naming
/// in the message that says so.
#[must_use]
pub fn resolve_local(dir: &Path, bare: &str) -> PathBuf {
    let names = candidates(bare);
    names
        .iter()
        .map(|n| dir.join(n))
        .find(|p| p.exists())
        .unwrap_or_else(|| dir.join(names.get(1).unwrap_or(&names[0])))
}

/// Skip anything larger, exactly as the content grep does.
const MAX_BYTES: u64 = 4 * 1024 * 1024;

/// Stop after this many. A backlinks pane answers "who points here", not
/// "enumerate the corpus".
pub const MAX_HITS: usize = 200;

/// One document that links to the target.
#[derive(Clone, Debug)]
pub struct Backlink {
    pub path: PathBuf,
    /// The first line that carries the link, trimmed, for the context row.
    pub line: String,
}

#[derive(Debug)]
pub enum Msg {
    Found(Backlink, u64),
    /// The search stopped at [`MAX_HITS`] with files still unread, so what
    /// was found is the first of them and not a count. Sent before `Done`.
    Capped(u64),
    Done(u64),
}

/// The stem a link would have to mention: the file name without `.md`.
#[must_use]
pub fn stem_of(target: &Path) -> String {
    target
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Canonical form for comparing two paths that may be spelled differently.
fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// Does `src`, living at `from`, link to `target`?
///
/// Pure but for the canonicalisation, so the rule is testable: parse, resolve
/// every link the way the reader would, and compare.
#[must_use]
pub fn links_to(src: &str, from: &Path, target: &Path, index: &[Entry]) -> bool {
    let doc = carrel_core::Document::parse(src);
    let dir = from.parent().unwrap_or(Path::new("."));
    let mut wiki = crate::wiki::Resolver::new(dir, index);
    for (i, link) in doc.links.iter().enumerate() {
        let id = carrel_core::LinkId(u32::try_from(i).unwrap_or(u32::MAX));
        let resolved = if doc.is_wikilink(id) {
            wiki.resolve(link)
        } else {
            // A relative link, the reader's own rule: relative to the file
            // that carries it. Anything with a scheme is not a local link.
            if link.contains("://") || link.starts_with('#') {
                None
            } else {
                let bare = link.split(['#', '?']).next().unwrap_or(link);
                (!bare.is_empty()).then(|| dir.join(bare))
            }
        };
        if resolved.is_some_and(|p| same_file(&p, target)) {
            return true;
        }
    }
    false
}

/// The first line of `src` that mentions `stem`, trimmed — the context row.
fn context_line(src: &str, stem: &str) -> String {
    src.lines()
        .find(|l| l.contains(stem))
        .map(|l| {
            let t = l.trim();
            t.chars().take(200).collect()
        })
        .unwrap_or_default()
}

/// A running [`spawn_under`] query. Dropping it stops the worker at its
/// next file, hit or no hit: a walk of a large tree that finds nothing
/// sends nothing, so a closed channel alone would never be noticed.
#[derive(Debug)]
pub struct Query {
    pub rx: Receiver<Msg>,
    _alive: std::sync::Arc<()>,
}

/// Find documents linking to `target`, on a background thread that also
/// does the walk and searches each file as the walk reaches it: first
/// under `near` — the document's own folder, where an answer is likeliest
/// and soonest — then under the rest of `root`.
///
/// Walking the whole of `root` before searching any of it said `Looking…`
/// for twenty seconds from a home directory, and could not be stopped.
#[must_use]
pub fn spawn_under(near: PathBuf, root: PathBuf, target: PathBuf, generation: u64) -> Query {
    let (tx, rx) = mpsc::channel();
    let alive = std::sync::Arc::new(());
    let watch = std::sync::Arc::downgrade(&alive);
    std::thread::spawn(move || {
        let stem = stem_of(&target);
        let mut sent = 0usize;
        // Set when a file comes up after the last hit the cap allows.
        let capped = std::cell::Cell::new(false);
        let mut visit = |e: Entry, skip: Option<&Path>| -> bool {
            if watch.strong_count() == 0 {
                return false;
            }
            if sent >= MAX_HITS {
                // The document itself is not a file left unread.
                if same_file(&e.path, &target) {
                    return true;
                }
                capped.set(true);
                return false;
            }
            if skip.is_some_and(|near| e.path.starts_with(near)) {
                return true;
            }
            if let Some(hit) = backlink(&e.path, &target, &stem) {
                if tx.send(Msg::Found(hit, generation)).is_err() {
                    return false;
                }
                sent += 1;
            }
            true
        };
        if !stem.is_empty() {
            let whole = crate::scan::walk_while(&near, |e| visit(e, None));
            if whole && !same_file(&near, &root) {
                crate::scan::walk_while(&root, |e| visit(e, Some(&near)));
            }
        }
        if capped.get() {
            let _ = tx.send(Msg::Capped(generation));
        }
        let _ = tx.send(Msg::Done(generation));
    });
    Query { rx, _alive: alive }
}

/// Find documents linking to `target`, on a background thread.
///
/// Every message carries `generation`; the caller ignores stale ones, exactly
/// as the content grep does.
#[must_use]
pub fn spawn(entries: Vec<Entry>, target: PathBuf, generation: u64) -> Receiver<Msg> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let stem = stem_of(&target);
        let mut sent = 0usize;
        for e in entries {
            if stem.is_empty() {
                break;
            }
            if sent >= MAX_HITS {
                let _ = tx.send(Msg::Capped(generation));
                break;
            }
            if let Some(hit) = backlink(&e.path, &target, &stem) {
                if tx.send(Msg::Found(hit, generation)).is_err() {
                    return; // the query was abandoned
                }
                sent += 1;
            }
        }
        let _ = tx.send(Msg::Done(generation));
    });
    rx
}

/// `path` as a backlink to `target`, if it is one.
fn backlink(path: &Path, target: &Path, stem: &str) -> Option<Backlink> {
    // A document is not its own backlink.
    if same_file(path, target) {
        return None;
    }
    // Regular files only, and by the handle: a FIFO named `notes.md`
    // would hold the worker for good.
    let (file, len) = crate::state::open_regular(path).ok()?;
    if len > MAX_BYTES {
        return None;
    }
    let src = std::io::read_to_string(std::io::Read::take(file, MAX_BYTES)).ok()?;
    // Stage 1: the cheap rejection. Stage 2: be sure.
    if !src.contains(stem) || !links_to(&src, path, target, &[]) {
        return None;
    }
    Some(Backlink {
        line: context_line(&src, stem),
        path: path.to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn a_relative_link_counts_and_a_mere_mention_does_not() {
        let d = tempfile::tempdir().unwrap();
        let target = write(d.path(), "architecture.md", "# Arch\n");
        let linker = write(d.path(), "a.md", "See [the spec](architecture.md).\n");
        let mentioner = write(d.path(), "b.md", "I have opinions about architecture.\n");

        assert!(links_to(
            &std::fs::read_to_string(&linker).unwrap(),
            &linker,
            &target,
            &[]
        ));
        assert!(
            !links_to(
                &std::fs::read_to_string(&mentioner).unwrap(),
                &mentioner,
                &target,
                &[]
            ),
            "saying the word is not linking"
        );
    }

    #[test]
    fn a_link_resolves_against_the_file_that_carries_it_not_the_root() {
        let d = tempfile::tempdir().unwrap();
        let target = write(d.path(), "docs/spec.md", "# Spec\n");
        // A sibling inside docs/ links with a bare name…
        let sibling = write(d.path(), "docs/notes.md", "see [it](spec.md)\n");
        assert!(links_to(
            &std::fs::read_to_string(&sibling).unwrap(),
            &sibling,
            &target,
            &[]
        ));
        // …and one at the root has to walk down, or it is not a link to this.
        let root = write(d.path(), "top.md", "see [it](spec.md)\n");
        assert!(
            !links_to(
                &std::fs::read_to_string(&root).unwrap(),
                &root,
                &target,
                &[]
            ),
            "docs/spec.md is not ./spec.md from the root"
        );
        let root_ok = write(d.path(), "top2.md", "see [it](docs/spec.md)\n");
        assert!(links_to(
            &std::fs::read_to_string(&root_ok).unwrap(),
            &root_ok,
            &target,
            &[]
        ));
    }

    #[test]
    fn a_wikilink_counts_through_the_readers_own_resolver() {
        let d = tempfile::tempdir().unwrap();
        let target = write(d.path(), "architecture.md", "# Arch\n");
        let linker = write(d.path(), "a.md", "see [[architecture]] for why\n");
        let index = vec![crate::scan::Entry {
            path: target.clone(),
            mtime: std::time::SystemTime::UNIX_EPOCH,
        }];
        assert!(links_to(
            &std::fs::read_to_string(&linker).unwrap(),
            &linker,
            &target,
            &index
        ));
    }

    #[test]
    fn an_external_url_is_never_a_backlink() {
        let d = tempfile::tempdir().unwrap();
        let target = write(d.path(), "spec.md", "# S\n");
        let linker = write(d.path(), "a.md", "see [x](https://example.com/spec.md)\n");
        assert!(!links_to(
            &std::fs::read_to_string(&linker).unwrap(),
            &linker,
            &target,
            &[]
        ));
    }

    #[test]
    fn the_query_streams_and_skips_the_document_itself() {
        let d = tempfile::tempdir().unwrap();
        let target = write(d.path(), "hub.md", "# Hub, which mentions hub\n");
        write(d.path(), "one.md", "see [hub](hub.md)\n");
        write(d.path(), "two.md", "no link, just the word hub\n");
        let entries: Vec<_> = ["hub.md", "one.md", "two.md"]
            .iter()
            .map(|n| crate::scan::Entry {
                path: d.path().join(n),
                mtime: std::time::SystemTime::UNIX_EPOCH,
            })
            .collect();

        let rx = spawn(entries, target, 7);
        let mut found = Vec::new();
        for msg in rx {
            match msg {
                Msg::Found(b, g) => {
                    assert_eq!(g, 7, "every message is stamped");
                    found.push(b);
                }
                Msg::Done(g) => {
                    assert_eq!(g, 7);
                    break;
                }
                Msg::Capped(_) => panic!("one hit is not the cap"),
            }
        }
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].path.ends_with("one.md"));
        assert!(found[0].line.contains("hub"));
    }

    fn found(q: &Query) -> Vec<PathBuf> {
        let mut out = Vec::new();
        loop {
            match q.rx.recv_timeout(std::time::Duration::from_secs(20)) {
                Ok(Msg::Found(b, _)) => out.push(b.path),
                Ok(Msg::Capped(_)) => {}
                Ok(Msg::Done(_)) => return out,
                Err(e) => panic!("the query never finished: {e}"),
            }
        }
    }

    /// The document's own folder is searched first and the rest of the
    /// root after it, each file once however the two folders overlap.
    #[test]
    fn a_query_under_a_root_answers_from_the_near_folder_first_and_once() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        let target = write(&root, "docs/guide.md", "# Guide\n");
        let sibling = write(&root, "docs/other.md", "see [g](guide.md)\n");
        let above = write(&root, "README.md", "read [the guide](docs/guide.md)\n");
        write(&root, "docs/deep/unrelated.md", "guide, but no link\n");
        let q = spawn_under(root.join("docs"), root.clone(), target.clone(), 0);
        assert_eq!(found(&q), vec![sibling.clone(), above]);
        // The root is the near folder: nothing is walked twice.
        let q = spawn_under(root.join("docs"), root.join("docs"), target, 0);
        assert_eq!(found(&q), vec![sibling]);
    }

    /// A FIFO named like a document is passed over, not opened: reading it
    /// would hold the worker until something wrote to it.
    #[cfg(unix)]
    #[test]
    fn a_fifo_named_like_a_document_does_not_hold_the_query() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        let target = write(&root, "guide.md", "# Guide\n");
        let sibling = write(&root, "other.md", "see [g](guide.md)\n");
        let fifo = root.join("pipe.md");
        let made = std::process::Command::new("mkfifo").arg(&fifo).status();
        if !made.is_ok_and(|s| s.success()) {
            eprintln!("SKIP: mkfifo not available");
            return;
        }
        let q = spawn_under(root.clone(), root, target, 0);
        assert_eq!(found(&q), vec![sibling]);
    }

    /// A walk stops at the file its caller says stop at.
    #[test]
    fn a_walk_stops_when_told() {
        let d = tempfile::tempdir().unwrap();
        for i in 0..20 {
            write(d.path(), &format!("n{i}.md"), "x\n");
        }
        let mut seen = 0;
        let whole = crate::scan::walk_while(d.path(), |_| {
            seen += 1;
            seen < 3
        });
        assert!(!whole && seen == 3, "{whole} {seen}");
    }

    /// The cap is said, not just kept to: a search that stopped with files
    /// still to read reports it, and one that read everything does not —
    /// even when what it found is exactly the cap.
    #[test]
    fn a_search_the_cap_stopped_says_so_and_one_that_finished_does_not() {
        for (linkers, capped) in [(MAX_HITS, false), (MAX_HITS + 1, true)] {
            let d = tempfile::tempdir().unwrap();
            let target = write(d.path(), "hub.md", "# Hub\n");
            for i in 0..linkers {
                write(d.path(), &format!("n{i}.md"), "see [hub](hub.md)\n");
            }
            let drain = |rx: &Receiver<Msg>| {
                let (mut found, mut said) = (0, false);
                loop {
                    match rx.recv_timeout(std::time::Duration::from_secs(20)) {
                        Ok(Msg::Found(..)) => found += 1,
                        Ok(Msg::Capped(g)) => {
                            assert_eq!(g, 3);
                            said = true;
                        }
                        Ok(Msg::Done(_)) => return (found, said),
                        Err(e) => panic!("the query never finished: {e}"),
                    }
                }
            };
            let root = d.path().to_path_buf();
            let q = spawn_under(root.clone(), root.clone(), target.clone(), 3);
            assert_eq!(
                drain(&q.rx),
                (MAX_HITS, capped),
                "{linkers} linkers, walked"
            );
            let entries: Vec<_> = (0..linkers)
                .map(|i| crate::scan::Entry {
                    path: root.join(format!("n{i}.md")),
                    mtime: std::time::SystemTime::UNIX_EPOCH,
                })
                .collect();
            let rx = spawn(entries, target, 3);
            assert_eq!(drain(&rx), (MAX_HITS, capped), "{linkers} linkers, listed");
        }
    }
}
