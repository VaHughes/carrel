//! Multi-file content search for the home screen — wave E, Q9.
//!
//! Same streaming shape as `scan.rs`: a background thread walks the already-
//! scanned entry list and sends hits down a channel, so the filesystem never
//! blocks a frame. Stale queries die by GENERATION: the state layer applies
//! only messages stamped with the newest one, and an abandoned thread finds
//! its receiver dropped and stops on the next send.
//!
//! The pattern comes from `carrel_core::content_pattern` — the same
//! compilation the reader uses, so a file the grep reports will light up
//! identically once opened. (Counts run over SOURCE text here and DISPLAY
//! text there, so an entity reference or invisible URL can shift a count by
//! one; accepted, and invisible in practice.)
//!
//! NO RATATUI — `scripts/check-discipline.sh` rule 6.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

use crate::scan::Entry;

/// Skip anything larger: a >4 MiB "markdown file" is a data dump, and the
/// reader itself refuses ≥4 GiB. Keeps a stray artifact from stalling a
/// keystroke's worth of results.
const MAX_BYTES: u64 = 4 * 1024 * 1024;

/// Stop after this many matching files; the picker is for finding a
/// document, not enumerating a corpus.
const MAX_HITS: usize = 500;

/// One matching file.
#[derive(Clone, Debug)]
pub struct Hit {
    pub path: PathBuf,
    /// How many matches inside the file.
    pub count: usize,
    /// The first matching line, trimmed, for the context row.
    pub first_line: String,
}

#[derive(Debug)]
pub enum Msg {
    Hit(Hit, u64),
    Done(u64),
}

/// Search `entries` for `needle` on a background thread. Every message is
/// stamped with `generation`; the caller ignores stale ones.
#[must_use]
pub fn spawn(entries: Vec<Entry>, needle: String, generation: u64) -> Receiver<Msg> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let Some(re) = carrel_core::content_pattern(&needle, true) else {
            let _ = tx.send(Msg::Done(generation));
            return;
        };
        let mut sent = 0usize;
        for e in entries {
            if sent >= MAX_HITS {
                break;
            }
            if let Some(hit) = grep_file(&re, &e.path) {
                if tx.send(Msg::Hit(hit, generation)).is_err() {
                    return; // receiver gone: a newer query took over
                }
                sent += 1;
            }
        }
        let _ = tx.send(Msg::Done(generation));
    });
    rx
}

/// Count matches in one file; `None` when unreadable, oversized, non-UTF-8,
/// or match-free.
fn grep_file(re: &carrel_core::Regex, path: &std::path::Path) -> Option<Hit> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > MAX_BYTES {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    let mut count = 0usize;
    let mut first: Option<(usize, usize)> = None;
    for m in re.find_iter(&text) {
        if first.is_none() {
            first = Some((m.start(), m.end()));
        }
        count += 1;
    }
    let (start, _) = first?;
    let line_start = text[..start].rfind('\n').map_or(0, |i| i + 1);
    let line_end = text[start..].find('\n').map_or(text.len(), |i| start + i);
    let first_line: String = text[line_start..line_end]
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(120)
        .collect();
    Some(Hit {
        path: path.to_path_buf(),
        count,
        first_line,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn entry(p: &std::path::Path) -> Entry {
        Entry {
            path: p.to_path_buf(),
            mtime: SystemTime::UNIX_EPOCH,
        }
    }

    fn collect(rx: &Receiver<Msg>) -> (Vec<Hit>, bool) {
        let mut hits = Vec::new();
        let mut done = false;
        while let Ok(msg) = rx.recv() {
            match msg {
                Msg::Hit(h, _) => hits.push(h),
                Msg::Done(_) => {
                    done = true;
                    break;
                }
            }
        }
        (hits, done)
    }

    #[test]
    fn finds_counts_and_first_lines_across_files() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.md");
        let b = d.path().join("b.md");
        let c = d.path().join("c.md");
        std::fs::write(&a, "intro\nthe needle here\nand a needle again\n").unwrap();
        std::fs::write(&b, "nothing relevant\n").unwrap();
        std::fs::write(&c, "needle at the top\n").unwrap();

        let rx = spawn(vec![entry(&a), entry(&b), entry(&c)], "needle".into(), 7);
        let (hits, done) = collect(&rx);
        assert!(done);
        assert_eq!(hits.len(), 2, "b.md has no match");
        let ha = hits.iter().find(|h| h.path == a).unwrap();
        assert_eq!(ha.count, 2);
        assert_eq!(ha.first_line, "the needle here");
    }

    #[test]
    fn smart_case_matches_like_the_reader() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.md");
        std::fs::write(&a, "Needle NEEDLE needle\n").unwrap();
        let rx = spawn(vec![entry(&a)], "needle".into(), 0);
        let (hits, _) = collect(&rx);
        assert_eq!(hits[0].count, 3, "lowercase needle is insensitive");
        let rx = spawn(vec![entry(&a)], "Needle".into(), 0);
        let (hits, _) = collect(&rx);
        assert_eq!(hits[0].count, 1, "a capital makes it exact");
    }

    #[test]
    fn unreadable_and_empty_needles_finish_cleanly() {
        let rx = spawn(
            vec![entry(std::path::Path::new("/nonexistent/x.md"))],
            "x".into(),
            3,
        );
        let (hits, done) = collect(&rx);
        assert!(hits.is_empty());
        assert!(done);
        let rx = spawn(vec![], "   ".into(), 4);
        let (hits, done) = collect(&rx);
        assert!(hits.is_empty());
        assert!(done, "whitespace needle: immediate Done");
    }

    #[test]
    fn messages_carry_the_generation_stamp() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.md");
        std::fs::write(&a, "needle\n").unwrap();
        let rx = spawn(vec![entry(&a)], "needle".into(), 42);
        let generation = match rx.recv().unwrap() {
            Msg::Hit(_, generation) | Msg::Done(generation) => generation,
        };
        assert_eq!(generation, 42);
    }
}
