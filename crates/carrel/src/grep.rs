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
//! identically once opened. The SOURCE is only the filter: a file whose
//! source holds no match is passed over at regex speed, which is nearly
//! every file. One that holds a match is parsed and counted over its
//! DISPLAY text, exactly as the reader will count it. Counting the source
//! disagreed wherever the display differs: a hyphen run becomes however
//! many dashes smart punctuation decides (`---` is `—`, which holds no
//! `--`), except in code, which the source cannot tell apart; and a match
//! inside a link's URL is never shown. The pattern carries both spellings
//! of every glyph smart punctuation rewrites, so the filter passes `don't`
//! for a reader showing `don’t`
//! (`the_grep_count_is_what_the_reader_finds_through_smart_punctuation`).
//! What the filter cannot see is accepted: a match the source does not
//! spell — through an entity (`&ndash;`) or across markup (`` `--`x ``) —
//! is found once the file is open, and not here.
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
        let Some(q) = Query::new(&needle) else {
            let _ = tx.send(Msg::Done(generation));
            return;
        };
        let mut sent = 0usize;
        for e in entries {
            if sent >= MAX_HITS {
                break;
            }
            if let Some(hit) = grep_file(&q, &e.path) {
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

/// A needle, and the source filters for it.
struct Query {
    needle: String,
    /// For a source with no soft hyphen — nearly all of them.
    plain: carrel_core::Regex,
    /// For one with a soft hyphen, which may sit inside any word.
    soft: carrel_core::Regex,
}

impl Query {
    /// `None` for an empty needle.
    fn new(needle: &str) -> Option<Self> {
        Some(Self {
            needle: needle.to_owned(),
            plain: carrel_core::content_pattern(needle, true)?,
            soft: carrel_core::content_pattern_soft(needle, true)?,
        })
    }
}

/// Count matches in one file; `None` when unreadable, oversized, non-UTF-8,
/// or match-free.
fn grep_file(q: &Query, path: &std::path::Path) -> Option<Hit> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > MAX_BYTES {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    // The source is the cheap filter: almost every file fails it, at
    // regex speed. One that passes is parsed and counted by the reader's
    // own search, over the display text.
    let filter = if text.contains('\u{AD}') {
        &q.soft
    } else {
        &q.plain
    };
    filter.find(&text)?;
    let doc = carrel_core::Document::parse(&text);
    let found = carrel_core::search(&doc, &q.needle, true);
    let count = found.len();
    let matches: Vec<HitLine> = found
        .ranges
        .iter()
        .take(MATCH_LINES)
        .map(|r| {
            let lineno = doc.line_of(carrel_core::DocByte(r.start));
            HitLine {
                lineno,
                line: source_line(&text, lineno),
            }
        })
        .collect();
    let first = matches.first()?;
    Some(Hit {
        path: path.to_path_buf(),
        count,
        first_line: first.line.clone(),
        matches,
    })
}

/// The trimmed text of 1-based source line `lineno`.
fn source_line(text: &str, lineno: u32) -> String {
    text.split('\n')
        .nth(lineno.saturating_sub(1) as usize)
        .unwrap_or_default()
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(120)
        .collect()
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
        // Encoded, because the path is read back as a URL: a `#` in a file
        // name would otherwise be taken for the fragment that follows it.
        let target = crate::links::encode_target(&rel);
        for m in &h.matches {
            // Angle-bracket targets, because a vault path may hold a space;
            // a code span for the text, because it may hold markdown.
            let _ = writeln!(
                out,
                "- [{}](<{target}#L{}>): `{}`",
                m.lineno,
                m.lineno,
                m.line.replace('`', "’"),
            );
        }
        if h.count > h.matches.len() {
            let more = h.count - h.matches.len();
            let _ = writeln!(out, "- […{more} more in {rel}](<{target}>)");
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

    /// The grep reads SOURCE, where the author typed `don't`; the reader
    /// searches DISPLAY text, where smart punctuation made it `don’t`. A file
    /// the home screen says matches must match once opened, the same number
    /// of times, whichever spelling was typed — this used to report hits the
    /// reader then called "no matches".
    #[test]
    fn the_grep_count_is_what_the_reader_finds_through_smart_punctuation() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.md");
        let src = "don't say \"hi\" -- wait...\n\nit's 10--20 --- done\n\n\
                   ```\ndon't \"hi\" x--y\n```\n";
        std::fs::write(&a, src).unwrap();
        let doc = carrel_core::Document::parse(src);
        for needle in [
            "don't", "don’t", "\"hi\"", "“hi”", "wait...", "wait…", "10--20", "10–20", "it's",
            "--- done", "— done",
        ] {
            let reader = carrel_core::search(&doc, needle, true).len();
            let rx = spawn(vec![entry(&a)], needle.into(), 0);
            let (hits, _) = collect(&rx);
            let grep = hits.first().map_or(0, |h| h.count);
            assert!(
                reader > 0,
                "{needle:?}: the reader finds nothing in {:?}",
                doc.text
            );
            assert_eq!(grep, reader, "{needle:?}: grep and reader disagree");
        }
    }

    /// A hyphen run becomes as many dashes as smart punctuation decides,
    /// not one per pair: `---` is `—`, which holds no `--` and no `-`. The
    /// source holds both. Inside code the run stays literal, which reading
    /// the source cannot tell apart — so the file list counted 3 where the
    /// reader then said `1 of 2`.
    #[test]
    fn a_hyphen_run_counts_what_the_reader_shows() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.md");
        let src = "Wait... what -- really --- yes.\n\nCode: `... --` and `---` stay.\n";
        std::fs::write(&a, src).unwrap();
        let q = Query::new("--").unwrap();
        assert_eq!(grep_file(&q, &a).map(|h| h.count), Some(3));
        let q = Query::new("-").unwrap();
        assert_eq!(grep_file(&q, &a).map(|h| h.count), Some(5), "code only");
    }

    /// A file whose only match is somewhere the reader never shows — a
    /// link's URL — is not offered: opening it would say "no matches".
    #[test]
    fn a_match_the_reader_cannot_show_is_no_hit() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.md");
        std::fs::write(&a, "[docs](https://zebra.example/)\n").unwrap();
        let q = Query::new("zebra").unwrap();
        assert!(grep_file(&q, &a).is_none());
    }

    /// A word the author divided with soft hyphens is found whole on the
    /// file list, as it is once open.
    #[test]
    fn a_soft_hyphenated_word_is_found_whole() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.md");
        std::fs::write(&a, "super\u{AD}cali\u{AD}fragi\u{AD}listic\n").unwrap();
        let q = Query::new("supercalifragilistic").unwrap();
        assert_eq!(grep_file(&q, &a).map(|h| h.count), Some(1));
    }

    proptest::proptest! {
        /// The file list's count is the reader's, for documents made of
        /// the punctuation smart punctuation rewrites, in prose and in code.
        #[test]
        fn the_grep_count_is_the_reader_count(
            parts in proptest::collection::vec(proptest::sample::select(&[
                "-", "--", "---", "----", "-----", "x", " ", "a-b", "...", "....",
                "'", "\"", "don't", "`--`", "`---`", "`... --`", "\n\n",
                "\n\n```\nx---y -- z\n```\n\n", "\n- item\n", "&ndash;",
                "super\u{AD}cali", "\u{AD}",
            ][..]), 0..24),
            needle in proptest::sample::select(&[
                "-", "--", "---", "–", "—", "...", "…", "'", "\"", "x-", "x--", "-x",
                "don't", "a-b", "percal", "supercali",
            ][..]),
        ) {
            let src = parts.concat();
            let d = tempfile::tempdir().unwrap();
            let a = d.path().join("a.md");
            std::fs::write(&a, &src).unwrap();
            let q = Query::new(needle).unwrap();
            let grep = grep_file(&q, &a).map_or(0, |h| h.count);
            let doc = carrel_core::Document::parse(&src);
            let reader = carrel_core::search(&doc, needle, true).len();
            // The source is only the filter. Two things show the reader a
            // match the source does not spell, and the filter cannot see
            // them: an entity (`&ndash;` is `–`), and markup between the
            // characters (`` `--`x `` reads `--x`). Both are accepted; any
            // file the filter passes is counted exactly.
            if grep > 0 || !(src.contains('&') || src.contains('`')) {
                proptest::prop_assert_eq!(grep, reader, "{:?} in {:?}", needle, src);
            }
        }
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
