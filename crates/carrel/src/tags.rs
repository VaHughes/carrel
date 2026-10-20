//! Tags across the folder, read as a document.
//!
//! `/rust` on the home screen already finds every document tagged `rust` —
//! retrieval works with a general tool. What it cannot answer is *which tags
//! are there*: you have to know a tag exists to search for it. This module
//! answers that, and answers it the way carrel answers everything else — by
//! handing the reader something to read. The tags become a generated markdown
//! document, a section per tag with a link per document, so collapsing, the
//! outline, in-document search and the heading bar all work on it with no new
//! machinery. It is `grep::render_results`' idea again, and `carrel_core::diff`'s.
//!
//! Same streaming shape as `grep.rs` and `scan.rs`: a background thread walks
//! the already-scanned entry list and reads each file (to [`BODY_MAX`]), so
//! the filesystem never blocks a frame. A file is parsed only when a byte
//! scan cannot rule out an inline tag ([`may_hold_inline_tag`]); a note whose
//! only hashes open headings costs the read and nothing more.
//!
//! # What counts as a tag
//!
//! `tags:` (or `tag:`) in a YAML `---` block, `tags = […]` in a TOML `+++`
//! one. Not a YAML parse — a reader must never fail on exotic YAML — but the
//! three spellings people actually write are all read: a flow list
//! (`[a, b]`), a block list (`- a`), and a bare scalar (`a, b` or `a b`).
//!
//! And an inline `#tag` in the body, by the reader's own parse
//! ([`carrel_core::Document::tags`]) — so the page lists exactly the tags the
//! reader paints as buttons. That parse is what tells a tag from the other
//! things a `#word` is in markdown: a heading needs a space, `#42` has no
//! letter, `C#` and a URL's `/#fragment` do not follow whitespace, a hex
//! color is refused by shape, and code is never read.
//!
//! # Nesting
//!
//! `project/alpha` is `alpha` inside `project`, Obsidian's reading. The page
//! makes it a section inside its parent's, and a parent's count is every
//! document tagged with it or with anything under it ([`groups`]).
//!
//! NO RATATUI — `scripts/check-discipline.sh` rule 6.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::SystemTime;

use carrel_core::MetaFormat;

use crate::scan::Entry;

/// The buffer a file's read starts with; most notes fit in it.
const HEAD_BYTES: usize = 8 * 1024;
/// How much of a file is read for its tags. Frontmatter is far inside it,
/// and a note longer than this is a dump, not a note: its tags past here are
/// not read, and a hostile file costs one bounded parse.
const BODY_MAX: u64 = 1024 * 1024;
/// Tags kept per document. A list longer than this is data, not labeling.
const MAX_TAGS_PER_FILE: usize = 32;
/// Characters kept per tag.
const MAX_TAG_CHARS: usize = carrel_core::document::MAX_TAG_CHARS;
/// Tagged documents collected before the scan stops. The document this
/// becomes says so when the cap is what ended it.
pub const MAX_TAGGED: usize = 5000;

/// One document that declares tags.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tagged {
    pub path: PathBuf,
    pub mtime: SystemTime,
    /// As written, in order, deduplicated case-insensitively.
    pub tags: Vec<String>,
    /// The document's title, when titles were asked for and it has one.
    pub title: Option<String>,
}

/// Everything one scan learned.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Index {
    pub tagged: Vec<Tagged>,
    /// Files whose heads were read, tagged or not.
    pub read: usize,
    /// The scan stopped at [`MAX_TAGGED`] with files still unread.
    pub capped: bool,
}

/// Where the home screen's request for tags stands.
///
/// One value rather than a flag and a slot, because the two could otherwise
/// disagree: a request still pending with a finished index beside it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Request {
    #[default]
    Idle,
    /// `#` was pressed and the tags are being read. The event loop owns the
    /// thread; whatever the reader does next withdraws this — a document
    /// must not open itself under someone who moved on.
    Wanted,
    /// The scan finished; `Action::HomeOpenTags` takes this and opens it.
    Ready(Index),
}

/// Which documents to read.
#[derive(Debug)]
pub enum Source {
    /// The file list the home screen already has.
    Entries(Vec<Entry>),
    /// A folder to walk first — for a reader with no file list behind it.
    Walk(PathBuf),
}

#[derive(Debug)]
pub enum Msg {
    Found(Tagged, u64),
    Done {
        read: usize,
        capped: bool,
        generation: u64,
    },
}

/// Read the head of every entry on a background thread, reporting the ones
/// that declare tags. Every message is stamped with `generation`; the caller
/// ignores stale ones, and a dropped receiver stops the thread at its next
/// send.
///
/// `titles` also reads each tagged document's title (`scan::title_of`), for
/// the reader who asked the file list to show titles.
#[must_use]
pub fn spawn(source: Source, titles: bool, generation: u64) -> Receiver<Msg> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let entries = match source {
            Source::Entries(entries) => entries,
            // No file list to borrow from: the reader was opened on a file.
            // The walk is the home screen's own, on this thread.
            Source::Walk(root) => crate::scan::walk_blocking(&root).0,
        };
        let Some((read, capped)) = read_all(&entries, titles, |t| {
            tx.send(Msg::Found(t, generation)).is_ok()
        }) else {
            return; // receiver gone: the reader moved on
        };
        let _ = tx.send(Msg::Done {
            read,
            capped,
            generation,
        });
    });
    rx
}

/// Read every entry's tags, handing each tagged document to `found` in the
/// entries' own order, and return `(read, capped)` — or `None` once `found`
/// says no one is listening.
///
/// Spread over a few threads, because an inline tag can only be told from
/// the other things a `#word` is by parsing the file, and parsing is the
/// cost: measured over a 25,000-document tree, the files the byte scan
/// could not rule out took seconds to parse on one core. The order the
/// documents arrive in is kept, so the result is the one a single thread
/// would give, and [`MAX_TAGGED`] stops the read at the same document.
fn read_all(
    entries: &[Entry],
    titles: bool,
    mut found: impl FnMut(Tagged) -> bool,
) -> Option<(usize, bool)> {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let workers = std::thread::available_parallelism()
        .map_or(1, std::num::NonZero::get)
        .clamp(1, 8);
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let (done_tx, done_rx) = mpsc::channel::<(usize, Option<Tagged>)>();
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let done_tx = done_tx.clone();
            let (next, stop) = (&next, &stop);
            scope.spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(e) = entries.get(i) else { break };
                    let tags = tags_of(&e.path);
                    let tagged = (!tags.is_empty()).then(|| Tagged {
                        path: e.path.clone(),
                        mtime: e.mtime,
                        tags,
                        title: titles.then(|| crate::scan::title_of(&e.path)).flatten(),
                    });
                    if done_tx.send((i, tagged)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(done_tx);
        // Results arrive in whatever order the workers finish; hold the
        // early ones until the documents before them are in.
        let mut waiting: HashMap<usize, Option<Tagged>> = HashMap::new();
        let (mut read, mut count, mut capped) = (0usize, 0usize, false);
        let mut alive = true;
        'outer: for (i, tagged) in &done_rx {
            waiting.insert(i, tagged);
            while let Some(tagged) = waiting.remove(&read) {
                if count >= MAX_TAGGED {
                    capped = true;
                    break 'outer;
                }
                read += 1;
                if let Some(t) = tagged {
                    if !found(t) {
                        alive = false;
                        break 'outer;
                    }
                    count += 1;
                }
            }
        }
        stop.store(true, Ordering::Relaxed);
        alive.then_some((read, capped))
    })
}

/// The tags a file carries: its frontmatter's, then the inline `#tags` in
/// its body. Empty when it has neither, is not a regular file, or cannot be
/// read.
///
/// Frontmatter is found by the reader's own rule
/// ([`carrel_core::frontmatter`]), so this can never index tags from a block
/// the reader shows as a rule and a paragraph, nor read a block in the wrong
/// dialect. Inline tags are found by the reader's own parse
/// ([`carrel_core::Document::tags`]) — the same buttons the reader paints —
/// but only for a file [`may_hold_inline_tag`] cannot rule out, which is
/// most of what keeps the scan near the cost of reading the files at all.
#[must_use]
pub fn tags_of(path: &Path) -> Vec<String> {
    use std::io::Read as _;
    if !std::fs::metadata(path).is_ok_and(|m| m.is_file()) {
        return Vec::new(); // a FIFO named `notes.md` blocks its reader
    }
    let Ok(f) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let mut buf = Vec::with_capacity(HEAD_BYTES);
    if f.take(BODY_MAX).read_to_end(&mut buf).is_err() {
        return Vec::new();
    }
    tags_in(&String::from_utf8_lossy(&buf))
}

/// [`tags_of`] over text already read.
#[must_use]
pub fn tags_in(text: &str) -> Vec<String> {
    let mut out = carrel_core::frontmatter(text)
        .map_or_else(Vec::new, |fm| parse_tags(&text[fm.body], fm.format));
    if may_hold_inline_tag(text.as_bytes()) {
        let doc = carrel_core::Document::parse(text);
        for r in &doc.tags {
            push_tag(&mut out, doc.tag_name(r));
        }
    }
    out
}

/// Could `bytes` hold an inline tag? `false` is certain; `true` means parse
/// and see.
///
/// Judged on raw bytes, so it may only refuse what no markup can turn into a
/// tag:
///
/// - **After the `#`**, whitespace, another `#`, or punctuation the display
///   text keeps as it is (`NEVER_NAMED`), since no name starts with any of
///   them. So headings (`# Title`, `## Part`), a Rust attribute
///   (`#[derive]`) and a word-ending `F#,` are dismissed. Markup that
///   vanishes is not refused: `#^x^` is `#x` once the superscript's
///   delimiters are gone, and `#![x](u)` is `#x`, an image's alt text.
/// - **Before it**, an ASCII letter or digit: no markup is one, so the
///   display text has it too, glued to the `#` — `C#x`, `page#frag`. Any
///   other byte may be markup the display text drops (`**#bold**`,
///   `|#cell|`, `>#quote`), and is left to the parse.
#[must_use]
pub fn may_hold_inline_tag(bytes: &[u8]) -> bool {
    /// Bytes that, straight after a `#`, the display text keeps as they are
    /// — so the `#` is followed by something no name starts with. Markup
    /// that vanishes (`*`, `_`, `^`, `~`, `<`, `\`, `&`) is not here.
    const NEVER_NAMED: &[u8] = b"#[](){}.,;:?\"')/=+@%|$`";
    let mut from = 0usize;
    while let Some(i) = bytes[from..].iter().position(|&b| b == b'#') {
        let at = from + i;
        let glued = at > 0 && bytes[at - 1].is_ascii_alphanumeric();
        let named = bytes
            .get(at + 1)
            .is_some_and(|&b| !(b.is_ascii_whitespace() || NEVER_NAMED.contains(&b)));
        if named && !glued {
            return true;
        }
        from = at + 1;
    }
    false
}

/// The tags in a frontmatter body.
///
/// Reads the top-level `tags` (or `tag`) key and nothing else:
///
/// ```text
/// tags: [rust, "web dev"]      a flow list — split on commas
/// tags:                        a block list — one per line
///   - rust
///   - web dev
/// tags: rust, web              a scalar with commas — split on them
/// tags: rust web               a scalar without — split on spaces
/// tags = ["rust", "web dev"]   TOML
/// ```
#[must_use]
pub fn parse_tags(body: &str, format: MetaFormat) -> Vec<String> {
    let lines: Vec<&str> = body.lines().collect();
    let mut out: Vec<String> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        // Key lines by the core's one predicate, so a tag is read only from
        // a line the card shows as a key: top-level (an indented `tags:`
        // belongs to something else), split at the block's own separator.
        let Some(value) = tags_value(line, format) else {
            continue;
        };
        let value = uncomment(value).trim();
        if let Some(open) = value.strip_prefix('[') {
            // A flow list may run over several lines; gather to its `]`.
            let mut flow = open.to_string();
            let mut next = i + 1;
            while !flow.contains(']') && next < lines.len() {
                flow.push(' ');
                flow.push_str(uncomment(lines[next]));
                next += 1;
            }
            let inner = flow.split(']').next().unwrap_or_default();
            for item in split_outside_quotes(inner, ',') {
                push_tag(&mut out, item);
            }
        } else if value.is_empty() {
            for item in lines[i + 1..]
                .iter()
                .map(|l| uncomment(l).trim())
                .filter(|l| !l.is_empty())
                .map_while(|l| l.strip_prefix("- ").or_else(|| (l == "-").then_some("")))
            {
                push_tag(&mut out, item);
            }
        } else if is_quoted(value) {
            push_tag(&mut out, value);
        } else if value.contains(',') {
            for item in value.split(',') {
                push_tag(&mut out, item);
            }
        } else {
            for item in value.split_whitespace() {
                push_tag(&mut out, item);
            }
        }
    }
    out
}

/// Where each tag is written in a frontmatter body: `(byte range, tag)`, in
/// order. For making the tags on a metadata card clickable.
///
/// The tags come from [`parse_tags`], so this can never disagree with the
/// tags page about what a tag is; their positions are then found by looking
/// for each one's text in the lines that belong to the `tags` key. Longest
/// first, so `rust` does not claim the middle of `rustlang`, and only at a
/// word edge, so it does not claim the start of it either.
#[must_use]
pub fn spans(body: &str, format: MetaFormat) -> Vec<(std::ops::Range<usize>, String)> {
    let tags = parse_tags(body, format);
    if tags.is_empty() {
        return Vec::new();
    }
    // The regions: for each `tags` / `tag` key, from just after its
    // separator to the next top-level key or the end — minus comments.
    // Kept as the pieces of text a tag may be found in.
    let mut pieces: Vec<std::ops::Range<usize>> = Vec::new();
    let mut inside = false;
    let mut at = 0usize;
    for line in body.split_inclusive('\n') {
        let from = match carrel_core::meta_split(line, format) {
            Some(m) if is_tags_key(&line[..m.key_end]) => {
                inside = true;
                m.sep_end
            }
            Some(_) => {
                inside = false;
                0
            }
            None => 0,
        };
        if inside {
            let kept = uncomment(line).len().max(from);
            pieces.push(at + from..at + kept);
        }
        at += line.len();
    }
    let mut order: Vec<&String> = tags.iter().collect();
    order.sort_by_key(|t| std::cmp::Reverse(t.len()));
    let edge = |c: Option<char>| c.is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
    let mut out: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    for tag in order {
        let found = pieces.iter().find_map(|piece| {
            let text = &body[piece.clone()];
            let mut from = 0;
            while let Some((s, e)) = find_spaced(text, tag, from) {
                let (s, e) = (piece.start + s, piece.start + e);
                if edge(body[..s].chars().next_back())
                    && edge(body[e..].chars().next())
                    && !out.iter().any(|(r, _)| s < r.end && r.start < e)
                {
                    return Some(s..e);
                }
                from = s - piece.start + 1;
                while !text.is_char_boundary(from) {
                    from += 1;
                }
            }
            None
        });
        if let Some(range) = found {
            out.push((range, tag.clone()));
        }
    }
    out.sort_by_key(|(r, _)| r.start);
    out
}

/// The value of `line` when it is a `tags` (or `tag`) key line, by
/// [`carrel_core::meta_split`] — the predicate the card styles keys with.
fn tags_value(line: &str, format: MetaFormat) -> Option<&str> {
    let m = carrel_core::meta_split(line, format)?;
    is_tags_key(&line[..m.key_end]).then(|| &line[m.value..])
}

/// `tags` or `tag`, in any case, quoted or not.
fn is_tags_key(key: &str) -> bool {
    let key = unquote_key(key);
    key.eq_ignore_ascii_case("tags") || key.eq_ignore_ascii_case("tag")
}

/// A key without the quotes YAML and TOML both allow around it.
pub(crate) fn unquote_key(key: &str) -> &str {
    for q in ['"', '\''] {
        if let Some(k) = key.strip_prefix(q).and_then(|k| k.strip_suffix(q)) {
            return k;
        }
    }
    key
}

/// Find `tag` in `text` at or after `from`, where each space in the tag
/// stands for any run of whitespace. A tag is stored with its whitespace
/// collapsed, so `"web  dev"` as written is the tag `web dev` — and unless
/// it is found as itself, the shorter tag `dev` claims the end of it.
fn find_spaced(text: &str, tag: &str, from: usize) -> Option<(usize, usize)> {
    let mut words = tag.split(' ').filter(|w| !w.is_empty());
    let first = words.next()?;
    let rest: Vec<&str> = words.collect();
    let mut search = from;
    while let Some(i) = text.get(search..)?.find(first) {
        let start = search + i;
        let mut end = start + first.len();
        let whole = rest.iter().all(|word| {
            let gap = text[end..].len() - text[end..].trim_start().len();
            if gap > 0 && text[end + gap..].starts_with(word) {
                end += gap + word.len();
                true
            } else {
                false
            }
        });
        if whole {
            return Some((start, end));
        }
        search = start + first.chars().next().map_or(1, char::len_utf8);
    }
    None
}

/// `line` without a trailing comment. `#` opens one only after whitespace (or
/// at the very start) and outside quotes — which is YAML's rule and TOML's,
/// and is what keeps `"#rust"` and `c#` as tags.
fn uncomment(line: &str) -> &str {
    let mut quote: Option<char> = None;
    let mut prev = ' ';
    for (i, c) in line.char_indices() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
        } else if opens_quote(c, prev) {
            quote = Some(c);
        } else if c == '#' && prev.is_whitespace() {
            return &line[..i];
        }
        prev = c;
    }
    line
}

/// A quote opens only where a value starts — after whitespace, a `[`, a `,`
/// or a list dash. Mid-word it is an apostrophe: `valentine's day` holds no
/// string, and reading one there swallows every comma and comment after it.
fn opens_quote(c: char, prev: char) -> bool {
    (c == '"' || c == '\'') && (prev.is_whitespace() || matches!(prev, '[' | ',' | ':' | '='))
}

fn is_quoted(s: &str) -> bool {
    s.len() >= 2
        && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')))
}

/// Split on `sep`, ignoring separators inside quotes.
fn split_outside_quotes(s: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let mut quote: Option<char> = None;
    let mut start = 0usize;
    let mut prev = ' ';
    for (i, c) in s.char_indices() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
        } else if opens_quote(c, prev) {
            quote = Some(c);
        } else if c == sep {
            out.push(&s[start..i]);
            start = i + c.len_utf8();
        }
        prev = c;
    }
    out.push(&s[start..]);
    out
}

/// Clean one raw item and keep it, unless it is empty, a YAML null, or a tag
/// this document already has.
fn push_tag(out: &mut Vec<String>, raw: &str) {
    let raw = raw.trim();
    let raw = if is_quoted(raw) {
        &raw[1..raw.len() - 1]
    } else {
        raw
    };
    // Obsidian writes tags with their `#`; the name is what follows it.
    let raw = raw.trim().trim_start_matches('#');
    // A tag travels into generated markdown and onto the screen: no control
    // characters, one line, bounded.
    let tag: String = raw
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_TAG_CHARS)
        .collect();
    if tag.is_empty() || matches!(tag.as_str(), "null" | "~") {
        return;
    }
    if out.len() < MAX_TAGS_PER_FILE && !out.iter().any(|t| same_tag(t, &tag)) {
        out.push(tag);
    }
}

/// Tags are compared without regard to case: `Rust` and `rust` are one tag,
/// which is how Obsidian and every static-site generator group them.
fn same_tag(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// The key a tag is grouped by: lowercase, and nested at its `/`s with the
/// empty and blank segments dropped, so `Project/Alpha`, `project//alpha`
/// and `project / alpha` are one tag. A tag that is nothing but slashes
/// keeps itself.
#[must_use]
pub fn key(tag: &str) -> String {
    let lower = tag.to_lowercase();
    let joined = lower
        .split('/')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("/");
    if joined.is_empty() { lower } else { joined }
}

/// One tag and the documents that carry it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group<'a> {
    /// The spelling shown, the whole path (`project/alpha`): the one used by
    /// the most documents, and among equals the first alphabetically, so the
    /// choice never depends on the order the scan happened to find them in.
    pub name: String,
    /// How deep it nests: 0 for `project`, 1 for `project/alpha`.
    pub depth: usize,
    /// The documents tagged with exactly this tag. Newest first.
    pub docs: Vec<&'a Tagged>,
    /// The documents tagged with it or with anything nested under it, each
    /// counted once — the number its heading shows.
    pub total: usize,
}

/// The index grouped by tag, as a tree flattened in reading order: each tag
/// followed by the tags nested under it. Siblings are most-used first (by
/// [`Group::total`]), then alphabetical.
///
/// A nested tag makes its parents exist: `project/alpha` alone still gives a
/// `project` group, with no documents of its own and alpha's in its total.
#[must_use]
pub fn groups(index: &Index) -> Vec<Group<'_>> {
    #[derive(Default)]
    struct Slot<'a> {
        spellings: HashMap<String, usize>,
        docs: Vec<&'a Tagged>,
        under: Vec<&'a Tagged>,
    }
    let mut by_key: HashMap<String, Slot<'_>> = HashMap::new();
    for doc in &index.tagged {
        for tag in &doc.tags {
            let full = key(tag);
            // The spelling of each level, read from this tag as written.
            let written: Vec<&str> = tag
                .split('/')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect();
            let ends = levels(&full);
            for (i, &end) in ends.iter().enumerate() {
                let k = &full[..end];
                let slot = by_key.entry(k.to_string()).or_default();
                let spelled = if written.len() == ends.len() {
                    written[..=i].join("/")
                } else {
                    k.to_string()
                };
                *slot.spellings.entry(spelled).or_default() += 1;
                slot.under.push(doc);
                if end == full.len() {
                    slot.docs.push(doc);
                }
            }
        }
    }
    let mut nodes: Vec<(String, Group<'_>)> = by_key
        .into_iter()
        .map(|(key, slot)| {
            let name = slot
                .spellings
                .into_iter()
                .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
                .map(|(s, _)| s)
                .unwrap_or_default();
            let mut docs = slot.docs;
            docs.sort_by(|a, b| b.mtime.cmp(&a.mtime).then_with(|| a.path.cmp(&b.path)));
            docs.dedup_by(|a, b| std::ptr::eq(*a, *b));
            let mut under = slot.under;
            under.sort_by_key(|d| std::ptr::from_ref(*d));
            under.dedup_by(|a, b| std::ptr::eq(*a, *b));
            let depth = levels(&key).len() - 1;
            let total = under.len();
            (
                key,
                Group {
                    name,
                    depth,
                    docs,
                    total,
                },
            )
        })
        .collect();
    // Sorting by the path of sort keys puts every child straight after its
    // parent, siblings in the page's order.
    let rank: HashMap<String, usize> = nodes.iter().map(|(k, g)| (k.clone(), g.total)).collect();
    let path_of = |k: &str| -> Vec<(std::cmp::Reverse<usize>, String)> {
        levels(k)
            .into_iter()
            .map(|end| {
                let prefix = &k[..end];
                (
                    std::cmp::Reverse(rank.get(prefix).copied().unwrap_or(0)),
                    prefix.to_string(),
                )
            })
            .collect()
    };
    nodes.sort_by_cached_key(|(k, _)| path_of(k));
    nodes.into_iter().map(|(_, g)| g).collect()
}

/// Where each level of a [`key`] ends: `a/b/c` is `[1, 3, 5]`. A key with an
/// empty level — only the all-slashes tag [`key`] keeps as written — is one
/// level, not a nest of nothing.
fn levels(key: &str) -> Vec<usize> {
    if key.split('/').any(str::is_empty) {
        return vec![key.len()];
    }
    key.match_indices('/')
        .map(|(i, _)| i)
        .chain(std::iter::once(key.len()))
        .collect()
}

/// Backslash-escape everything markdown could read as syntax.
///
/// A tag, a title and a file name are untrusted text headed for a heading
/// and a link label. `CommonMark` lets any ASCII punctuation be escaped, so escaping all
/// of it is both sufficient and the whole rule.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for c in text.chars() {
        // A label is one line. A file name may hold a newline, and a blank
        // line inside a link label ends the link and starts a paragraph.
        if c.is_control() {
            out.push(' ');
            continue;
        }
        if c.is_ascii_punctuation() {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// Render the index as a markdown document: a section per tag, a link per
/// document.
///
/// Paths read relative to `root`, targets included, so the links resolve
/// wherever the document is opened from — the opener points link resolution
/// at the same root, exactly as it does for search results.
#[must_use]
pub fn render(root: &Path, index: &Index) -> String {
    use std::fmt::Write as _;
    let groups = groups(index);
    let mut out = String::from("# Tags\n\n");
    let _ = writeln!(
        out,
        "_{} in {} of the {} read. Most used first; under each tag, newest first._\n",
        plural(groups.len(), "tag", "tags"),
        plural(index.tagged.len(), "document", "documents"),
        index.read,
    );
    if index.capped {
        let _ = writeln!(
            out,
            "_Stopped at {MAX_TAGGED} tagged documents; the rest of this folder was not read._\n"
        );
    }
    for g in &groups {
        // A nested tag is a section inside its parent's, so collapsing the
        // parent folds its children with it and expanding it shows every
        // document under it. Markdown stops at six levels; deeper tags sit
        // beside each other at the sixth.
        let hashes = "#".repeat((2 + g.depth).min(6));
        let _ = writeln!(out, "{hashes} {} ({})\n", escape(&g.name), g.total);
        for d in &g.docs {
            let rel = d
                .path
                .strip_prefix(root)
                .unwrap_or(&d.path)
                .display()
                .to_string();
            let label = d.title.as_deref().unwrap_or(&rel);
            // An angle-bracket target, because a path may hold a space, and
            // an encoded one, because it may hold anything else.
            let target = crate::links::encode_target(&rel);
            let _ = writeln!(out, "- [{}](<{target}>)", escape(label));
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn yaml(body: &str) -> Vec<String> {
        parse_tags(body, MetaFormat::Yaml)
    }

    #[test]
    fn the_three_spellings_people_write_are_all_read() {
        let want = vec!["rust".to_string(), "web dev".to_string()];
        assert_eq!(yaml("tags: [rust, web dev]\n"), want, "flow list");
        assert_eq!(yaml("tags: [rust, \"web dev\"]\n"), want, "quoted item");
        assert_eq!(yaml("tags:\n  - rust\n  - web dev\n"), want, "block list");
        assert_eq!(yaml("tags:\n- rust\n- web dev\n"), want, "flush block list");
        assert_eq!(yaml("tags: rust, web dev\n"), want, "comma scalar");
        assert_eq!(
            parse_tags("tags = [\"rust\", \"web dev\"]\n", MetaFormat::Toml),
            want,
            "TOML"
        );
    }

    #[test]
    fn a_scalar_without_commas_splits_on_spaces() {
        // Jekyll's rule and Obsidian's: `tags: a b` is two tags.
        assert_eq!(yaml("tags: rust web\n"), ["rust", "web"]);
        // Quoting is how one tag with a space is written.
        assert_eq!(yaml("tags: \"web dev\"\n"), ["web dev"]);
        assert_eq!(yaml("tag: solo\n"), ["solo"], "the singular key");
    }

    #[test]
    fn a_flow_list_may_span_lines() {
        assert_eq!(
            yaml("title: x\ntags: [rust,\n  notes,\n  ideas]\ndate: 2026\n"),
            ["rust", "notes", "ideas"]
        );
    }

    #[test]
    fn a_block_list_ends_at_the_next_key() {
        assert_eq!(
            yaml("tags:\n  - rust\n\n  - notes\ntitle: not a tag\n- stray\n"),
            ["rust", "notes"]
        );
    }

    #[test]
    fn only_the_top_level_key_counts() {
        assert_eq!(yaml("meta:\n  tags: [inner]\n"), Vec::<String>::new());
        assert_eq!(yaml("hashtags: [no]\nmytags: no\n"), Vec::<String>::new());
        assert_eq!(
            yaml("Tags: [Yes]\n"),
            ["Yes"],
            "key case is not the author's problem"
        );
    }

    #[test]
    fn nothing_is_a_tag() {
        for body in [
            "tags:\n",
            "tags: []\n",
            "tags: null\n",
            "tags: ~\n",
            "tags: [ , ]\n",
            "tags: # nothing yet\n",
            "title: no tags here\n",
            "",
        ] {
            assert_eq!(yaml(body), Vec::<String>::new(), "{body:?}");
        }
    }

    #[test]
    fn a_hash_is_a_comment_only_where_yaml_says_so() {
        assert_eq!(yaml("tags: [rust, notes] # my tags\n"), ["rust", "notes"]);
        assert_eq!(yaml("tags:\n  - rust # the language\n"), ["rust"]);
        // Obsidian's spelling: the `#` is part of the quoted value, and the
        // tag is the name after it.
        assert_eq!(yaml("tags: [\"#rust\", '#notes']\n"), ["rust", "notes"]);
        assert_eq!(yaml("tags:\n  - \"#rust\"\n"), ["rust"]);
        // Mid-word, a `#` is just a character.
        assert_eq!(yaml("tags: [c#, f#]\n"), ["c#", "f#"]);
    }

    #[test]
    fn a_comma_inside_quotes_does_not_split() {
        assert_eq!(yaml("tags: [\"a, b\", c]\n"), ["a, b", "c"]);
    }

    #[test]
    fn an_apostrophe_inside_a_word_is_not_a_quote() {
        // A quote opens only where an item starts. Otherwise one possessive
        // swallows every comma and comment after it.
        assert_eq!(
            yaml("tags: [valentine's day, gifts]\n"),
            ["valentine's day", "gifts"]
        );
        assert_eq!(
            yaml("tags: [don't, stop]  # believing\n"),
            ["don't", "stop"]
        );
        assert_eq!(
            yaml("tags:\n  - valentine's day # x\n"),
            ["valentine's day"]
        );
        assert_eq!(yaml("tags: [rock'n'roll, jazz]\n"), ["rock'n'roll", "jazz"]);
        assert_eq!(
            yaml("tags: ['it''s', \"a, b\"]\n").len(),
            2,
            "quoted items still are"
        );
    }

    #[test]
    fn a_pipe_named_like_a_document_is_not_read() {
        // A FIFO blocks its reader until someone writes. The walk lists it;
        // the scan must not sit on it forever.
        let d = tempfile::tempdir().unwrap();
        let fifo = d.path().join("pipe.md");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .is_ok_and(|s| s.success());
        if !made {
            eprintln!("SKIP: mkfifo not available");
            return;
        }
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(tags_of(&fifo));
        });
        let got = rx.recv_timeout(std::time::Duration::from_secs(5));
        assert_eq!(got, Ok(Vec::new()), "the scan blocked on a pipe");
    }

    #[test]
    fn one_document_holds_a_tag_once_whatever_its_case() {
        assert_eq!(yaml("tags: [Rust, rust, RUST, notes]\n"), ["Rust", "notes"]);
    }

    #[test]
    fn a_tag_is_one_bounded_line_with_no_control_characters() {
        let long = "x".repeat(200);
        let tags = yaml(&format!(
            "tags: [\"a\u{7}b\", \"  spaced   out  \", {long}]\n"
        ));
        assert_eq!(tags[1], "spaced out");
        assert_eq!(tags[2].chars().count(), MAX_TAG_CHARS);
        assert!(tags.iter().all(|t| !t.chars().any(char::is_control)));
        let many: Vec<String> = (0..100).map(|i| format!("t{i}")).collect();
        assert_eq!(
            yaml(&format!("tags: [{}]\n", many.join(", "))).len(),
            MAX_TAGS_PER_FILE
        );
    }

    /// The core's finder, as `(body, is TOML)`.
    fn frontmatter(text: &str) -> Option<(&str, bool)> {
        carrel_core::frontmatter(text).map(|f| (&text[f.body], f.format == MetaFormat::Toml))
    }

    #[test]
    fn frontmatter_must_open_the_file_and_be_closed() {
        assert_eq!(
            frontmatter("---\ntags: [a]\n---\nbody"),
            Some(("tags: [a]\n", false))
        );
        assert_eq!(
            frontmatter("+++\ntags = [\"a\"]\n+++\n"),
            Some(("tags = [\"a\"]\n", true))
        );
        assert_eq!(frontmatter("---\na: b\n...\n"), Some(("a: b\n", false)));
        assert_eq!(
            frontmatter("\u{FEFF}---\na: b\n---\n"),
            Some(("a: b\n", false))
        );
        // A rule, then prose that happens to say `tags:` — not frontmatter.
        assert_eq!(frontmatter("---\ntags: [a]\n\nnever closed\n"), None);
        assert_eq!(frontmatter("\n---\ntags: [a]\n---\n"), None, "not first");
        assert_eq!(frontmatter("# Title\n\ntags: [a]\n"), None);
        assert_eq!(frontmatter("---"), None);
    }

    fn write(dir: &Path, name: &str, body: &str) -> Entry {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        Entry {
            path,
            mtime: SystemTime::UNIX_EPOCH,
        }
    }

    /// Openings that are, and are not, frontmatter. The same list as the
    /// core's `the_frontmatter_finder_agrees_with_the_parse`.
    const TRICKY: &[&str] = &[
        "---\ntags: [a]\ntitle: T\n---\n",
        "---\n\ntags: [a]\ntitle: T\n---\n",
        "---\n---\n",
        "---\n---\ntags: [a]\n---\n",
        "---\ntags: [a]\ntitle: T\n---\t\n",
        "---\t\ntags: [a]\ntitle: T\n---\n",
        "--- \ntags: [a]\ntitle: T\n---\n",
        "---\ntags: [a]\ntitle: T\n---  \n",
        "\u{feff}---\ntags: [a]\ntitle: T\n---\n",
        "---\ntags: [a]\ntitle: T\n...\n",
        "+++\ntags: [a]\ntitle: T\n...\n",
        "+++\ntags = [\"a\"]\ntitle = \"T\"\n+++\n",
        "+++\ntags: [a]\ntitle: T\n+++\n",
        "---\ntags = [a]\ntitle = T\n---\n",
        "+++\ntags = [\"a\"]\n---\n+++\n",
        "---\r\ntags: [a]\r\ntitle: T\r\n---\r\n",
        " ---\ntags: [a]\ntitle: T\n---\n",
        "----\ntags: [a]\ntitle: T\n----\n",
        "---\ntags: [a]\ntitle: T\n",
        "---\ntags: [a]\ntitle: T\n---",
        "---\ntags:[a]\ntitle:T\n---\n",
        "---\n  tags: [a]\n  title: T\n---\n",
        "---\n# a comment\ntags: [a]\n---\n",
        "---\nmeta:\n  title: Nested\ntags:\n\t- a\n---\n# Heading\n",
        "---\ntags: [a]\n---\n\n---\ntags: [b]\ntitle: Later\n---\n",
        "# Title\n\n---\ntags: [a]\ntitle: T\n---\n",
        "\n---\ntags: [a]\ntitle: T\n---\n",
        "---\n\"tags\": [q]\n'title': Quoted key\n---\n",
        "+++\n\"tags\" = [\"q\"]\ntitle=\"Tight\"\n+++\n",
        "---",
        "",
    ];

    /// The tag scan, the title scan and the card's tag buttons read
    /// frontmatter exactly where the reader shows a card, and read from it
    /// exactly what the card holds: no tags or title from a block the
    /// reader shows as a rule and a paragraph, none missed from one it
    /// shows as a card, and the card's own dialect rather than a guess.
    #[test]
    fn the_tag_and_title_scans_agree_with_the_reader() {
        use carrel_core::{BlockIdx, Document, NodeKind};
        let d = tempfile::tempdir().unwrap();
        let mut cards = 0;
        for (i, &src) in TRICKY.iter().enumerate() {
            let path = write(d.path(), &format!("{i}.md"), src).path;
            let doc = Document::parse(src);
            let card = (doc.block_count() > 0)
                .then(|| doc.node_for_block(BlockIdx(0)))
                .and_then(|n| match n.kind {
                    NodeKind::Metadata { format, .. } => Some((n, format)),
                    _ => None,
                });
            let (tags, title) = card.map_or((Vec::new(), None), |(n, format)| {
                cards += 1;
                let shown = doc.block_text(BlockIdx(0));
                let source = doc.copy_text(n.doc.clone()).unwrap();
                let tags = parse_tags(&source, format);
                let buttons: Vec<String> =
                    spans(shown, format).into_iter().map(|(_, t)| t).collect();
                let mut sorted = tags.clone();
                sorted.sort();
                let mut buttons = buttons;
                buttons.sort();
                assert_eq!(buttons, sorted, "the card's buttons: {src:?}");
                (tags, crate::scan::frontmatter_title(&source, format))
            });
            assert_eq!(tags_of(&path), tags, "tags: {src:?}");
            let scanned = crate::scan::title_of(&path);
            match title {
                Some(t) => assert_eq!(scanned, Some(t), "title: {src:?}"),
                // No card title: whatever the scan finds must be a heading
                // the reader shows, never a frontmatter value.
                None => assert!(
                    scanned.as_ref().is_none_or(|s| {
                        (0..doc.block_count()).any(|b| {
                            let b = BlockIdx(b as u32);
                            matches!(doc.node_for_block(b).kind, NodeKind::Heading { .. })
                                && doc.block_text(b) == s
                        })
                    }),
                    "title {scanned:?}: {src:?}"
                ),
            }
        }
        assert!((8..TRICKY.len() - 8).contains(&cards), "{cards}");
    }

    #[test]
    fn a_file_is_read_for_its_frontmatter_and_nothing_else() {
        let d = tempfile::tempdir().unwrap();
        let tagged = write(d.path(), "a.md", "---\ntags: [rust]\n---\n\n# A\n");
        let body = write(d.path(), "b.md", "# B\n\ntags: [nope]\n#inline tag\n");
        let rule = write(d.path(), "c.md", "---\n\ntags: [nope]\n\nprose\n");
        assert_eq!(tags_of(&tagged.path), ["rust"]);
        assert_eq!(
            tags_of(&body.path),
            ["inline"],
            "the body's `tags:` line is not frontmatter; its #tag is a tag"
        );
        assert!(
            tags_of(&rule.path).is_empty(),
            "an unclosed fence is a rule"
        );
        assert!(tags_of(&d.path().join("missing.md")).is_empty());
    }

    #[test]
    fn frontmatter_longer_than_the_first_read_is_still_read_whole() {
        // The tags sit past HEAD_BYTES. Parsing the truncated head would
        // find `tags:` with half its list — or none of it.
        let d = tempfile::tempdir().unwrap();
        let filler = format!("note: {}\n", "x".repeat(100)).repeat(HEAD_BYTES / 100 + 5);
        let e = write(
            d.path(),
            "big.md",
            &format!("---\n{filler}tags:\n  - late\n  - later\n---\nbody\n"),
        );
        assert_eq!(tags_of(&e.path), ["late", "later"]);
    }

    fn collect(rx: &Receiver<Msg>) -> Index {
        let mut index = Index::default();
        while let Ok(msg) = rx.recv() {
            match msg {
                Msg::Found(t, _) => index.tagged.push(t),
                Msg::Done { read, capped, .. } => {
                    index.read = read;
                    index.capped = capped;
                    break;
                }
            }
        }
        index
    }

    #[test]
    fn the_scan_reports_tagged_documents_and_how_many_it_read() {
        let d = tempfile::tempdir().unwrap();
        let entries = vec![
            write(d.path(), "a.md", "---\ntitle: Alpha\ntags: [rust]\n---\n"),
            write(d.path(), "b.md", "no frontmatter\n"),
            write(d.path(), "c.md", "---\ntags: [rust, notes]\n---\n# Gamma\n"),
        ];
        let index = collect(&spawn(Source::Entries(entries.clone()), false, 1));
        assert_eq!(index.read, 3);
        assert_eq!(index.tagged.len(), 2);
        assert!(index.tagged.iter().all(|t| t.title.is_none()));

        let titled = collect(&spawn(Source::Entries(entries), true, 2));
        let names: Vec<_> = titled.tagged.iter().map(|t| t.title.as_deref()).collect();
        assert_eq!(names, [Some("Alpha"), Some("Gamma")]);
    }

    fn doc(path: &str, secs: u64, tags: &[&str]) -> Tagged {
        Tagged {
            path: PathBuf::from(path),
            mtime: SystemTime::UNIX_EPOCH + Duration::from_secs(secs),
            tags: tags.iter().map(ToString::to_string).collect(),
            title: None,
        }
    }

    #[test]
    fn tags_group_without_regard_to_case_most_used_first() {
        let index = Index {
            tagged: vec![
                doc("/r/a.md", 1, &["Rust", "notes"]),
                doc("/r/b.md", 3, &["rust"]),
                doc("/r/c.md", 2, &["rust", "ideas"]),
            ],
            read: 3,
            capped: false,
        };
        let g = groups(&index);
        let shape: Vec<(&str, usize)> = g.iter().map(|g| (g.name.as_str(), g.docs.len())).collect();
        assert_eq!(shape, [("rust", 3), ("ideas", 1), ("notes", 1)]);
        let newest_first: Vec<_> = g[0].docs.iter().map(|d| d.path.to_str().unwrap()).collect();
        assert_eq!(newest_first, ["/r/b.md", "/r/c.md", "/r/a.md"]);
    }

    #[test]
    fn the_spelling_shown_does_not_depend_on_scan_order() {
        let a = doc("/r/a.md", 1, &["API"]);
        let b = doc("/r/b.md", 2, &["api"]);
        let name = |tagged: Vec<Tagged>| {
            groups(&Index {
                tagged,
                read: 2,
                capped: false,
            })[0]
                .name
                .clone()
        };
        assert_eq!(name(vec![a.clone(), b.clone()]), name(vec![b, a]));
    }

    #[test]
    fn the_document_has_a_section_per_tag_and_a_link_per_document() {
        let mut titled = doc("/r/sub dir/b.md", 3, &["rust"]);
        titled.title = Some("On *Rust*".into());
        let index = Index {
            tagged: vec![doc("/r/a.md", 1, &["rust", "notes"]), titled],
            read: 9,
            capped: false,
        };
        let md = render(Path::new("/r"), &index);
        assert!(md.starts_with("# Tags\n"), "{md}");
        assert!(md.contains("2 tags in 2 documents of the 9 read"), "{md}");
        assert!(md.contains("## rust (2)\n"), "{md}");
        assert!(md.contains("## notes (1)\n"), "{md}");
        // Newest first, the title where there is one, and a target that
        // survives the space in its path.
        let rust = md.split("## rust (2)").nth(1).unwrap();
        let first = rust.lines().find(|l| l.starts_with("- ")).unwrap();
        assert_eq!(first, "- [On \\*Rust\\*](<sub dir/b.md>)");
        assert!(rust.contains("- [a\\.md](<a.md>)"), "{md}");
        assert!(
            !md.contains("Stopped at"),
            "not capped, so it does not say so"
        );
    }

    #[test]
    fn a_hostile_tag_cannot_write_markdown_into_the_document() {
        // Each of these would otherwise become a link, a heading, an image,
        // emphasis or raw HTML in a document the reader trusts as carrel's.
        let index = Index {
            tagged: vec![doc(
                "/r/<a>.md",
                1,
                &[
                    "[click](http://evil.example)",
                    "# not a heading",
                    "![x](y.png)",
                    "<script>",
                    "`code`",
                    "*bold*",
                ],
            )],
            read: 1,
            capped: false,
        };
        let md = render(Path::new("/r"), &index);
        let parsed = carrel_core::Document::parse(&md);
        assert_eq!(
            parsed.links.len(),
            6,
            "one link per tag section — the file's — and none from a tag: {:?}",
            parsed.links
        );
        assert!(
            parsed.links.iter().all(|l| &**l == "%3Ca%3E.md"),
            "{:?}",
            parsed.links
        );
        let headings: Vec<&str> = parsed
            .nodes
            .iter()
            .filter(|n| matches!(n.kind, carrel_core::NodeKind::Heading { .. }))
            .map(|n| &parsed.text[n.doc.start as usize..n.doc.end as usize])
            .collect();
        // The link-shaped tag holds slashes, so it nests: a parent section
        // `[click](http:` and its child. Still characters, never a link.
        assert_eq!(
            headings.len(),
            8,
            "`# Tags`, one per tag, one nested parent: {headings:?}"
        );
        for literal in [
            "[click](http: (1)",
            "[click](http:/evil.example) (1)",
            "<script> (1)",
            "*bold* (1)",
        ] {
            assert!(
                headings.contains(&literal),
                "{literal:?} must read as its own characters: {headings:?}"
            );
        }
    }

    #[test]
    fn a_folder_can_be_walked_when_there_is_no_file_list() {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), "a.md", "---\ntags: [rust]\n---\n");
        write(d.path(), "b.md", "plain\n");
        let index = collect(&spawn(Source::Walk(d.path().to_path_buf()), false, 1));
        assert_eq!(index.read, 2);
        assert_eq!(index.tagged.len(), 1);
    }

    #[test]
    fn each_tag_is_found_where_it_is_written() {
        let at_in = |body: &str, format| -> Vec<(String, String)> {
            spans(body, format)
                .into_iter()
                .map(|(r, tag)| (body[r].to_string(), tag))
                .collect()
        };
        let at = |body: &str| at_in(body, MetaFormat::Yaml);
        let pair = |a: &str| (a.to_string(), a.to_string());
        assert_eq!(
            at("title: About tags\ntags: [rust, web dev]\ndate: rust\n"),
            [pair("rust"), pair("web dev")],
            "only inside the tags key — not the title, not a later key"
        );
        assert_eq!(
            at("tags:\n  - rustlang\n  - rust\n"),
            [pair("rustlang"), pair("rust")],
            "the short one does not claim part of the long one"
        );
        assert_eq!(
            at_in("tags = [\"notes\", \"#inbox\"]\n", MetaFormat::Toml),
            [pair("notes"), pair("inbox")],
            "TOML, and a tag written with its hash is the name after it"
        );
        assert_eq!(
            at("tags: [\"web  dev\", dev, Rust]\n"),
            [
                ("web  dev".to_string(), "web dev".to_string()),
                pair("dev"),
                pair("Rust")
            ],
            "a tag's spaces are its own: `dev` is the second one, not the end of the first"
        );
        assert_eq!(
            at("tags: # about rust\n  - rust\n"),
            [pair("rust")],
            "the item"
        );
        let body = "tags: # about rust\n  - rust\n";
        assert_eq!(
            spans(body, MetaFormat::Yaml)[0].0.start,
            body.rfind("rust").unwrap(),
            "not the comment"
        );
        assert_eq!(
            at("tag: rust\ntitle: x\ntags: [web, rust]\n"),
            [pair("rust"), pair("web")],
            "both keys are tags"
        );
        assert_eq!(at("tags: [rust, rust]\n").len(), 1, "one tag, one place");
        assert!(at("title: no tags\n").is_empty());
        assert!(at("tags: []\n").is_empty());
    }

    #[test]
    fn inline_tags_join_the_frontmatter_ones_once_each() {
        assert_eq!(
            tags_in("---\ntags: [Rust]\n---\n\nOn #rust and #notes/daily.\n"),
            ["Rust", "notes/daily"]
        );
        assert_eq!(
            tags_in("# C# notes\n\nissue #42, `#code`, #fff, a ## b\n\n```\n#tag\n```\n"),
            Vec::<String>::new(),
            "nothing the reader does not paint as a tag"
        );
        // A hostile file: a hundred thousand tags is still one bounded list.
        let many = (0..100_000).fold(String::new(), |mut s, i| {
            use std::fmt::Write as _;
            let _ = write!(s, "#t{i} ");
            s
        });
        assert_eq!(tags_in(&many).len(), MAX_TAGS_PER_FILE);
    }

    #[test]
    fn the_byte_scan_rules_out_only_what_cannot_be_a_tag() {
        for no in [
            "# Heading\n\n## Part\n\nprose\n",
            "C# and F#, page#frag\n",
            "#[derive(Debug)]\n#{x} #(y) #/usr\n",
            "a # b ##\n",
            "",
        ] {
            assert!(!may_hold_inline_tag(no.as_bytes()), "{no:?}");
        }
        for maybe in [
            "#tag",
            "a #tag",
            "a\n#1st",
            "x\u{a0}#nbsp",
            "**#bold**",
            "|#cell|",
        ] {
            assert!(may_hold_inline_tag(maybe.as_bytes()), "{maybe:?}");
        }
    }

    /// Every source the core reads a tag from must get past the byte scan,
    /// or the tags page silently loses a tag the reader shows.
    #[test]
    fn the_byte_scan_never_hides_a_tag_the_reader_shows() {
        for src in [
            "#a",
            "x #a",
            "x\u{a0}#a",
            "x\u{3000}#日本",
            "| #cell |\n|-|\n",
            "- #item",
            "x\\\n#after",
            "x\t#tab",
            "**#bold**",
            "_#em_",
            "|a|#cell|\n|-|-|\n",
            ">#quote",
            "<b>#html</b>",
            "x #\\_esc",
            "x #&eacute;t",
            "x\u{a0}#nb",
            "#1st",
        ] {
            let doc = carrel_core::Document::parse(src);
            assert!(!doc.tags.is_empty(), "the fixture must hold a tag: {src:?}");
            assert!(may_hold_inline_tag(src.as_bytes()), "{src:?}");
        }
    }

    /// The same promise over every short string of markdown's punctuation:
    /// a generated corpus, because the hand-picked list above is exactly
    /// the cases someone thought of.
    #[test]
    fn the_byte_scan_never_hides_a_tag_in_generated_markdown() {
        const ALPHABET: &[&str] = &[
            "#", "a", "1", " ", "\n", "*", "_", "\\", "&", "`", "[", "]", "(", ")", "|", ">", "-",
            "é", ";", "/", "!", "<", "~", "^", "$", ":", "&#35;", "&amp;", "\u{a0}", "![x](u)",
            "[x]", "[x]: u\n", "<b>", "{", "=", "@", "%", "+", "\"", "'", ".", ",", "?", "[^1]",
            "[[w]]", "}",
        ];
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut tagged = 0;
        for _ in 0..50_000 {
            let mut src = String::new();
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let len = 1 + (seed % 14) as usize;
            for k in 0..len {
                let pick = (seed.rotate_left(k as u32 * 5) as usize) % ALPHABET.len();
                src.push_str(ALPHABET[pick]);
            }
            let doc = carrel_core::Document::parse(&src);
            if !doc.tags.is_empty() {
                tagged += 1;
                assert!(may_hold_inline_tag(src.as_bytes()), "{src:?}");
            }
        }
        assert!(tagged > 100, "the corpus must hold tags to test: {tagged}");
    }

    #[test]
    fn tag_keys_nest_at_slashes_and_ignore_case_and_blanks() {
        assert_eq!(key("Project/Alpha"), "project/alpha");
        assert_eq!(key("project//alpha/"), "project/alpha");
        assert_eq!(key(" project / alpha "), "project/alpha");
        assert_eq!(key("rust"), "rust");
        assert_eq!(key("//"), "//", "nothing but slashes keeps itself");
    }

    #[test]
    fn nested_tags_group_under_their_parents_with_totals() {
        let index = Index {
            tagged: vec![
                doc("/r/a.md", 1, &["project/alpha"]),
                doc("/r/b.md", 2, &["Project/Beta", "project"]),
                doc("/r/c.md", 3, &["project/alpha/v2", "rust"]),
                doc("/r/d.md", 4, &["project/alpha", "project/alpha//"]),
                doc("/r/e.md", 5, &["zebra"]),
                doc("/r/f.md", 6, &["zebra"]),
            ],
            read: 6,
            capped: false,
        };
        let g = groups(&index);
        let shape: Vec<(&str, usize, usize, usize)> = g
            .iter()
            .map(|g| (g.name.as_str(), g.depth, g.docs.len(), g.total))
            .collect();
        assert_eq!(
            shape,
            [
                ("project", 0, 1, 4),
                ("project/alpha", 1, 2, 3),
                ("project/alpha/v2", 2, 1, 1),
                ("Project/Beta", 1, 1, 1),
                ("zebra", 0, 2, 2),
                ("rust", 0, 1, 1),
            ],
            "a parent first, its children after it — not interleaved with \
             another tag by count — each document counted once"
        );
        let md = render(Path::new("/r"), &index);
        for line in [
            "## project (4)\n",
            "### project\\/alpha (3)\n",
            "#### project\\/alpha\\/v2 (1)\n",
            "### Project\\/Beta (1)\n",
            "## rust (1)\n",
        ] {
            assert!(md.contains(line), "{line:?} in\n{md}");
        }
    }

    #[test]
    fn a_parent_exists_when_only_its_children_are_used() {
        let index = Index {
            tagged: vec![doc("/r/a.md", 1, &["area/home"])],
            read: 1,
            capped: false,
        };
        let shape: Vec<(String, usize, usize)> = groups(&index)
            .into_iter()
            .map(|g| (g.name, g.docs.len(), g.total))
            .collect();
        assert_eq!(
            shape,
            [("area".to_string(), 0, 1), ("area/home".to_string(), 1, 1)]
        );
    }

    #[test]
    fn nesting_deeper_than_markdown_headings_stays_on_the_page() {
        let index = Index {
            tagged: vec![doc("/r/a.md", 1, &["a/b/c/d/e/f/g"])],
            read: 1,
            capped: false,
        };
        let md = render(Path::new("/r"), &index);
        let parsed = carrel_core::Document::parse(&md);
        let headings = parsed
            .nodes
            .iter()
            .filter(|n| matches!(n.kind, carrel_core::NodeKind::Heading { .. }))
            .count();
        assert_eq!(headings, 8, "the title and all seven levels:\n{md}");
        assert_eq!(parsed.links.len(), 1, "the document, once, under its tag");
    }

    #[test]
    fn a_capped_scan_says_so() {
        let index = Index {
            tagged: vec![doc("/r/a.md", 1, &["x"])],
            read: 1,
            capped: true,
        };
        assert!(render(Path::new("/r"), &index).contains("Stopped at"));
    }
}
