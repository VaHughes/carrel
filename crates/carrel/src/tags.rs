//! Frontmatter tags across the folder, read as a document.
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
//! the already-scanned entry list and reads the HEAD of each file, so the
//! filesystem never blocks a frame. Most files do not open with a frontmatter
//! fence and cost one short read.
//!
//! # What counts as a tag
//!
//! `tags:` (or `tag:`) in a YAML `---` block, `tags = […]` in a TOML `+++`
//! one. Not a YAML parse — a reader must never fail on exotic YAML — but the
//! three spellings people actually write are all read: a flow list
//! (`[a, b]`), a block list (`- a`), and a bare scalar (`a, b` or `a b`).
//!
//! **Inline `#tags` in the body are not read.** A `#word` in markdown is also
//! a heading typed without its space, an issue number, a hex color, a URL
//! fragment and a C preprocessor line; telling those apart needs the whole
//! file parsed, for every file, to find tags the author did not declare.
//!
//! NO RATATUI — `scripts/check-discipline.sh` rule 6.

use std::collections::HashMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::SystemTime;

use crate::scan::Entry;

/// The first read. A frontmatter block almost always ends inside it, and a
/// file that does not open with a fence is dismissed from its first bytes.
const HEAD_BYTES: usize = 8 * 1024;
/// How far to keep reading for a closing fence before deciding the opening
/// `---` was a rule, not frontmatter. A truncated block must not be parsed:
/// half a tag list is a wrong answer that looks like a right one.
const FRONTMATTER_MAX: u64 = 64 * 1024;
/// Tags kept per document. A list longer than this is data, not labeling.
const MAX_TAGS_PER_FILE: usize = 32;
/// Characters kept per tag.
const MAX_TAG_CHARS: usize = 64;
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
pub fn spawn(entries: Vec<Entry>, titles: bool, generation: u64) -> Receiver<Msg> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let (mut read, mut found, mut capped) = (0usize, 0usize, false);
        for e in entries {
            if found >= MAX_TAGGED {
                capped = true;
                break;
            }
            read += 1;
            let tags = tags_of(&e.path);
            if tags.is_empty() {
                continue;
            }
            let title = titles.then(|| crate::scan::title_of(&e.path)).flatten();
            let t = Tagged {
                path: e.path,
                mtime: e.mtime,
                tags,
                title,
            };
            if tx.send(Msg::Found(t, generation)).is_err() {
                return; // receiver gone: the reader moved on
            }
            found += 1;
        }
        let _ = tx.send(Msg::Done {
            read,
            capped,
            generation,
        });
    });
    rx
}

/// The tags a file's frontmatter declares. Empty when it has no frontmatter,
/// no tags, or cannot be read.
#[must_use]
pub fn tags_of(path: &Path) -> Vec<String> {
    // Regular files only. The walk lists whatever is named `*.md`, and a
    // FIFO blocks whoever opens it until someone writes — which would hold
    // this thread, and the request it serves, forever.
    if !std::fs::metadata(path).is_ok_and(|m| m.is_file()) {
        return Vec::new();
    }
    let Ok(mut f) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let mut buf = vec![0u8; HEAD_BYTES];
    let Ok(n) = f.read(&mut buf) else {
        return Vec::new();
    };
    buf.truncate(n);
    // Dismiss the common case before decoding anything.
    let start = buf.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&buf);
    if !(start.starts_with(b"---") || start.starts_with(b"+++")) {
        return Vec::new();
    }
    if frontmatter(&String::from_utf8_lossy(&buf)).is_none() && n == HEAD_BYTES {
        // No closing fence yet and the buffer was full: keep reading, to a
        // bound, rather than parse a block we have only part of.
        let _ = f
            .take(FRONTMATTER_MAX - HEAD_BYTES as u64)
            .read_to_end(&mut buf);
    }
    let head = String::from_utf8_lossy(&buf);
    frontmatter(&head).map_or_else(Vec::new, |(body, toml)| parse_tags(body, toml))
}

/// The body of a leading frontmatter block and whether it is TOML, or `None`
/// when the text does not open with a CLOSED one.
fn frontmatter(text: &str) -> Option<(&str, bool)> {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let (first, rest) = text.split_once('\n')?;
    let fence = first.trim_end();
    let toml = match fence {
        "---" => false,
        "+++" => true,
        _ => return None,
    };
    let mut at = 0usize;
    for line in rest.split_inclusive('\n') {
        let t = line.trim_end();
        // YAML may also close a document with `...`.
        if t == fence || (!toml && t == "...") {
            return Some((&rest[..at], toml));
        }
        at += line.len();
    }
    None
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
pub fn parse_tags(body: &str, toml: bool) -> Vec<String> {
    let lines: Vec<&str> = body.lines().collect();
    let mut out: Vec<String> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        // Top-level keys only: an indented `tags:` belongs to something else.
        if line.starts_with([' ', '\t']) {
            continue;
        }
        let Some((key, value)) = line.split_once(if toml { '=' } else { ':' }) else {
            continue;
        };
        let key = key.trim();
        if !(key.eq_ignore_ascii_case("tags") || key.eq_ignore_ascii_case("tag")) {
            continue;
        }
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

/// One tag and the documents that carry it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group<'a> {
    /// The spelling shown: the one used by the most documents, and among
    /// equals the first alphabetically, so the choice never depends on the
    /// order the scan happened to find them in.
    pub name: String,
    /// Newest first.
    pub docs: Vec<&'a Tagged>,
}

/// The index grouped by tag: most-used first, then alphabetically.
#[must_use]
pub fn groups(index: &Index) -> Vec<Group<'_>> {
    let mut by_key: HashMap<String, (HashMap<&str, usize>, Vec<&Tagged>)> = HashMap::new();
    for doc in &index.tagged {
        for tag in &doc.tags {
            let slot = by_key.entry(tag.to_lowercase()).or_default();
            *slot.0.entry(tag.as_str()).or_default() += 1;
            slot.1.push(doc);
        }
    }
    let mut out: Vec<(String, Group<'_>)> = by_key
        .into_iter()
        .map(|(key, (spellings, mut docs))| {
            let name = spellings
                .into_iter()
                .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(a.0)))
                .map(|(s, _)| s.to_string())
                .unwrap_or_default();
            docs.sort_by(|a, b| b.mtime.cmp(&a.mtime).then_with(|| a.path.cmp(&b.path)));
            (key, Group { name, docs })
        })
        .collect();
    out.sort_by(|a, b| {
        b.1.docs
            .len()
            .cmp(&a.1.docs.len())
            .then_with(|| a.0.cmp(&b.0))
    });
    out.into_iter().map(|(_, g)| g).collect()
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
        let _ = writeln!(out, "## {} ({})\n", escape(&g.name), g.docs.len());
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
        parse_tags(body, false)
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
            parse_tags("tags = [\"rust\", \"web dev\"]\n", true),
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

    #[test]
    fn a_file_is_read_for_its_frontmatter_and_nothing_else() {
        let d = tempfile::tempdir().unwrap();
        let tagged = write(d.path(), "a.md", "---\ntags: [rust]\n---\n\n# A\n");
        let body = write(d.path(), "b.md", "# B\n\ntags: [nope]\n#inline tag\n");
        let rule = write(d.path(), "c.md", "---\n\ntags: [nope]\n\nprose\n");
        assert_eq!(tags_of(&tagged.path), ["rust"]);
        assert!(
            tags_of(&body.path).is_empty(),
            "the body is not frontmatter"
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
        let index = collect(&spawn(entries.clone(), false, 1));
        assert_eq!(index.read, 3);
        assert_eq!(index.tagged.len(), 2);
        assert!(index.tagged.iter().all(|t| t.title.is_none()));

        let titled = collect(&spawn(entries, true, 2));
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
        assert_eq!(headings.len(), 7, "`# Tags` and one per tag: {headings:?}");
        for literal in [
            "[click](http://evil.example) (1)",
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
    fn a_capped_scan_says_so() {
        let index = Index {
            tagged: vec![doc("/r/a.md", 1, &["x"])],
            read: 1,
            capped: true,
        };
        assert!(render(Path::new("/r"), &index).contains("Stopped at"));
    }
}
