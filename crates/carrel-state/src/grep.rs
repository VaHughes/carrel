//! Multi-file content search for the home screen — wave E, Q9.
//!
//! Same streaming shape as `scan.rs`: a background thread walks the already-
//! scanned entry list and sends hits down a channel, so the filesystem never
//! blocks a frame. The files are read and filtered by a few workers, and
//! their hits are sent in entry order, so the list fills exactly as a single
//! thread would fill it. Stale queries die by GENERATION: the state layer
//! applies only messages stamped with the newest one, and an abandoned query
//! finds its receiver dropped on its next send and stops every worker.
//!
//! The SOURCE is only the filter: `carrel_core::SourceFilter` passes over a
//! file only when the reader, having opened it, would find nothing — so it
//! passes over nearly every file, and never one the reader matches. A file
//! it passes is parsed and counted over its DISPLAY text by the reader's own
//! search, so the count is the one the reader shows. Counting the source
//! disagreed wherever the display differs: a hyphen run becomes however
//! many dashes smart punctuation decides (`---` is `—`, which holds no
//! `--`), except in code, which the source cannot tell apart; and a match
//! inside a link's URL is never shown. Filtering by the reader's pattern
//! alone missed what the display spells and the source does not — an
//! entity (`&ndash;` is `–`), a phrase across markup (`**foo** bar`), list
//! markers or table pipes between two words, inline math (`$x^2$` is
//! `x²`) — so the home screen said "no match" for a file that, once open,
//! had one. `SourceFilter` covers each of those; the tests below hold one
//! case per construct, and `the_grep_count_is_the_reader_count` mixes them.
//!
//! Inline math is the filter's costliest step: the file is tokenized to
//! find its spans. That answer does not depend on the needle, so it is kept
//! per file (`MATH_VIEWS`) until the file's mtime or length changes, and
//! for as long as the file is in the list a finished search was given.
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

/// At most this many workers per query. The filter's loose pass scans every
/// byte that holds no plain match at DFA speed, where the old literal-only
/// filter skipped at memchr speed; spread over a few cores, reading and
/// filtering a large tree takes no longer than the single-threaded grep it
/// replaced did.
const WORKERS: usize = 4;

/// Search `entries` for `needle` on a background thread. Every message is
/// stamped with `generation`; the caller ignores stale ones. Hits arrive in
/// `entries` order, whichever worker found them.
#[must_use]
pub fn spawn(entries: Vec<Entry>, needle: String, generation: u64) -> Search {
    let (tx, rx) = mpsc::channel();
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let cancelled = std::sync::Arc::clone(&cancel);
    std::thread::spawn(move || {
        if let Some(q) = Query::new(&needle) {
            let workers = std::thread::available_parallelism().map_or(1, |n| n.get().min(WORKERS));
            let gone = grep_all(&q, &entries, workers, &cancelled, |hit| {
                tx.send(Msg::Hit(hit, generation)).is_ok()
            });
            if gone {
                return; // receiver gone: a newer query took over
            }
            // Finished, not cancelled: `entries` is the tree as it stands,
            // so this is when the kept views can follow it. Once per
            // settled query, on this thread — never per keystroke, never on
            // a list a newer query has already replaced.
            // The set is built before the lock is taken: the workers of the
            // query that replaces this one wait on that lock.
            let keep = listed(&entries);
            prune(
                &mut MATH_VIEWS
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
                &keep,
            );
        }
        let _ = tx.send(Msg::Done(generation));
    });
    Search { rx, cancel }
}

/// A running search: read it as the [`Receiver`] it dereferences to, and drop
/// it to stop it.
///
/// A dropped receiver alone was noticed only when the search next tried to
/// SEND a hit, so a query a newer keystroke replaced, with no hits left to
/// find, went on reading and filtering the whole tree — on up to four
/// threads, and typing a needle leaves one such query per keystroke. Dropping
/// this sets a flag every worker checks before each file.
#[derive(Debug)]
pub struct Search {
    rx: Receiver<Msg>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl std::ops::Deref for Search {
    type Target = Receiver<Msg>;
    fn deref(&self) -> &Receiver<Msg> {
        &self.rx
    }
}

impl Drop for Search {
    fn drop(&mut self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// [`grep_file`] over `entries` on `workers` threads, each taking the next
/// file as it finishes one. Hits are handed to `deliver` in `entries` order
/// — a hit waits for every file before it — up to [`MAX_HITS`]. Returns
/// `true` when `deliver` refused one (nobody is listening) or `cancel` was
/// set, either of which stops every worker at its next file.
fn grep_all(
    q: &Query,
    entries: &[Entry],
    workers: usize,
    cancel: &std::sync::atomic::AtomicBool,
    mut deliver: impl FnMut(Hit) -> bool,
) -> bool {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let (wtx, wrx) = mpsc::channel::<(usize, Option<Hit>)>();
    std::thread::scope(|s| {
        for _ in 0..workers {
            let (wtx, next, stop) = (wtx.clone(), &next, &stop);
            s.spawn(move || {
                while !stop.load(Ordering::Relaxed) && !cancel.load(Ordering::Relaxed) {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(e) = entries.get(i) else { break };
                    if wtx.send((i, grep_file(q, &e.path))).is_err() {
                        break;
                    }
                }
            });
        }
        drop(wtx);
        // Answers arrive in any order; each waits in its slot until every
        // file before it has answered, so hits go out in `entries` order.
        let mut slots: Vec<Option<Option<Hit>>> = vec![None; entries.len()];
        let mut emit = 0usize;
        let mut sent = 0usize;
        let mut gone = false;
        for (i, answer) in wrx {
            slots[i] = Some(answer);
            while let Some(answer) = slots.get_mut(emit).and_then(Option::take) {
                emit += 1;
                let Some(hit) = answer else { continue };
                if sent < MAX_HITS && !gone {
                    gone = !deliver(hit);
                    sent += 1;
                }
                if sent >= MAX_HITS || gone {
                    stop.store(true, Ordering::Relaxed);
                }
            }
        }
        gone || cancel.load(Ordering::Relaxed)
    })
}

/// A needle, and the source filter for it.
struct Query {
    needle: String,
    filter: carrel_core::SourceFilter,
}

impl Query {
    /// `None` for an empty needle.
    fn new(needle: &str) -> Option<Self> {
        Some(Self {
            needle: needle.to_owned(),
            filter: carrel_core::SourceFilter::new(needle)?,
        })
    }
}

/// Count matches in one file; `None` when unreadable, oversized, binary, or
/// match-free. Decoded by the reader's rule, so a file with a stray Latin-1
/// byte is searched, as it opens.
fn grep_file(q: &Query, path: &std::path::Path) -> Option<Hit> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > MAX_BYTES {
        return None;
    }
    let text = crate::text::decode_document(std::fs::read(path).ok()?)
        .ok()?
        .0;
    // The source is the cheap filter: almost every file fails it, at
    // regex speed, and it never fails one the reader would match. One that
    // passes is parsed and counted by the reader's own search, over the
    // display text — which is also what drops the filter's false positives.
    let stamp = (meta.modified().ok(), meta.len());
    if !q
        .filter
        .may_match_with(&text, || math_view(path, stamp, &text))
    {
        return None;
    }
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

/// A file's modification time and length: what says a kept view is stale.
type Stamp = (Option<std::time::SystemTime>, u64);

/// A file's math view, and the stamp it was built under.
type MathViews = std::collections::HashMap<PathBuf, (Stamp, Option<std::sync::Arc<str>>)>;

/// Math views kept across queries, by file. Building one tokenizes the file,
/// the costliest step of the filter, and its answer does not depend on the
/// needle — so each file pays once per change, not once per keystroke. Most
/// entries are `None` (no inline math), which costs a path; a file that has
/// math keeps one rendered copy of itself.
static MATH_VIEWS: std::sync::LazyLock<std::sync::Mutex<MathViews>> =
    std::sync::LazyLock::new(Default::default);

/// Keep only the views of files in `keep`: the list a search was given is
/// every file under the root as it stands, so a view of anything else is
/// of a file deleted since, or under a root the list has left.
fn prune(views: &mut MathViews, keep: &Listed<'_>) {
    views.retain(|p, _| keep.contains(p.as_path()));
}

/// The paths of a file list, to look them up.
type Listed<'a> = std::collections::HashSet<&'a std::path::Path>;

fn listed(keep: &[Entry]) -> Listed<'_> {
    keep.iter().map(|e| e.path.as_path()).collect()
}

/// The file's math view, from the cache when its stamp still matches.
fn math_view(path: &std::path::Path, stamp: Stamp, text: &str) -> Option<std::sync::Arc<str>> {
    let lock = || {
        MATH_VIEWS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    };
    if let Some((s, view)) = lock().get(path)
        && *s == stamp
    {
        return view.clone();
    }
    // Built outside the lock: another query's thread may be waiting on it.
    let view: Option<std::sync::Arc<str>> = carrel_core::math_view(text).map(Into::into);
    lock().insert(path.to_path_buf(), (stamp, view.clone()));
    view
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

    /// The grep's count for `needle` in a file holding `src`, beside the
    /// reader's: what opening the file and pressing `/` would say.
    fn grep_and_reader(src: &str, needle: &str) -> (usize, usize) {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.md");
        std::fs::write(&a, src).unwrap();
        let q = Query::new(needle).unwrap();
        let grep = grep_file(&q, &a).map_or(0, |h| h.count);
        let doc = carrel_core::Document::parse(src);
        (grep, carrel_core::search(&doc, needle, true).len())
    }

    /// The reader finds `needle` in `src`, and the file list says so, with
    /// the same count.
    #[track_caller]
    fn agrees(src: &str, needle: &str) {
        let (grep, reader) = grep_and_reader(src, needle);
        assert!(
            reader > 0,
            "{needle:?}: the fixture is wrong, the reader finds nothing in {:?}",
            carrel_core::Document::parse(src).text
        );
        assert_eq!(grep, reader, "{needle:?} in {src:?}");
    }

    // One test per way the display text spells a needle the source does
    // not. Each failed before the filter learned the construct: the source
    // regex passed the file over, and the reader then found it.

    #[test]
    fn a_named_entity_is_found_as_what_it_shows() {
        agrees("pages 10&ndash;20\n", "10–20");
        agrees("R&amp;D budget\n", "R&D");
        agrees("&copy; 2026\n", "© 2026");
    }

    #[test]
    fn a_numeric_entity_is_found_as_what_it_shows() {
        agrees("wait&#8212;what\n", "wait—what");
        agrees("&#x41;BC\n", "ABC");
        agrees("&#97;b\n", "ab");
    }

    #[test]
    fn an_entity_that_shows_two_letters_is_found() {
        agrees("&fjlig;ord\n", "fjord");
    }

    #[test]
    fn a_phrase_across_emphasis_is_found() {
        agrees("**foo** bar\n", "foo bar");
        agrees("fo*o* *b*ar ~~baz~~ __qux__\n", "foo bar baz qux");
    }

    #[test]
    fn a_phrase_across_a_code_span_is_found() {
        agrees("`foo` bar\n", "foo bar");
    }

    #[test]
    fn a_phrase_across_a_link_is_found() {
        agrees("[foo](x.md) bar\n", "foo bar");
        agrees("[foo](<a b.md> \"title\") bar\n", "foo bar");
        agrees("![foo](pic.png) bar\n", "foo bar");
        agrees("<https://ex.example/p> next\n", "ex.example/p next");
    }

    #[test]
    fn a_phrase_across_a_reference_link_is_found() {
        agrees("[foo][r] bar\n\n[r]: http://x.example\n", "foo bar");
    }

    #[test]
    fn a_phrase_across_a_reference_definition_is_found() {
        agrees("foo\n\n[r]: http://x.example \"t\"\n\nbar\n", "foo bar");
    }

    #[test]
    fn a_phrase_across_a_wikilink_alias_is_found() {
        agrees("[[target|foo]] bar\n", "foo bar");
    }

    #[test]
    fn a_phrase_across_inline_html_is_found() {
        agrees("foo<br>bar\n", "foo bar");
        agrees("<span class=\"x\">foo</span> bar\n", "foo bar");
        agrees("foo<!-- a > b -->bar\n", "foobar");
        agrees("see <img alt=\"foo\" src=x.png> bar\n", "see foo bar");
    }

    #[test]
    fn a_phrase_across_a_hidden_html_body_is_found() {
        agrees("foo\n\n<script>var x;</script>\n\nbar\n", "foo bar");
    }

    #[test]
    fn a_backslash_escape_is_found_as_what_it_shows() {
        agrees("a\\*b\n", "a*b");
        agrees("a-\\-b\n", "a--b");
    }

    #[test]
    fn a_phrase_across_list_markers_is_found() {
        agrees("- foo\n- bar\n", "foo bar");
        agrees("1. foo\n2. bar\n", "foo bar");
        agrees("bar\n\n- [x] foo\n", "bar foo");
    }

    #[test]
    fn a_phrase_across_block_markers_is_found() {
        agrees("> foo\n> bar\n", "foo bar");
        agrees("foo\n## bar\n", "foo bar");
        agrees("para\n\n```rust\nfn main() {}\n```\n", "para fn main");
        agrees("> [!NOTE]\n> foo\n", "Note foo");
    }

    #[test]
    fn a_phrase_across_table_cells_is_found() {
        agrees("| foo | bar |\n|---|---|\n| baz | qux |\n", "foo bar");
        agrees("| foo | bar |\n|---|---|\n| baz | qux |\n", "bar baz");
    }

    /// GFM drops a row's cells past the header's column count, letters
    /// and all.
    #[test]
    fn a_phrase_across_dropped_table_cells_is_found() {
        agrees("| a | b |\n|---|---|\n| e | f | gone |\n| h | i |\n", "f h");
    }

    /// An HTML block ends when the quote around it does, comment open or
    /// not: the comment hid the rest of that block, and no more.
    #[test]
    fn a_phrase_after_a_comment_its_quote_cut_short_is_found() {
        // The `>` inside matters: it ends a plain tag, not a comment.
        agrees("> x\n> <!-- a > b\n\nfoo\n", "x foo");
    }

    #[test]
    fn a_phrase_across_a_footnote_definition_is_found() {
        agrees("foo[^1]\n\n[^1]: bar\n", "foo[^1] bar");
    }

    #[test]
    fn an_attached_script_is_found_as_what_it_shows() {
        agrees("x^2^ and H~2~O\n", "x2 and H2O");
    }

    #[test]
    fn inline_math_is_found_as_it_renders() {
        let src = "the $x^2$ term, $\\alpha$, and $\\frac{a+b}{c}$ $y$.\n";
        agrees(src, "x²");
        agrees(src, "the x²");
        agrees(src, "x² term");
        agrees(src, "α, and (a + b)/c");
        agrees(src, "c y");
    }

    /// A kept math view answers only for the file as it was: a file that
    /// gains math is filtered by its new view, not the old one.
    #[test]
    fn a_kept_math_view_is_dropped_when_the_file_changes() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.md");
        let q = Query::new("x²").unwrap();
        std::fs::write(&a, "the $x$ and $y$ terms\n").unwrap();
        assert!(grep_file(&q, &a).is_none(), "no x² yet");
        std::fs::write(&a, "the $x^2$ and $y$ terms, longer\n").unwrap();
        assert_eq!(grep_file(&q, &a).map(|h| h.count), Some(1));
    }

    /// The kept views are the files the search was given, and no others: a
    /// file deleted, or one under a root the list has left, is dropped
    /// (black-box review, 2026-10-31: they were kept for the session).
    #[test]
    fn a_kept_math_view_goes_with_its_file() {
        let d = tempfile::tempdir().unwrap();
        let (a, b) = (d.path().join("a.md"), d.path().join("b.md"));
        let mut views = MathViews::new();
        for p in [&a, &b] {
            views.insert(p.clone(), ((None, 0), None));
        }
        prune(&mut views, &listed(&[entry(&a)]));
        assert!(views.contains_key(&a), "still listed, still kept");
        assert!(
            !views.contains_key(&b),
            "gone from the list, gone from the cache"
        );
        prune(&mut views, &listed(&[]));
        assert!(views.is_empty());
    }

    /// A search that runs to its end prunes the shared cache to its list.
    #[test]
    fn a_finished_search_prunes_the_kept_views() {
        let d = tempfile::tempdir().unwrap();
        let gone = d.path().join("gone.md");
        let here = d.path().join("here.md");
        std::fs::write(&here, "needle\n").unwrap();
        MATH_VIEWS
            .lock()
            .unwrap()
            .insert(gone.clone(), ((None, 0), None));
        let (_, done) = collect(&spawn(vec![entry(&here)], "needle".into(), 0));
        assert!(done);
        assert!(!MATH_VIEWS.lock().unwrap().contains_key(&gone));
    }

    /// Workers finish files in any order; the list fills in `entries`
    /// order regardless, so the newest-first sort holds while it streams.
    #[test]
    fn hits_arrive_in_entry_order_whichever_worker_finds_them() {
        let d = tempfile::tempdir().unwrap();
        let mut entries = Vec::new();
        for i in 0..64 {
            let p = d.path().join(format!("{i}.md"));
            // Sizes vary so the workers finish out of step.
            std::fs::write(
                &p,
                format!("needle\n{}", "padding words\n".repeat(i * 37 % 500)),
            )
            .unwrap();
            entries.push(entry(&p));
        }
        let rx = spawn(entries.clone(), "needle".into(), 0);
        let (hits, done) = collect(&rx);
        assert!(done);
        let got: Vec<_> = hits.iter().map(|h| h.path.clone()).collect();
        let want: Vec<_> = entries.iter().map(|e| e.path.clone()).collect();
        assert_eq!(got, want);
    }

    #[test]
    fn the_hit_list_stops_at_its_cap() {
        let d = tempfile::tempdir().unwrap();
        let entries: Vec<Entry> = (0..MAX_HITS + 7)
            .map(|i| {
                let p = d.path().join(format!("{i}.md"));
                std::fs::write(&p, "needle\n").unwrap();
                entry(&p)
            })
            .collect();
        let (hits, done) = collect(&spawn(entries, "needle".into(), 0));
        assert!(done);
        assert_eq!(hits.len(), MAX_HITS);
    }

    proptest::proptest! {
        // More cases than the default: a needle meets the one part that
        // spells it differently (`$x^2$` for `x²`) only now and then.
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(1024))]
        /// The file list's count is the reader's, for documents made of
        /// the punctuation smart punctuation rewrites, in prose and in code,
        /// and of the entities, inline markup, block structure and math
        /// that make the display text spell a needle the source does not.
        /// No exemption: a file the reader matches is a hit.
        #[test]
        fn the_grep_count_is_the_reader_count(
            parts in proptest::collection::vec(proptest::sample::select(&[
                "-", "--", "---", "----", "-----", "x", " ", "a-b", "...", "....",
                "'", "\"", "don't", "`--`", "`---`", "`... --`", "\n\n",
                "\n\n```\nx---y -- z\n```\n\n", "\n- item\n", "&ndash;",
                "super\u{AD}cali", "\u{AD}",
                "&amp;", "&#8212;", "&#x2d;", "**", "_", "~~", "`", "\\-", "\\*",
                "[x](u.md)", "[[t|x]]", "<b>", "</b>", "<!-- c -->", "<br>",
                "\n> ", "\n1. ", "| x | - |\n|---|---|\n", "$x^2$", "x^2^", "a",
            ][..]), 0..24),
            needle in proptest::sample::select(&[
                "-", "--", "---", "–", "—", "...", "…", "'", "\"", "x-", "x--", "-x",
                "don't", "a-b", "percal", "supercali", "&", "x²", "x x", "x-x", "ax",
                "x2", "-a", "x — x", "*x",
            ][..]),
        ) {
            let src = parts.concat();
            let (grep, reader) = grep_and_reader(&src, needle);
            proptest::prop_assert_eq!(grep, reader, "{:?} in {:?}", needle, src);
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

    /// A cancelled search reads nothing more: every worker checks the flag
    /// before its next file, so with it set at the start not one of these
    /// matching files is delivered.
    #[test]
    fn a_cancelled_search_reads_no_further_file() {
        let d = tempfile::tempdir().unwrap();
        let entries: Vec<Entry> = (0..50)
            .map(|i| {
                let p = d.path().join(format!("f{i}.md"));
                std::fs::write(&p, "needle\n").unwrap();
                entry(&p)
            })
            .collect();
        let q = Query::new("needle").unwrap();
        let mut delivered = 0;
        let live = std::sync::atomic::AtomicBool::new(false);
        grep_all(&q, &entries, 2, &live, |_| {
            delivered += 1;
            true
        });
        assert_eq!(delivered, 50, "control: uncancelled, every file is a hit");
        let mut delivered = 0;
        let cancelled = std::sync::atomic::AtomicBool::new(true);
        let gone = grep_all(&q, &entries, 2, &cancelled, |_| {
            delivered += 1;
            true
        });
        assert_eq!(delivered, 0);
        assert!(gone, "reported as stopped");
    }

    /// Dropping the handle — what replacing it with a newer query does — is
    /// what sets that flag.
    #[test]
    fn dropping_a_search_cancels_it() {
        let search = spawn(Vec::new(), "x".into(), 1);
        let flag = std::sync::Arc::clone(&search.cancel);
        assert!(!flag.load(std::sync::atomic::Ordering::Relaxed));
        drop(search);
        assert!(flag.load(std::sync::atomic::Ordering::Relaxed));
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
