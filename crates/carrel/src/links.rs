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

/// Skip anything larger, exactly as the content grep does.
const MAX_BYTES: u64 = 4 * 1024 * 1024;

/// Stop after this many. A backlinks pane answers "who points here", not
/// "enumerate the corpus".
const MAX_HITS: usize = 200;

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
    for (i, link) in doc.links.iter().enumerate() {
        let id = carrel_core::LinkId(u32::try_from(i).unwrap_or(u32::MAX));
        let resolved = if doc.is_wikilink(id) {
            crate::wiki::resolve(link, dir, index)
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

/// Find documents linking to `target`, on a background thread.
///
/// Every message carries `generation`; the caller ignores stale ones, exactly
/// as the content grep does.
#[must_use]
pub fn spawn(entries: Vec<Entry>, target: PathBuf, generation: u64) -> Receiver<Msg> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let stem = stem_of(&target);
        if stem.is_empty() {
            let _ = tx.send(Msg::Done(generation));
            return;
        }
        let mut sent = 0usize;
        for e in entries {
            if sent >= MAX_HITS {
                break;
            }
            // A document is not its own backlink.
            if same_file(&e.path, &target) {
                continue;
            }
            if std::fs::metadata(&e.path).is_ok_and(|m| m.len() > MAX_BYTES) {
                continue;
            }
            let Ok(src) = std::fs::read_to_string(&e.path) else {
                continue;
            };
            // Stage 1: the cheap rejection.
            if !src.contains(&stem) {
                continue;
            }
            // Stage 2: be sure.
            if !links_to(&src, &e.path, &target, &[]) {
                continue;
            }
            let hit = Backlink {
                line: context_line(&src, &stem),
                path: e.path.clone(),
            };
            if tx.send(Msg::Found(hit, generation)).is_err() {
                return; // the query was abandoned
            }
            sent += 1;
        }
        let _ = tx.send(Msg::Done(generation));
    });
    rx
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
            }
        }
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].path.ends_with("one.md"));
        assert!(found[0].line.contains("hub"));
    }
}
