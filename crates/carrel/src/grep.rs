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

/// How many matching lines travel with a hit. The count stays total, but a
/// results document that quoted ten thousand lines would be a second copy of
/// the library; eight shows the shape and the file itself shows the rest.
const MATCH_LINES: usize = 8;

/// One matching line inside a hit file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HitLine {
    /// 1-based source line number — what the reader's `42G` and a `#L12`
    /// link both mean.
    pub lineno: u32,
    /// The line itself, trimmed, for display.
    pub line: String,
}

/// One matching file.
#[derive(Clone, Debug)]
pub struct Hit {
    pub path: PathBuf,
    /// How many matches inside the file.
    pub count: usize,
    /// The first matching line, trimmed, for the context row.
    pub first_line: String,
    /// The first few matching lines, for the results document.
    pub matches: Vec<HitLine>,
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
    let mut matches: Vec<HitLine> = Vec::new();
    for m in re.find_iter(&text) {
        if matches.len() < MATCH_LINES {
            let (lineno, line) = context_line(&text, m.start());
            matches.push(HitLine { lineno, line });
        }
        count += 1;
    }
    let first = matches.first()?;
    Some(Hit {
        path: path.to_path_buf(),
        count,
        first_line: first.line.clone(),
        matches,
    })
}

/// The 1-based number and trimmed text of the source line holding `byte`.
fn context_line(text: &str, byte: usize) -> (u32, String) {
    let line_start = text[..byte].rfind('\n').map_or(0, |i| i + 1);
    let line_end = text[byte..].find('\n').map_or(text.len(), |i| byte + i);
    let lineno = u32::try_from(text[..byte].matches('\n').count() + 1).unwrap_or(u32::MAX);
    let line: String = text[line_start..line_end]
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(120)
        .collect();
    (lineno, line)
}

/// Render search hits as a markdown document: a section per file with a link
/// per match, so the outline, folding, breadcrumb and in-document search all
/// work on results with no new machinery — the same idea as
/// `carrel_core::diff`, which turns a diff into a document for the same
/// reason.
///
/// Every match links to its file at its line (`path#L12`, GitHub's own
/// anchor shape, which the reader jumps to as a row). Paths read relative
/// to `root`, targets included, so the links resolve wherever the document
/// is opened from — the opener points link resolution at the same root.
#[must_use]
pub fn render_results(root: &std::path::Path, query: &str, hits: &[Hit]) -> String {
    use std::fmt::Write as _;
    fn plural(n: usize, one: &str, many: &str) -> String {
        if n == 1 {
            format!("1 {one}")
        } else {
            format!("{n} {many}")
        }
    }
    let total: usize = hits.iter().map(|h| h.count).sum();
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# {} match {:?}\n",
        plural(hits.len(), "file", "files"),
        query
    );
    let _ = writeln!(
        out,
        "_{} in {}. Every line below is a link._\n",
        plural(total, "match", "matches"),
        plural(hits.len(), "file", "files"),
    );
    for h in hits {
        let rel = h
            .path
            .strip_prefix(root)
            .unwrap_or(&h.path)
            .display()
            .to_string();
        let _ = writeln!(out, "## {rel} — {}", plural(h.count, "match", "matches"));
        for m in &h.matches {
            // Angle-bracket targets, because a vault path may hold a space;
            // a code span for the text, because it may hold markdown.
            let _ = writeln!(
                out,
                "- [{}](<{rel}#L{}>): `{}`",
                m.lineno,
                m.lineno,
                m.line.replace('`', "’"),
            );
        }
        if h.count > h.matches.len() {
            let more = h.count - h.matches.len();
            let _ = writeln!(out, "- […{more} more in {rel}](<{rel}>)");
        }
        out.push('\n');
    }
    out
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
    fn hits_carry_line_numbers_capped_per_file() {
        use std::fmt::Write as _;
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.md");
        let mut body = String::from("nothing here\n");
        for i in 0..20 {
            let _ = writeln!(body, "line {i} with a needle in it");
        }
        std::fs::write(&a, &body).unwrap();
        let rx = spawn(vec![entry(&a)], "needle".into(), 0);
        let (hits, _) = collect(&rx);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].count, 20, "the count stays total");
        assert_eq!(hits[0].matches.len(), MATCH_LINES, "the lines are capped");
        assert_eq!(hits[0].matches[0].lineno, 2);
        assert!(hits[0].matches[0].line.contains("line 0"));
        assert_eq!(hits[0].first_line, hits[0].matches[0].line);
    }

    #[test]
    fn the_results_document_links_every_match_to_its_line() {
        let root = std::path::Path::new("/vault");
        let hits = vec![
            Hit {
                path: root.join("notes/arch.md"),
                count: 3,
                first_line: "a needle".into(),
                matches: vec![
                    HitLine {
                        lineno: 12,
                        line: "a needle".into(),
                    },
                    HitLine {
                        lineno: 45,
                        line: "another `tricky` [one]".into(),
                    },
                ],
            },
            Hit {
                path: root.join("solo.md"),
                count: 1,
                first_line: "needle".into(),
                matches: vec![HitLine {
                    lineno: 1,
                    line: "needle".into(),
                }],
            },
        ];
        let doc = render_results(root, "needle", &hits);
        assert!(doc.contains("# 2 files match \"needle\""), "{doc}");
        assert!(doc.contains("## notes/arch.md — 3 matches"), "{doc}");
        assert!(
            doc.contains("- [12](<notes/arch.md#L12>): `a needle`"),
            "{doc}"
        );
        // Markdown inside a match cannot break the link: the text rides in a
        // code span, with its backticks disarmed.
        assert!(
            doc.contains("`another ’tricky’ [one]`"),
            "code span, not markup: {doc}"
        );
        // The capped file says where the rest went, as a link to the file.
        assert!(
            doc.contains("[…1 more in notes/arch.md](<notes/arch.md>)"),
            "{doc}"
        );
        // And the whole thing parses: sections for the outline, links to click.
        let parsed = carrel_core::Document::parse(&doc);
        assert_eq!(parsed.links.len(), 4, "two matches + more + solo: {doc}");
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
