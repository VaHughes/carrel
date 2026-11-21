//! Unified diffs and `git log` output, turned into markdown.
//!
//! **A raw diff is not markdown**, and parsing one as markdown is nonsense:
//! `---` is a setext rule, indented lines become code blocks, and
//! `diff --git a/x b/x` is a paragraph. So the input is *transformed* before
//! it reaches the parser rather than given a document builder of its own.
//!
//! The transform is deliberately small, because everything downstream is then
//! free and correct by construction:
//!
//! - a **heading per file** means [`Document::section_path`] sees files as
//!   sections, so folding, the breadcrumb band and the outline picker all work
//!   on a diff with no new code;
//! - a **`diff`-tagged fence** per file means syntect's bundled Diff syntax
//!   highlights it, once `TokenKind::Inserted`/`Deleted` exist to receive the
//!   scopes;
//! - search-through-resize, selection and every position invariant hold,
//!   because what the reader ends up with is an ordinary `Document`.
//!
//! **Known and accepted:** [`Document::to_src`] maps into the *synthesized*
//! markdown, not the original diff. `to_src` serves "open the source here" and
//! reload re-anchoring; a piped diff has no source file to open, so nothing
//! real is lost. Written down rather than discovered later.
//!
//! [`Document::section_path`]: crate::Document::section_path
//! [`Document::to_src`]: crate::Document::to_src

use std::collections::BTreeMap;
use std::fmt::Write as _;

/// How far in to look for a diff marker before giving up.
const SNIFF_LINES: usize = 40;

/// Does this look like a unified diff, or `git log`/`git show` output?
///
/// **Narrow on purpose, and never applied to a `.md` file.** The caller
/// restricts this to piped input and to files named `.diff`/`.patch`; that
/// single rule removes the whole class of "my markdown document *about* diffs
/// got mangled" bugs. What is left is a strong marker in the first
/// `SNIFF_LINES` lines.
#[must_use]
pub fn looks_like_diff(text: &str) -> bool {
    text.lines().take(SNIFF_LINES).any(|l| {
        l.starts_with("diff --git ")
            || l.starts_with("Index: ")
            || is_commit_line(l)
            || is_hunk_header(l)
    })
}

/// `@@ -1,3 +1,4 @@` — the only hunk shape unified diffs produce.
fn is_hunk_header(l: &str) -> bool {
    let Some(rest) = l.strip_prefix("@@ -") else {
        return false;
    };
    let Some((range, tail)) = rest.split_once(" +") else {
        return false;
    };
    is_line_range(range)
        && tail
            .split_once(" @@")
            .is_some_and(|(r, _)| is_line_range(r))
}

/// How many lines a hunk header says follow on each side, old and new:
/// `@@ -1,3 +1,4 @@` is `(3, 4)`, and a side with no count is one line.
pub(crate) fn hunk_lines(l: &str) -> Option<(usize, usize)> {
    if !is_hunk_header(l) {
        return None;
    }
    let (old, rest) = l.strip_prefix("@@ -")?.split_once(" +")?;
    let (new, _) = rest.split_once(" @@")?;
    let count = |range: &str| {
        range
            .split_once(',')
            .map_or(Some(1), |(_, n)| n.parse().ok())
    };
    Some((count(old)?, count(new)?))
}

/// `12` or `12,3`.
fn is_line_range(s: &str) -> bool {
    let (a, b) = s.split_once(',').unwrap_or((s, "0"));
    !a.is_empty()
        && a.bytes().all(|c| c.is_ascii_digit())
        && !b.is_empty()
        && b.bytes().all(|c| c.is_ascii_digit())
}

/// `commit 4f2a1b…` with a plausible object name.
fn is_commit_line(l: &str) -> bool {
    l.strip_prefix("commit ").is_some_and(|rest| {
        let sha = rest.split_whitespace().next().unwrap_or("");
        sha.len() >= 7 && sha.bytes().all(|c| c.is_ascii_hexdigit())
    })
}

/// The fence that can safely wrap `body`.
///
/// A diff **of a markdown file** contains fences, and a three-backtick fence
/// would end at the first one. `CommonMark` closes a fence only on a run at
/// least as long as the opener, so outrun the longest run inside.
fn fence_for(body: &str) -> String {
    let mut longest = 0usize;
    let mut run = 0usize;
    for c in body.chars() {
        if c == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    "`".repeat(longest.max(2) + 1)
}

/// The file a `diff --git a/X b/X` line is about.
///
/// Takes the **b-side**, because that is the file as it now is — which is
/// also the new name of a rename, and, for a deletion, still the real path:
/// git spells a deletion out on the `deleted file mode` and `+++ /dev/null`
/// lines, never on this one.
///
/// The separator is the ` b/`, **not** the first space. Git does not quote a
/// path merely because it contains spaces, so splitting at the first space
/// turned `a/my notes.md b/my notes.md` into the heading
/// `notes.md b/my notes.md` — and a mangled heading is a mangled section
/// index, breadcrumb and outline, because those are all read back out of the
/// synthesized markdown. The LAST ` b/` is the one to cut at, so a rename to
/// a name containing ` b/` still ends whole; a path that contains ` b/` on
/// *both* sides is genuinely ambiguous from this line alone and is not worth
/// a second parser.
fn path_of(l: &str) -> Option<&str> {
    let rest = l.strip_prefix("diff --git ")?;
    if let Some((_, b)) = rest.rsplit_once(" b/") {
        return Some(b);
    }
    // No `b/` at all: `diff.mnemonicPrefix` writes `i/… w/…` (as this
    // machine's git is configured to, which is how the shape was checked) and
    // `--no-prefix` writes bare paths. Nothing here can tell those two halves
    // apart once a path has a space in it, so keep the old first-space split
    // rather than regress such a diff to "(unknown file)".
    rest.split_once(' ').map(|(_, b)| b)
}

/// A file's accumulated hunks, and the counts to put beside its heading.
#[derive(Default)]
struct FileDiff {
    heading: String,
    body: String,
    added: usize,
    removed: usize,
}

impl FileDiff {
    fn flush(self, out: &mut String) {
        if self.heading.is_empty() && self.body.trim().is_empty() {
            return;
        }
        if !self.heading.is_empty() {
            out.push_str("## ");
            out.push_str(&escape_inline(&self.heading));
            if self.added > 0 || self.removed > 0 {
                let _ = write!(out, "  +{} −{}", self.added, self.removed);
            }
            out.push_str("\n\n");
        }
        if !self.body.trim().is_empty() {
            let fence = fence_for(&self.body);
            out.push_str(&fence);
            out.push_str("diff\n");
            out.push_str(&self.body);
            if !self.body.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&fence);
            out.push_str("\n\n");
        }
    }
}

/// Write the text between commits and files — a commit's headers, its
/// message, any prose around a bare diff — as markdown that cannot escape
/// its section.
///
/// Paragraphs (split at blank lines) reflow, because a commit body is prose
/// hard-wrapped at 72 columns. Two kinds of paragraph are records instead,
/// one per line, joined with hard breaks:
///
/// - the **header block** git writes unindented under `commit` (`Merge:`,
///   `Author:`, `Date:`, and `--format=fuller`'s four), and
/// - the **trailer block** closing a message (`Co-Authored-By:`,
///   `Signed-off-by:`), when it is not also the subject line — a
///   conventional-commit subject (`fix: …`) has the same shape.
///
/// Inside a paragraph, some lines are records too, each kept on its own
/// line by a hard break around it:
///
/// - a **trailer** run closing the last paragraph after the subject, even
///   with no blank line above it — git's own trailer parsing reads the
///   closing lines of the message, and a `Signed-off-by:` written straight
///   under the body is still one;
/// - an **indented** line, four columns or more deeper than the message —
///   code, a quoted command, an aligned table — which reflowed into a run
///   of words; and
/// - a **comment** line opening with `#`, which is what git writes for a
///   merge's `# Conflicts:` list.
///
/// A record is shown exactly as written ([`record_safe`]): no markup, no
/// autolink eating a `<mail>`'s brackets.
///
/// A commit message is someone else's text, and its markdown would
/// otherwise be parsed as structure: a `# …` line opening a section that
/// steals the commit's folding and outline entry, a fence or `<pre>`
/// swallowing every commit after it. [`neutralize`] escapes the few line
/// starts that open a block; everything inline (`code`, *emphasis*, links)
/// keeps working, and so do lists, which commit bodies use. Smart
/// punctuation's dashes do not ([`escape_dashes`]): `--` in a commit
/// message is a flag or an HTML comment, not an en dash. Nor does raw
/// HTML: a message is not HTML, and a tag the reader would strip is a
/// word of it (`--target <sha>`, `Vec<String>`), so [`settle`] escapes the
/// `<` of every tag the parse finds. An autolink is a link, and stays one.
///
/// Every backslash this adds is planned as a [`Plan`] and then checked
/// against the parse of the text it produces ([`settle`]), never against
/// a parse of something else.
fn emit_message(message: &[&str], out: &mut String) {
    // A lone CR is a line ending to the markdown parser but not to
    // `str::lines`, so the line start after one was never neutralized:
    // `body\r# stolen\r```` stole a heading and opened a fence over every
    // later commit. It is shown as the space it looks like.
    let spaced: Vec<std::borrow::Cow<'_, str>> = message
        .iter()
        .map(|l| {
            if l.contains('\r') {
                l.replace('\r', " ").into()
            } else {
                (*l).into()
            }
        })
        .collect();
    let message: Vec<&str> = spaced.iter().map(AsRef::as_ref).collect();
    let mut paras: Vec<&[&str]> = Vec::new();
    for chunk in message.split(|l| l.trim().is_empty()) {
        if !chunk.is_empty() {
            paras.push(chunk);
        }
    }
    let header = paras
        .first()
        .is_some_and(|p| p.iter().all(|l| is_header_line(l)));
    let body = usize::from(header);
    // The message's own indent — git's four columns, or none for prose
    // around a bare diff — from which a deeper line counts as indented.
    let base = paras[body.min(paras.len())..]
        .iter()
        .flat_map(|p| p.iter())
        .map(|l| indent_cols(l))
        .min()
        .unwrap_or(0);
    for (i, p) in paras.iter().enumerate() {
        // The trailer run: the last paragraph's closing trailer-shaped
        // lines. Never in the subject's paragraph, whose own first line
        // has the shape whenever it is a conventional-commit subject.
        let trailers = if i == 0 && header {
            0
        } else if i + 1 == paras.len() && i > body {
            p.len()
                - p.iter()
                    .rev()
                    .take_while(|l| is_trailer_line(l.trim()))
                    .count()
        } else {
            p.len()
        };
        // An indented line is four columns deeper than the text it sits
        // under: the message, or the list item it continues. Measured from
        // the message alone, an item's second line — indented to the item's
        // text, as people wrap them — was a record, its `code` shown with
        // its backticks.
        let mut item: Option<usize> = None;
        let records: Vec<bool> = p
            .iter()
            .enumerate()
            .map(|(j, l)| {
                let cols = indent_cols(l);
                if item.is_some_and(|text| cols < text) {
                    item = None;
                }
                let is = j >= trailers
                    || cols >= item.unwrap_or(base) + 4
                    || l.trim_start().starts_with('#');
                if !is && let Some(marker) = list_marker(l.trim_start()) {
                    item = Some(cols + marker);
                }
                is
            })
            .collect();
        let record = |j: usize| records[j];
        let mut plan = Plan::default();
        for (j, line) in p.iter().enumerate() {
            if record(j) {
                record_safe(line.trim(), &mut plan);
            } else {
                block_safe(line.trim(), &mut plan);
            }
            // A hard break around a record. Where the parser would not read
            // it as one — the line ends in the author's own break, a list
            // item follows, the message's markdown ends the paragraph first
            // — `settle` takes it out again.
            if j + 1 < p.len() && (record(j) || record(j + 1)) {
                plan.esc.insert(plan.base.len(), Esc::Break);
            }
            plan.base.push('\n');
        }
        escape_dashes(&mut plan);
        out.push_str(&settle(plan));
        out.push('\n');
    }
}

/// Who added a backslash, which decides what [`settle`] may do with it
/// when the parse says it would be shown.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Esc {
    /// Keeps a line start, or a record's character, literal. Never taken
    /// out: whatever would show it is broken open instead.
    Keep,
    /// Keeps a hyphen pair from smart punctuation, or a `$` from math.
    /// Neither applies inside a code span, an autolink or raw HTML, so
    /// there it is taken out.
    Plain,
    /// A hard break around a record line, taken out wherever the parser
    /// does not read it as one.
    Break,
}

/// A paragraph as the message wrote it, line starts normalized (`base`),
/// and the backslashes this pass inserts into it: one before `base[pos]`
/// for each entry of `esc`.
///
/// Planning the escapes as data rather than writing them into a string is
/// what lets [`settle`] take one out again. Each layer that used to edit
/// the string planned on a parse the next layer changed: `<!--` swallowed
/// a backtick until its `--` was escaped, freeing the backtick to open a
/// code span over a record line's escapes, which then showed as
/// backslashes — and each patch for one such shape uncovered another.
#[derive(Default)]
struct Plan {
    base: String,
    esc: BTreeMap<usize, Esc>,
    /// Hyphens and `$`s left unescaped because a bare URL holds them, in
    /// order. `settle` escapes any the reader would not link after all.
    bare: Vec<usize>,
}

impl Plan {
    /// Escape the next character pushed.
    fn escape_next(&mut self) {
        self.esc.insert(self.base.len(), Esc::Keep);
    }

    /// The paragraph with the backslashes `keep` admits, and where each
    /// landed: `(offset in the text, position in base, kind)`.
    fn render(&self, keep: impl Fn(usize) -> bool) -> (String, Vec<(usize, usize, Esc)>) {
        let mut text = String::with_capacity(self.base.len() + self.esc.len());
        let mut at = Vec::with_capacity(self.esc.len());
        let mut last = 0;
        for (&pos, &kind) in self.esc.iter().filter(|&(&pos, _)| keep(pos)) {
            text.push_str(&self.base[last..pos]);
            at.push((text.len(), pos, kind));
            text.push('\\');
            last = pos;
        }
        text.push_str(&self.base[last..]);
        (text, at)
    }

    /// Does a backslash the author wrote escape `base[k]`? It does when it
    /// ends an odd run of backslashes that nothing of ours escapes.
    fn author_escapes(&self, k: usize) -> bool {
        let b = self.base.as_bytes();
        let mut escapes = false;
        let mut j = k;
        while j > 0 && b[j - 1] == b'\\' {
            j -= 1;
            if self.esc.contains_key(&j) {
                // Ours escapes this one, which ends the run that counts.
                break;
            }
            escapes = !escapes;
        }
        escapes && !self.esc.contains_key(&k)
    }

    /// Stop the construct spanning `r` in the rendered text forming: escape
    /// its `<`, `$` or `[`, or both backtick runs of a code span. `false`
    /// when there is nothing left to escape, which `settle` takes as no
    /// progress.
    ///
    /// Both runs, because a span's closer is a backtick run like any other:
    /// with only the opener escaped it opened a span with the NEXT run, and
    /// every later span in the paragraph paired one run off — `a `b` /
    /// `<c>` d `e` f` read as `a `b <c>de` f`.
    fn break_open(&mut self, r: &std::ops::Range<usize>, at: &[(usize, usize, Esc)]) -> bool {
        let base_of = |t: usize| t - at.partition_point(|&(u, ..)| u < t);
        let pos = base_of(r.start);
        let b = self.base.as_bytes();
        match b.get(pos) {
            Some(b'`') => {
                let run = b[pos..].iter().take_while(|&&c| c == b'`').count();
                for k in pos..pos + run {
                    self.esc.insert(k, Esc::Keep);
                }
                // The closer: the backtick run the range ends with, in the
                // rendered text, where an inserted backslash ends a run.
                let mut k = base_of(r.end - 1);
                if r.end - r.start > run && b.get(k) == Some(&b'`') {
                    // A backtick the author escaped is literal once the span
                    // is gone; a backslash put before it would be escaped
                    // by the author's instead, and free it: a span closed
                    // by an escaped-looking backtick read `\\`` broken.
                    while !self.author_escapes(k) {
                        let starts_run = self.esc.contains_key(&k);
                        self.esc.insert(k, Esc::Keep);
                        if starts_run || k == 0 || b[k - 1] != b'`' {
                            break;
                        }
                        k -= 1;
                    }
                }
                true
            }
            Some(b'<' | b'$' | b'[') if self.esc.get(&pos) != Some(&Esc::Keep) => {
                self.esc.insert(pos, Esc::Keep);
                true
            }
            _ => false,
        }
    }
}

/// What the parser makes of a rendered paragraph: the ranges in which a
/// backslash is literal, and where it reads a hard break.
struct Parsed {
    /// Each range starts at its construct's opener. The flag says whether
    /// a hyphen or `$` escape may simply be taken out there.
    literal: Vec<(std::ops::Range<usize>, bool)>,
    /// Where math is. All of it is broken open: a message is shell.
    math: Vec<std::ops::Range<usize>>,
    /// Where raw HTML is, each range opening at its `<`. All of it is
    /// broken open too: a message is not HTML, and the reader strips a
    /// tag, so `Vec<String>` read as `Vec`.
    html: Vec<std::ops::Range<usize>>,
    breaks: std::collections::HashSet<usize>,
    /// The ranges flagged droppable, in order.
    droppable: Vec<std::ops::Range<usize>>,
    /// The bare URLs the reader would link, in order, when asked for.
    linked: Vec<std::ops::Range<usize>>,
}

impl Parsed {
    /// Parsed with the document's own options, math and all — the parse
    /// the reader will make of this text.
    #[allow(clippy::too_many_lines)]
    fn of(text: &str, urls: bool) -> Self {
        use pulldown_cmark::{Event, LinkType, Tag, TagEnd};
        let mut parsed = Parsed {
            literal: Vec::new(),
            math: Vec::new(),
            html: Vec::new(),
            breaks: std::collections::HashSet::new(),
            droppable: Vec::new(),
            linked: Vec::new(),
        };
        let mut events = crate::document::body_events(text).peekable();
        // Inside a link or a code block the reader links no bare URL.
        let mut no_urls = 0usize;
        // The wikilink being read, and how many links are open inside it.
        let mut wiki: Option<(std::ops::Range<usize>, usize)> = None;
        while let Some((event, r)) = events.next() {
            if urls {
                match &event {
                    Event::Start(Tag::Link { .. } | Tag::Image { .. } | Tag::CodeBlock(_)) => {
                        no_urls += 1;
                    }
                    Event::End(TagEnd::Link | TagEnd::Image | TagEnd::CodeBlock) => {
                        no_urls = no_urls.saturating_sub(1);
                    }
                    Event::Text(first) if no_urls == 0 => {
                        // One text run, as `Document` coalesces it, and
                        // the bare URLs it would link there.
                        let mut shown = first.to_string();
                        let mut end = r.end;
                        while let Some((Event::Text(next), at)) = events.peek() {
                            if at.start != end {
                                break;
                            }
                            shown.push_str(next);
                            end = at.end;
                            events.next();
                        }
                        let runs_on = !matches!(
                            events.peek(),
                            None | Some((Event::End(_) | Event::SoftBreak | Event::HardBreak, _))
                        ) && text[end..]
                            .chars()
                            .next()
                            .is_some_and(|c| !c.is_whitespace() && c != '<');
                        // Linked AND shown as typed: a URL the reader
                        // cannot pair with its source (an entity in it)
                        // shows its smart punctuation.
                        let raw = &text[r.start..end];
                        parsed.linked.extend(
                            crate::document::bare_urls(&shown, raw, runs_on)
                                .into_iter()
                                .filter_map(|u| u.typed)
                                .map(|t| r.start + t.start..r.start + t.end),
                        );
                        continue;
                    }
                    _ => {}
                }
            }
            let wikilink = match &event {
                Event::Start(Tag::Link {
                    link_type: LinkType::WikiLink { .. },
                    ..
                }) => Some(r.clone()),
                Event::Start(Tag::Image {
                    link_type: LinkType::WikiLink { .. },
                    ..
                }) if text.as_bytes().get(r.start) == Some(&b'!') => Some(r.start + 1..r.end),
                _ => None,
            };
            match (&event, &mut wiki) {
                (Event::Start(Tag::Link { .. } | Tag::Image { .. }), Some((_, inner))) => {
                    *inner += 1;
                }
                (Event::Start(_), None) => wiki = wikilink.map(|w| (w, 0)),
                (Event::End(TagEnd::Link | TagEnd::Image), Some((_, inner))) => {
                    if *inner == 0 {
                        wiki = None;
                    } else {
                        *inner -= 1;
                    }
                }
                _ => {}
            }
            // Math or HTML in a wikilink's label is undone by breaking the
            // wikilink: the parser reports it offset there (`[[$|f$]]` gave
            // its math as `f$`), so its own opener cannot be found.
            let within = wiki.as_ref().map(|(w, _)| w.clone());
            match event {
                Event::InlineHtml(_) | Event::Html(_) => {
                    if let Some(w) = within {
                        parsed.html.push(w);
                    } else if text.as_bytes().get(r.start) == Some(&b'<') {
                        parsed.html.push(r.clone());
                    }
                    parsed.literal.push((r, true));
                }
                Event::Code(_)
                | Event::Start(Tag::Link {
                    link_type: LinkType::Autolink | LinkType::Email,
                    ..
                }) => parsed.literal.push((r, true)),
                // An embed is the same wikilink behind a `!`, and its
                // opener is the bracket.
                Event::Start(Tag::Image {
                    link_type: LinkType::WikiLink { .. },
                    ..
                }) if text.as_bytes().get(r.start) == Some(&b'!') => {
                    parsed.literal.push((r.start + 1..r.end, false));
                }
                // A wikilink shows its target raw but smart-punctuates a
                // label, so `[[#|---]]` read as a dash with the escape out
                // and kept a backslash with it in. Neither is the message:
                // break it open.
                Event::Start(Tag::Link {
                    link_type: LinkType::WikiLink { .. },
                    ..
                }) => parsed.literal.push((r, false)),
                Event::InlineMath(_) | Event::DisplayMath(_) => {
                    parsed.math.push(within.unwrap_or_else(|| r.clone()));
                    parsed.literal.push((r, true));
                }
                Event::HardBreak => {
                    parsed.breaks.insert(r.start);
                }
                _ => {}
            }
        }
        parsed.droppable = parsed
            .literal
            .iter()
            .filter(|(_, ok)| *ok)
            .map(|(r, _)| r.clone())
            .collect();
        parsed.droppable.sort_by_key(|r| r.start);
        parsed
    }

    /// Every construct that would show one of these backslashes.
    fn showing(
        &self,
        at: &[(usize, usize, Esc)],
        which: impl Fn(Esc) -> bool,
    ) -> Vec<std::ops::Range<usize>> {
        self.literal
            .iter()
            .filter(|(r, plain_ok)| {
                let first = at.partition_point(|&(t, ..)| t <= r.start);
                at[first..]
                    .iter()
                    .take_while(|&&(t, ..)| t < r.end)
                    .any(|&(_, _, kind)| which(kind) || (kind == Esc::Plain && !plain_ok))
            })
            .map(|(r, _)| r.clone())
            .collect()
    }

    /// The construct holding offset `t` in which a hyphen or `$` escape is
    /// taken out. Those never nest (a code span, raw HTML, an autolink and
    /// math hold no other), so the last one starting before `t` is the
    /// only candidate — a binary search, since a paragraph of thousands of
    /// code spans asked this once per escape.
    fn inside(&self, t: usize) -> Option<std::ops::Range<usize>> {
        let i = self.droppable.partition_point(|r| r.start < t);
        let r = self.droppable[..i].last()?;
        (t < r.end).then(|| r.clone())
    }
}

/// Settle `plan` against the parse of its own output, until every
/// backslash it added is read as an escape or a hard break:
///
/// 1. a [`Esc::Keep`] or [`Esc::Break`] inside a code span, raw HTML, an
///    autolink or math — or any escape inside a wikilink — is shown there,
///    so that construct is broken open, its opener escaped, and the
///    paragraph parsed again;
/// 2. a [`Esc::Plain`] inside one is taken out, since neither smart
///    punctuation nor math reaches there, and the result parsed again, to
///    prove taking it out changed no construct;
/// 3. a [`Esc::Break`] the parser does not read as a hard break is taken
///    out.
///
/// Each round that does not finish escapes one more `` ` ``, `<`, `$` or
/// `[` for good, or takes out at least one break, and neither is ever
/// undone, so the rounds are bounded by the count of those. A round is one
/// parse, two when a hyphen or `$` sat in a code span; a paragraph that
/// holds none of the characters that open a construct, and no break, is
/// not parsed. The two give-ups (an opener with nothing left to escape,
/// the bound running out) are not known to be reachable; the property in
/// `hostile_messages` is what says so.
fn settle(mut plan: Plan) -> String {
    let breaks = plan.esc.values().filter(|&&k| k == Esc::Break).count();
    let openers = plan
        .base
        .bytes()
        .filter(|b| matches!(b, b'`' | b'<' | b'$' | b'['))
        .count();
    // With no escape planned there is still raw HTML to find, if any.
    let unescaped = plan.esc.is_empty() && !plan.base.contains('<');
    if plan.bare.is_empty() && (unescaped || (openers == 0 && breaks == 0)) {
        return plan.render(|_| true).0;
    }
    // Break every construct one parse finds at once, not the first: one at
    // a time made a paragraph of N of them N parses of the whole paragraph,
    // and a 50 KB message took seconds.
    let break_all = |plan: &mut Plan, found: &[std::ops::Range<usize>], at: &[_]| {
        let mut progress = false;
        for r in found {
            progress |= plan.break_open(r, at);
        }
        progress
    };
    let bare = plan.bare.len();
    for round in 0..=openers + breaks + bare + FALLBACK_ROUND {
        if round == FALLBACK_ROUND {
            // Still unsettled: a paragraph built to cascade. Escape every
            // opener there is, so no construct can form and only the
            // breaks are left to settle — a bound on the work, at the cost
            // of this paragraph's own code spans and links.
            let b = plan.base.as_bytes();
            let open: Vec<usize> = b
                .iter()
                .enumerate()
                .filter(|&(i, c)| matches!(c, b'`' | b'<' | b'$' | b'[') && !plan.author_escapes(i))
                .map(|(i, _)| i)
                .collect();
            for i in open {
                plan.esc.insert(i, Esc::Keep);
            }
        }
        let (text, at) = plan.render(|_| true);
        let parsed = Parsed::of(&text, false);
        let found = parsed.showing(&at, |k| k != Esc::Plain);
        if !found.is_empty() {
            if break_all(&mut plan, &found, &at) {
                continue;
            }
            return text;
        }
        // Take out the hyphen and `$` escapes a construct holds, each
        // remembered with the construct, and parse what is left.
        let held: Vec<(usize, std::ops::Range<usize>)> = at
            .iter()
            .filter(|&&(_, _, k)| k == Esc::Plain)
            .filter_map(|&(t, pos, _)| parsed.inside(t).map(|r| (pos, r)))
            .collect();
        let (text, at2, parsed2) = if held.is_empty() && plan.bare.is_empty() {
            (text, at.clone(), parsed)
        } else {
            // `held` is in plan order, so a binary search finds a position.
            let (text2, at2) =
                plan.render(|pos| held.binary_search_by_key(&pos, |(p, _)| *p).is_err());
            let parsed2 = Parsed::of(&text2, !plan.bare.is_empty());
            (text2, at2, parsed2)
        };
        let mut found = parsed2.showing(&at2, |_| true);
        found.extend(parsed2.math.iter().cloned());
        found.extend(parsed2.html.iter().cloned());
        if !found.is_empty() {
            if break_all(&mut plan, &found, &at2) {
                continue;
            }
            return text;
        }
        // Taking an escape out unmade the construct that held it: break
        // that construct instead, in the text it formed in.
        let unmade: Vec<std::ops::Range<usize>> = held
            .iter()
            .filter(|(pos, _)| {
                let t = pos + at2.partition_point(|&(_, p, _)| p < *pos);
                parsed2.inside(t).is_none()
            })
            .map(|(_, r)| r.clone())
            .collect();
        if !unmade.is_empty() {
            if break_all(&mut plan, &unmade, &at) {
                continue;
            }
            return text;
        }
        let unread: Vec<usize> = at2
            .iter()
            .filter(|&&(t, _, k)| k == Esc::Break && !parsed2.breaks.contains(&t))
            .map(|&(_, pos, _)| pos)
            .collect();
        if !unread.is_empty() {
            for pos in unread {
                plan.esc.remove(&pos);
            }
            continue;
        }
        if !plan.escape_unlinked(&at2, &parsed2) {
            return text;
        }
    }
    plan.render(|_| true).0
}

impl Plan {
    /// Escape each hyphen or `$` left bare for a URL the reader does not
    /// link after all — cut by markup, inside a link label, in a record
    /// line — like any other. `false` when every one is linked.
    fn escape_unlinked(&mut self, at: &[(usize, usize, Esc)], parsed: &Parsed) -> bool {
        let unlinked: Vec<usize> = self
            .bare
            .iter()
            .copied()
            .filter(|&pos| {
                let t = pos + at.partition_point(|&(_, p, _)| p < pos);
                let i = parsed.linked.partition_point(|r| r.start <= t);
                parsed.linked[..i].last().is_none_or(|r| t >= r.end)
            })
            .collect();
        self.bare.retain(|p| !unlinked.contains(p));
        for &pos in &unlinked {
            self.esc.insert(pos, Esc::Plain);
        }
        !unlinked.is_empty()
    }
}

/// The round of [`settle`] after which a paragraph that has not settled
/// has every opener escaped. Ordinary messages settle in one or two; the
/// cold review's 50 KB cascades in two or three.
const FALLBACK_ROUND: usize = 16;

/// Leading columns of whitespace, a tab reaching the next multiple of four
/// as markdown counts it.
fn indent_cols(l: &str) -> usize {
    let mut col = 0;
    for b in l.bytes() {
        match b {
            b' ' => col += 1,
            b'\t' => col += 4 - col % 4,
            _ => break,
        }
    }
    col
}

/// A record line, shown exactly as written: every inline character
/// escaped ([`escape_inline`]), and the few line starts that leaves able
/// to open a block — a list marker, a setext underline, a definition —
/// escaped too.
fn record_safe(l: &str, plan: &mut Plan) {
    let start = plan.base.len();
    let mut chars = l.chars().peekable();
    while let Some(c) = chars.next() {
        // A dot before another: three unescaped in a row are an ellipsis.
        if is_inline_special(c) || (c == '.' && chars.peek() == Some(&'.')) {
            plan.escape_next();
        }
        plan.base.push(c);
    }
    let digits = l.bytes().take_while(u8::is_ascii_digit).count();
    if l.starts_with(['-', '+', '=', ':']) {
        plan.esc.insert(start, Esc::Keep);
    } else if (1..=9).contains(&digits) && matches!(l.as_bytes().get(digits), Some(b'.' | b')')) {
        plan.esc.insert(start + digits, Esc::Keep);
    }
}

/// Escape hyphens so smart punctuation finds no pair to turn into a dash,
/// and every `$`: a `$` in a commit message is shell (`$$`, `$HOME`), and
/// math is the one inline construct that outranks a code span — `$$` lines
/// around a backtick run swallowed it.
///
/// Planned everywhere, code spans included: [`settle`] takes out the ones
/// a code span, an autolink or raw HTML would show, on the parse of the
/// final text. An escaped hyphen ends a run, so only a hyphen followed by
/// another unescaped one needs it; an escape pair the author wrote is
/// already literal.
fn escape_dashes(plan: &mut Plan) {
    let b = plan.base.as_bytes();
    // A bare URL is found in one unbroken run of text, and an escape cuts
    // the run: `see https://x.example/a--b ok` was no link. The reader
    // shows a URL's hyphens as typed, so they need no escape; a `$` pair in
    // one that reads as math is broken open by `settle`. The scan is over
    // the plain text, so it finds URLs the reader will not link (an
    // `<http://…>` autolink's tail, a record line's): `settle` checks each
    // against the final parse and escapes those.
    let urls = crate::document::extended_autolinks(&plan.base);
    let mut url = urls.iter().map(|(r, _)| r.clone()).peekable();
    let mut add = Vec::new();
    let mut bare = Vec::new();
    let mut after_backslash = false;
    for i in 0..b.len() {
        while url.peek().is_some_and(|r| r.end <= i) {
            url.next();
        }
        let in_url = url.peek().is_some_and(|r| r.start <= i);
        let escaped = after_backslash || plan.esc.contains_key(&i);
        after_backslash = false;
        if escaped {
            continue;
        }
        let wanted = match b[i] {
            b'\\' => {
                after_backslash = true;
                false
            }
            b'$' => true,
            b'-' => b.get(i + 1) == Some(&b'-') && !plan.esc.contains_key(&(i + 1)),
            _ => false,
        };
        if wanted && in_url {
            bare.push(i);
        } else if wanted {
            add.push(i);
        }
    }
    plan.bare = bare;
    for i in add {
        plan.esc.insert(i, Esc::Plain);
    }
}

/// `Author: …`, `AuthorDate: …`, `Merge: …`, `Reflog message: …` — the
/// unindented lines git writes between `commit` and the message.
fn is_header_line(l: &str) -> bool {
    let Some((key, _)) = l.split_once(':') else {
        return false;
    };
    !key.is_empty()
        && key.len() <= 24
        && key.starts_with(|c: char| c.is_ascii_alphabetic())
        && key
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b' ')
}

/// `Signed-off-by: Name <mail>` — git's trailer shape: a token of letters,
/// digits and hyphens, a colon, a space, a value.
fn is_trailer_line(l: &str) -> bool {
    let Some((key, value)) = l.split_once(": ") else {
        return false;
    };
    !value.trim().is_empty()
        && key.starts_with(|c: char| c.is_ascii_alphanumeric())
        && key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
}

/// Does `c` mean something inline? Markup, and the quotes smart
/// punctuation curls.
fn is_inline_special(c: char) -> bool {
    matches!(
        c,
        '\\' | '`'
            | '*'
            | '_'
            | '['
            | ']'
            | '<'
            | '>'
            | '#'
            | '!'
            | '|'
            | '~'
            | '^'
            | '&'
            | '$'
            | '\''
            | '"'
    )
}

/// Backslash-escape every character that means something inline, so a
/// file path reads exactly as git wrote it: `<mail>` keeps its brackets
/// instead of becoming an autolink, `*` and `_` stay literal, and no
/// `--` or `...` in a name becomes a dash or an ellipsis.
fn escape_inline(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if is_inline_special(c) || (matches!(c, '-' | '.') && chars.peek() == Some(&c)) {
            out.push('\\');
        }
        // A lone CR would end the heading line; see `emit_message`.
        out.push(if c == '\r' { ' ' } else { c });
    }
    out
}

/// A message line with its block-level meaning removed, except a list
/// marker, which is kept: a list in a commit body should read as one.
///
/// Every marker the line opens with is peeled, not just the first: in
/// markdown `- - # x` is a heading in a list item in a list item, so
/// neutralizing only what followed the first marker left the `#` free.
/// What is left after the last marker is neutralized, and each marker is
/// written back with one space — whatever whitespace followed it.
fn block_safe(line: &str, plan: &mut Plan) {
    let mut rest = line;
    while let Some(marker_len) = list_marker(rest) {
        let (marker, after) = rest.split_at(marker_len);
        plan.base.push_str(marker.trim_end());
        plan.base.push(' ');
        rest = after.trim_start();
    }
    neutralize(rest, plan);
}

/// The byte length of a leading `-`, `*`, `+`, `1.` or `1)` marker and the
/// whitespace character after it — a space, or a TAB, which markdown takes
/// just the same. A marker that ends the line is left to [`neutralize`]: a
/// lone `-` under a line is a setext underline, not an empty item.
fn list_marker(line: &str) -> Option<usize> {
    let b = line.as_bytes();
    let gap = |at: usize| matches!(b.get(at), Some(b' ' | b'\t'));
    if matches!(b.first(), Some(b'-' | b'*' | b'+')) && gap(1) {
        return Some(2);
    }
    let digits = b.iter().take_while(|c| c.is_ascii_digit()).count();
    ((1..=9).contains(&digits) && matches!(b.get(digits), Some(b'.' | b')')) && gap(digits + 1))
        .then_some(digits + 2)
}

/// Escape the first character of `line` when the line would open a block:
/// a heading, quote, HTML block, fence, table row or definition, a link
/// reference definition (which would vanish), or a run of `-`/`=`/`*`/`_`
/// (a rule, or a setext underline that would turn the line above into a
/// heading). Inline markup at the start — `` `code` ``, `*emphasis*`,
/// `[a link](x)` — is left alone, because it cannot leave its paragraph.
fn neutralize(line: &str, plan: &mut Plan) {
    // A marker with nothing after it is an empty list item, and the marker
    // is gone from the screen: `+`, or `1.` with its delimiter.
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    if (1..=9).contains(&digits) && matches!(&line[digits..], "." | ")") {
        plan.base.push_str(&line[..digits]);
        plan.escape_next();
        plan.base.push_str(&line[digits..]);
        return;
    }
    if !line.is_empty() && line.chars().all(|c| "-+=*_|: \t".contains(c)) {
        // Every glyph, not just the first: `\---` would still read `--` as
        // an en dash, so the line would print as `-–`.
        for c in line.chars() {
            if c.is_ascii_punctuation() {
                plan.escape_next();
            }
            plan.base.push(c);
        }
        return;
    }
    let opens_block = line.starts_with(['#', '>', '<', '|', ':'])
        || line.starts_with("```")
        || line.starts_with("~~~")
        // A reference definition — or the first line of one whose label
        // runs onto the next line, which vanishes all the same.
        || (line.starts_with('[') && (line.contains("]:") || !line.contains(']')));
    if opens_block {
        plan.escape_next();
    }
    plan.base.push_str(line);
}

/// Turn a unified diff — or `git log`/`git show` output — into markdown.
///
/// Commits become `#` headings, files become `##` headings with their line
/// counts, and each file's hunks become one `diff`-tagged fence. Anything that
/// is neither (a commit message, `Author:`, trailing prose) passes through as
/// text — see `emit_message` for how it keeps its lines and its section —
/// which is what makes `git log` readable rather than merely not-broken.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn to_markdown(src: &str) -> String {
    let mut out = String::with_capacity(src.len() + src.len() / 8);
    let mut file = FileDiff::default();
    // Message lines are held back so a run of them can be emitted as one
    // paragraph — emitting per line would make every commit message a
    // stack of one-line paragraphs.
    let mut message: Vec<&str> = Vec::new();
    // Lines the hunk being read still has to come, old side and new. Inside
    // a hunk a line is content whatever it looks like: a removed `-- note`
    // is written `--- note`, which read as a file header and was dropped,
    // and `---i;` was left out of the counts.
    let mut hunk = (0usize, 0usize);

    let flush_message = |message: &mut Vec<&str>, out: &mut String| {
        emit_message(message, out);
        message.clear();
    };

    for line in src.lines() {
        if hunk != (0, 0) {
            let side = match line.as_bytes().first() {
                // Context, which some tools write with its space trimmed.
                Some(b' ') | None => Some((1, 1)),
                Some(b'-') => Some((1, 0)),
                Some(b'+') => Some((0, 1)),
                // `\ No newline at end of file` is a line of neither side.
                Some(b'\\') => Some((0, 0)),
                // The hunk is shorter than it said: read on as outside it.
                Some(_) => None,
            };
            if let Some((old, new)) = side {
                hunk = (hunk.0.saturating_sub(old), hunk.1.saturating_sub(new));
                file.removed += usize::from((old, new) == (1, 0));
                file.added += usize::from((old, new) == (0, 1));
                file.body.push_str(line);
                file.body.push('\n');
                continue;
            }
            hunk = (0, 0);
        }
        // A commit opens a new section and closes whatever came before.
        if is_commit_line(line) {
            std::mem::take(&mut file).flush(&mut out);
            flush_message(&mut message, &mut out);
            let sha = line
                .strip_prefix("commit ")
                .unwrap_or("")
                .split_whitespace()
                .next()
                .unwrap_or("");
            let short = &sha[..sha.len().min(12)];
            let _ = writeln!(out, "# commit {short}\n");
            continue;
        }

        if line.starts_with("diff --git ") {
            std::mem::take(&mut file).flush(&mut out);
            flush_message(&mut message, &mut out);
            file.heading = path_of(line).unwrap_or("(unknown file)").to_string();
            continue;
        }

        // Inside a file: hunks and their content. The `---`/`+++` pair is
        // dropped — the heading already says which file this is, and keeping
        // them would put two red/green lines at the top of every fence that
        // mean nothing to a reader.
        if !file.heading.is_empty() {
            if line.starts_with("--- ") || line.starts_with("+++ ") {
                continue;
            }
            // `index abc..def 100644`, `new file mode`, `similarity index` —
            // plumbing. A reader does not want it; `git show` does.
            if line.starts_with("index ")
                || line.starts_with("new file mode ")
                || line.starts_with("deleted file mode ")
                || line.starts_with("old mode ")
                || line.starts_with("new mode ")
                || line.starts_with("similarity index ")
                || line.starts_with("rename from ")
                || line.starts_with("rename to ")
            {
                continue;
            }
            if let Some(lines) = hunk_lines(line) {
                hunk = lines;
            } else if line.starts_with('+') && !line.starts_with("+++") {
                // Outside any hunk this can count: a combined diff's `@@@`.
                file.added += 1;
            } else if line.starts_with('-') && !line.starts_with("---") {
                file.removed += 1;
            }
            file.body.push_str(line);
            file.body.push('\n');
            continue;
        }

        // A bare diff with no `diff --git` header (plain `diff -u` output):
        // open an unnamed file at the first hunk so the content still lands
        // inside a fence rather than being read as markdown.
        if is_hunk_header(line) || line.starts_with("--- ") || line.starts_with("+++ ") {
            flush_message(&mut message, &mut out);
            file.heading = String::new();
            file.body.push_str(line);
            file.body.push('\n');
            continue;
        }
        if !file.body.is_empty() {
            file.body.push_str(line);
            file.body.push('\n');
            continue;
        }

        message.push(line);
    }

    file.flush(&mut out);
    flush_message(&mut message, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Document;

    const SHOW: &str = "\
commit 4f2a1bcd9e8f7a6b5c4d3e2f1a0b9c8d7e6f5a4b
Author: Someone <s@example.com>
Date:   Thu Aug 21 09:00:00 2026 -0400

    Anchor the selection to its path

diff --git a/crates/carrel/src/home.rs b/crates/carrel/src/home.rs
index 1111111..2222222 100644
--- a/crates/carrel/src/home.rs
+++ b/crates/carrel/src/home.rs
@@ -54,3 +54,4 @@
     let width = clamp(cols, 10, 60);
-    let height = old(entries);
+    let full = min(PICKER_ROWS + 3, rows);
+    let height = min(wanted + 3, full);
     let x = centre(cols, width);
";

    #[test]
    fn a_git_show_capture_becomes_sections_and_a_fence() {
        assert!(looks_like_diff(SHOW));
        let md = to_markdown(SHOW);
        assert!(md.contains("# commit 4f2a1bcd9e8f"), "{md}");
        assert!(
            md.contains("## crates/carrel/src/home.rs  +2 −1"),
            "heading with counts: {md}"
        );
        assert!(md.contains("```diff"), "{md}");
        // The plumbing is gone; the hunk header and its content are not.
        assert!(!md.contains("index 1111111"), "{md}");
        assert!(!md.contains("--- a/crates"), "{md}");
        assert!(md.contains("@@ -54,3 +54,4 @@"), "{md}");
        assert!(md.contains("+    let full = min("), "{md}");
        // The commit message survives as prose.
        assert!(md.contains("Anchor the selection to its path"), "{md}");
    }

    #[test]
    fn the_parsed_result_has_the_structure_the_reader_needs() {
        let doc = Document::parse(&to_markdown(SHOW));
        let headings: Vec<_> = doc
            .nodes
            .iter()
            .filter(|n| matches!(n.kind, crate::NodeKind::Heading { .. }))
            .map(|n| doc.text[n.doc.start as usize..n.doc.end as usize].to_string())
            .collect();
        assert_eq!(headings.len(), 2, "a commit and a file: {headings:?}");
        // The hunk is a code block tagged `diff`, which is what earns it
        // syntect's Diff syntax.
        assert!(
            doc.nodes.iter().any(|n| matches!(
                &n.kind,
                crate::NodeKind::CodeBlock { lang: Some(l) } if &**l == "diff"
            )),
            "no diff-tagged code block"
        );
    }

    #[test]
    fn a_diff_of_a_markdown_file_does_not_break_out_of_its_fence() {
        let src = "\
diff --git a/README.md b/README.md
--- a/README.md
+++ b/README.md
@@ -1,3 +1,3 @@
-```bash
+```sh
 carrel --version
-```
+```
";
        let md = to_markdown(src);
        let doc = Document::parse(&md);
        // One code block, and it holds the inner fences as text.
        let code: Vec<_> = doc
            .nodes
            .iter()
            .filter(|n| matches!(n.kind, crate::NodeKind::CodeBlock { .. }))
            .collect();
        assert_eq!(code.len(), 1, "the inner fence broke out: {md}");
        assert!(doc.text.contains("```sh"), "{:?}", doc.text);
    }

    #[test]
    fn git_log_with_no_hunks_still_becomes_sections() {
        let src = "\
commit aaaaaaabbbbbbbcccccccdddddddeeeeeeefffffff
Author: A <a@example.com>

    First thing

commit 1111111222222233333334444444555555566666666
Author: B <b@example.com>

    Second thing
";
        assert!(looks_like_diff(src));
        let md = to_markdown(src);
        assert!(md.contains("# commit aaaaaaabbbbb"), "{md}");
        assert!(md.contains("# commit 111111122222"), "{md}");
        assert!(md.contains("First thing"), "{md}");
        assert!(!md.contains("```"), "no hunks, so no fence: {md}");
    }

    #[test]
    fn a_bare_unified_diff_with_no_git_header_still_lands_in_a_fence() {
        let src = "--- old.txt\n+++ new.txt\n@@ -1 +1 @@\n-a\n+b\n";
        assert!(looks_like_diff(src));
        let doc = Document::parse(&to_markdown(src));
        assert!(
            doc.nodes
                .iter()
                .any(|n| matches!(n.kind, crate::NodeKind::CodeBlock { .. })),
            "{:?}",
            doc.text
        );
    }

    /// The whole point of the `diff` tag: syntect classifies the LINE, not
    /// just the sigil. Before the container-pass entry, `-gone` gave one
    /// `Punctuation` token for `-` and nothing for `gone`.
    #[test]
    fn a_diff_fence_classifies_whole_lines_not_just_the_sigils() {
        use crate::TokenKind;
        let doc = Document::parse("```diff\n@@ -1,2 +1,2 @@\n ctx\n-gone\n+added\n```\n");
        let block = doc
            .layout_order
            .iter()
            .position(|_| true)
            .map(|i| crate::BlockIdx(u32::try_from(i).unwrap()))
            .unwrap();
        let toks = doc.tokens(block);
        let text_of = |k: TokenKind| -> Vec<&str> {
            toks.iter()
                .filter(|t| {
                    t.kind == k
                        || matches!(
                            (k, t.kind),
                            (TokenKind::Inserted, TokenKind::InsertedWord)
                                | (TokenKind::Deleted, TokenKind::DeletedWord)
                        )
                })
                .map(|t| &doc.text[t.doc.start as usize..t.doc.end as usize])
                .collect()
        };
        assert!(
            text_of(TokenKind::Deleted)
                .iter()
                .any(|s| s.contains("gone")),
            "the deleted LINE must classify, not just `-`: {toks:?}"
        );
        assert!(
            text_of(TokenKind::Inserted)
                .iter()
                .any(|s| s.contains("added")),
            "the inserted LINE must classify: {toks:?}"
        );
        // The `@@` sigils themselves stay `Punctuation` (they match the
        // generic prefix); `Meta` is the range between them, which is the
        // part carrying information.
        assert!(
            text_of(TokenKind::Meta).iter().any(|s| s.contains("1,2")),
            "the hunk range is chrome: {toks:?}"
        );
    }

    #[test]
    fn ordinary_prose_is_not_a_diff() {
        for s in [
            "# A document\n\nAbout diffs, even.\n",
            "Use `diff --git` to see it.\n", // inline code, not a line start
            "commit to the plan\n",          // not an object name
            "@@ not a hunk header @@\n",
            "",
        ] {
            assert!(!looks_like_diff(s), "false positive: {s:?}");
        }
    }

    #[test]
    fn crlf_input_survives() {
        let src = SHOW.replace('\n', "\r\n");
        assert!(looks_like_diff(&src));
        let md = to_markdown(&src);
        assert!(md.contains("## crates/carrel/src/home.rs"), "{md}");
    }

    /// `md.contains("## ")` was the whole assertion here, which passed while
    /// the heading read `## dev/null` — a heading is worthless unless it
    /// names the file. Git spells a deletion out in `deleted file mode` and
    /// `+++ /dev/null`; the `diff --git` line still carries the real path on
    /// both sides, so that is what the reader must see.
    #[test]
    fn a_deletion_names_the_file_that_is_going_away() {
        let src = "diff --git a/gone.rs b/gone.rs\ndeleted file mode 100644\n\
                   --- a/gone.rs\n+++ /dev/null\n@@ -1 +0,0 @@\n-x\n";
        let md = to_markdown(src);
        assert!(md.contains("## gone.rs"), "{md}");
    }

    /// Git does not quote a path just because it has a space in it, so the
    /// separator is the ` b/`, not the first space. Splitting at the space
    /// produced the heading `## notes.md b/my notes.md` — and a garbled
    /// heading is a garbled section index, breadcrumb and outline.
    #[test]
    fn a_path_with_spaces_survives_the_heading() {
        let src = "diff --git a/my notes.md b/my notes.md\n@@ -1 +1 @@\n-a\n+b\n";
        let md = to_markdown(src);
        assert!(md.contains("## my notes.md  +1 −1"), "{md}");
        // A rename takes the NEW name, which is the file as it now is.
        let src = "diff --git a/old name.md b/new name.md\nrename from old name.md\n\
                   rename to new name.md\n@@ -1 +1 @@\n-a\n+b\n";
        let md = to_markdown(src);
        assert!(md.contains("## new name.md"), "{md}");
    }

    const LOG: &str = "\
commit 900449309c16739ea37977c6c0a01271423956f5
Merge: dd63473 b20c3db
Author: Some One <s@example.com>
Date:   Thu Oct 8 12:54:38 2026 -0400

    Anchor the selection to its path

    The list re-sorts while a scan streams, so a bare
    index drifts onto a different file.

    Co-Authored-By: Claude <noreply@example.com>
    Signed-off-by: Some One <s@example.com>

commit 15d032117b441b4f9c4c6933af5ed2c6e6a80738
Author:     Some One <s@example.com>
AuthorDate: Thu Oct 8 16:38:47 2026 -0400
Commit:     Some One <s@example.com>
CommitDate: Thu Oct 8 16:38:47 2026 -0400

    fix: a subject in the
    shape: of a trailer
";

    /// `git log` headers and trailers are records, one per line. They were
    /// joined into a single wrapped paragraph — `Merge: … Author: … Date: …`
    /// — while the message body, which IS prose, should still reflow.
    #[test]
    fn commit_headers_and_trailers_keep_their_own_lines() {
        let doc = Document::parse(&to_markdown(LOG));
        let t = &doc.text;
        assert!(
            t.contains("Merge: dd63473 b20c3db\nAuthor: Some One <s@example.com>\nDate:"),
            "headers run together: {t:?}"
        );
        assert!(
            t.contains("Author:     Some One <s@example.com>\nAuthorDate:"),
            "--format=fuller headers run together: {t:?}"
        );
        assert!(
            t.contains("Co-Authored-By: Claude <noreply@example.com>\nSigned-off-by:"),
            "trailers run together: {t:?}"
        );
        assert!(
            t.contains("so a bare index drifts"),
            "the body must still reflow: {t:?}"
        );
        // A paragraph shaped like trailers is not one when it is the subject:
        // a conventional-commit subject wrapped over two lines still reflows.
        assert!(t.contains("fix: a subject in the shape: of"), "{t:?}");
    }

    /// Ordered list markers can continue a paragraph or open a new item.
    /// Backtick spans must follow the parser's decision in both cases.
    #[test]
    fn message_code_spans_use_the_parsers_actual_list_boundaries() {
        for message in [
            "`\n2)\t`--flag`",
            "prefix `\n2) `--flag`",
            "- `\n2) `--flag`",
            "1. `\n2. `--flag`",
            "`\n1) `--flag`",
            "`\n* `--flag`",
            "`\n2) ``--flag``",
        ] {
            let mut lines = String::new();
            for line in message.lines() {
                writeln!(lines, "    {line}").unwrap();
            }
            let src = format!("commit abcdef0123456789\nAuthor: A\n\n{lines}");
            let markdown = to_markdown(&src);
            let doc = Document::parse(&markdown);
            assert!(
                doc.text.contains("--flag"),
                "{message:?}: {:?}\n{markdown}",
                doc.text
            );
            assert!(
                !doc.text.contains(['\u{2013}', '\u{2014}']),
                "{message:?}: {:?}",
                doc.text
            );
            assert!(
                !doc.text.contains("\\--flag"),
                "an escape leaked into code: {:?}",
                doc.text
            );
        }
        let doc = Document::parse(&to_markdown(&two_commits(
            "URL <https://a--b.test/$HOME> and `--flag`",
        )));
        assert!(
            doc.links
                .iter()
                .any(|url| &**url == "https://a--b.test/$HOME")
        );
        assert!(doc.text.contains("--flag"));
    }

    /// A trailer is a record even with no blank line above it — after body
    /// text, or after an indented line — and keeps its `<mail>`. An indented
    /// line and a `#` comment line (a merge's `# Conflicts:` list) keep
    /// their own lines. And `--` in a message is two hyphens: a flag, an
    /// HTML comment, never an en dash.
    /// `$` in a message is literal: no math forms, `$$` reads as written,
    /// and a backtick run that math would have swallowed still closes its
    /// code span — so the `--` inside stays two hyphens. Found by
    /// `only_the_headings_it_wrote` in CI.
    #[test]
    fn a_dollar_in_a_message_is_shell_not_math() {
        let src = "commit 0abcdef0123456789\nAuthor: A\n\n    $$\n    ```$$\n    <!--``\n\n    kill $$ and echo $HOME$\n";
        let md = to_markdown(src);
        let doc = Document::parse(&md);
        assert!(
            !doc.text.contains(['\u{2013}', '\u{2014}']),
            "{:?}\n{md}",
            doc.text
        );
        assert!(doc.text.contains("<!--"), "{:?}\n{md}", doc.text);
        assert!(
            doc.text.contains("kill $$ and echo $HOME$"),
            "{:?}\n{md}",
            doc.text
        );
        assert!(
            !doc.nodes
                .iter()
                .any(|n| format!("{:?}", n.kind).starts_with("Math")),
            "{md}"
        );
    }

    #[test]
    fn message_records_keep_their_lines_and_hyphens_stay_hyphens() {
        let src = "\
commit aaaaaaabbbbbbbcccccccdddddddeeeeeeefffffff
Author: A <a@example.com>

    Subject line

    Body text here
    > quote
        indented code
    Signed-off-by: X <x@y>

commit 1111111222222233333334444444555555566666666
Merge: dd63473 b20c3db
Author: B <b@example.com>

    Merge branch 'topic'

    # Conflicts:
    #\tsrc/a.rs
    #\tsrc/b.rs

commit 2222222333333344444445555555666666677777777
Author: C <c@example.com>

    Use --force <!-- note --> and `--keep` here
    still -- prose, a---b
";
        let md = to_markdown(src);
        let doc = Document::parse(&md);
        let t = &doc.text;
        for want in [
            "Body text here > quote\nindented code\nSigned-off-by: X <x@y>",
            "# Conflicts:\n#",
            "src/a.rs\n#",
            "Use --force <!-- note --> and --keep here still -- prose, a---b",
        ] {
            assert!(t.contains(want), "{want:?} not in {t:?}\n{md}");
        }
        let (headings, structure) = stolen_structure(src);
        assert_eq!(headings.len(), 3, "{headings:?}");
        assert!(structure.is_empty(), "{structure:?}");
    }

    /// A commit message is someone else's text. Whatever markdown it holds
    /// must stay inside its commit: a `#` line is not a section, a `---`
    /// under a line is not a setext heading, a fence or `<pre>` must not
    /// swallow the next commit, a link definition must not vanish.
    #[test]
    fn a_hostile_commit_message_cannot_break_its_commit_section() {
        let src = "\
commit aaaaaaabbbbbbbcccccccdddddddeeeeeeefffffff
Author: A <a@example.com>

    # not a heading

    Title-ish line
    ===

    another
    ---

    ```
    <pre>
    > quoted
    [ref]: http://example.com/x
    : a definition
    | a | b |
    |---|---|
    *emphasis* and `code` and [link](x) stay inline
    - a list item stays a list
    - # with a hash inside

commit 1111111222222233333334444444555555566666666
Author: B <b@example.com>

    Second
";
        let md = to_markdown(src);
        let doc = Document::parse(&md);
        let headings: Vec<_> = doc
            .nodes
            .iter()
            .filter(|n| matches!(n.kind, crate::NodeKind::Heading { .. }))
            .map(|n| doc.text[n.doc.start as usize..n.doc.end as usize].to_string())
            .collect();
        assert_eq!(
            headings,
            ["commit aaaaaaabbbbb", "commit 111111122222"],
            "a message line became structure: {md}"
        );
        for kind in [
            "CodeBlock",
            "BlockQuote",
            "Table",
            "Rule",
            "DefTerm",
            "DefDetails",
            "Math",
        ] {
            assert!(
                !doc.nodes
                    .iter()
                    .any(|n| format!("{:?}", n.kind).starts_with(kind)),
                "a message line became a {kind}: {md}"
            );
        }
        for literal in [
            "# not a heading",
            "===",
            "```",
            "<pre>",
            "> quoted",
            "[ref]: http://example.com/x",
            ": a definition",
            "| a | b |",
            // Escaped glyph by glyph, or smart punctuation turns the run
            // it no longer reads as a rule into an em dash.
            "another ---",
            "|---|---|",
        ] {
            assert!(
                doc.text.contains(literal),
                "{literal:?} lost: {:?}",
                doc.text
            );
        }
        // What a commit message legitimately uses still works.
        assert!(
            doc.text.contains("emphasis and code and link stay"),
            "{:?}",
            doc.text
        );
        assert!(
            doc.nodes
                .iter()
                .any(|n| matches!(n.kind, crate::NodeKind::Item)),
            "a list is a list: {md}"
        );
        assert!(doc.text.contains("# with a hash inside"), "{:?}", doc.text);
    }

    /// The headings `to_markdown` wrote for `src`'s commits and files, and
    /// every structural node of the parse it would not have: what a commit
    /// message managed to turn into structure.
    fn stolen_structure(src: &str) -> (Vec<String>, Vec<String>) {
        let md = to_markdown(src);
        let doc = Document::parse(&md);
        let headings = doc
            .nodes
            .iter()
            .filter(|n| matches!(n.kind, crate::NodeKind::Heading { .. }))
            .map(|n| doc.text[n.doc.start as usize..n.doc.end as usize].to_string())
            .collect();
        let structure = doc
            .nodes
            .iter()
            .filter(|n| match &n.kind {
                crate::NodeKind::CodeBlock { lang } => lang.as_deref() != Some("diff"),
                crate::NodeKind::BlockQuote
                | crate::NodeKind::Table { .. }
                | crate::NodeKind::Rule
                | crate::NodeKind::DefTerm
                | crate::NodeKind::DefDetails
                | crate::NodeKind::Metadata { .. }
                | crate::NodeKind::AlertLabel { .. } => true,
                _ => false,
            })
            .map(|n| format!("{:?}", n.kind))
            .collect();
        (headings, structure)
    }

    /// `commit <sha>`, a message, and the commit after it.
    fn two_commits(message_line: &str) -> String {
        format!(
            "commit aaaaaaabbbbbbbcccccccdddddddeeeeeeefffffff\n\
             Author: A <a@example.com>\n\n    Subject\n\n    {message_line}\n    more\n\n\
             commit 1111111222222233333334444444555555566666666\n\
             Author: B <b@example.com>\n\n    Second\n"
        )
    }

    /// A list marker is a marker whatever whitespace follows it — a tab
    /// included — and a list item can open another. The marker check
    /// wanted exactly one space, so `-\t# x` passed through whole: a
    /// level-1 heading inside the commit, a fake commit in the outline, the
    /// breadcrumb and the fold. And it peeled one marker, so `- - # x`
    /// escaped the `-` and left the `#` to open a heading in the nested
    /// item. A fence or a rule got through the same way.
    #[test]
    fn a_marker_followed_by_a_tab_or_another_marker_cannot_open_a_block() {
        for line in [
            "-\t# stolen heading",
            "1.\t# stolen heading",
            "- \t# stolen",
            "- - # nested",
            "- 1. # nested",
            "* + 2) # deeper",
            "*\t```",
            "+\t---",
            "-\t> quoted",
            "-\t<pre>",
            "1)\t~~~",
            "- - ===",
            "-\t[ref]: http://example.com/x",
            "- [x",
        ] {
            let (headings, structure) = stolen_structure(&two_commits(line));
            assert_eq!(
                headings,
                ["commit aaaaaaabbbbb", "commit 111111122222"],
                "{line:?} became a heading"
            );
            assert!(structure.is_empty(), "{line:?} became {structure:?}");
        }
        // The text survives, and a list is still a list.
        let doc = Document::parse(&to_markdown(&two_commits("-\t# kept")));
        assert!(doc.text.contains("# kept"), "{:?}", doc.text);
        assert!(
            doc.nodes
                .iter()
                .any(|n| matches!(n.kind, crate::NodeKind::Item))
        );
    }

    /// A link reference definition's label may run across lines, and the
    /// whole definition then vanishes. A line that opens a bracket it does
    /// not close is the first line of one.
    #[test]
    fn a_link_definition_split_across_lines_does_not_vanish() {
        let src = "commit aaaaaaabbbbbbbcccccccdddddddeeeeeeefffffff\n\
                   Author: A <a@example.com>\n\n    Subject\n\n    [split\n    label]: /url\n";
        let doc = Document::parse(&to_markdown(src));
        assert!(doc.text.contains("label"), "{:?}", doc.text);
    }

    /// Commit messages built from hostile fragments, line by line: the only
    /// headings in the result are the ones `to_markdown` wrote, and no message
    /// text becomes a fence, a rule, a table, a quote, a definition, an alert
    /// or frontmatter.
    mod hostile_messages {
        use super::*;
        use proptest::prelude::*;

        const FRAGMENTS: &[&str] = &[
            "#",
            "# ",
            "##",
            "#tag",
            "-",
            "- ",
            "-\t",
            "*",
            "* ",
            "+",
            "+ ",
            "1.",
            "1. ",
            "2)",
            "1.\t",
            "\t",
            " ",
            "  ",
            ">",
            "> ",
            "<pre>",
            "</pre>",
            "<div>",
            "<!--",
            "-->",
            "<details>",
            "</details>",
            "<summary>s</summary>",
            "```",
            "~~~",
            "---",
            "===",
            "***",
            "___",
            "- - -",
            "|",
            "| a | b |",
            "|---|---|",
            ":",
            ": ",
            "[x]:",
            "[^a]:",
            "[x",
            "]: /u",
            "[!NOTE]",
            "$$",
            "\\",
            "- [ ] ",
            "&#35;",
            "&gt;",
            "<",
            "x",
            "word",
            "\u{a0}",
            "\u{3000}",
            "    ",
            "commit",
            "diff --git",
            "@@ -1 +1 @@",
            "`",
            "+++ ",
            "--- ",
            "Signed-off-by: x",
            "Author: y",
            "=",
            "-=-",
            "[word",
            "word]: /u",
            "Signed-off-by: x <y@z>",
            "Co-Authored-By: w",
            "# Conflicts:",
            "#\t",
            "--force",
            "--",
            "`--x`",
            "``",
            "<http://a--b>",
            "<a@b>",
            "\\-",
            "        ",
            "<b>",
            "</b>",
            "<kbd>",
            "<a x=\"--`\">",
            "<!DOCTYPE",
            "<?",
            "]]>",
            "<![CDATA[",
            "<a",
            "x=\"",
            "\">",
            "$x",
            "x$",
            "<https://a",
            "[[",
            "]]",
            "](",
            ")",
            "<script>",
            "</script>",
            "<!-->",
            "https://a",
            " www.a.b/",
            "\r",
            "[[x|]]",
            "![[",
            "!",
            "<sha>",
            "Vec<word>",
            "<word/>",
            "<Word x=word>",
            "<!-- word -->",
            "</",
            "^",
            "x^2^",
            "~",
            "...",
            "\"",
            "'",
            "http://a",
            "$|f$",
            "|",
        ];

        fn line() -> impl Strategy<Value = String> {
            prop::collection::vec(prop::sample::select(FRAGMENTS), 0..5)
                .prop_map(|parts| parts.concat())
        }

        /// One commit: its header, then message lines indented as git
        /// indents them, and maybe a file.
        fn commit() -> impl Strategy<Value = (Vec<String>, bool)> {
            (prop::collection::vec(line(), 1..8), any::<bool>())
        }

        /// A backtick span across a record line made its escapes literal.
        #[test]
        fn a_record_line_inside_a_backtick_span_shows_no_backslashes() {
            for mid in ["#<!--", "# Conflicts:", "#\tsrc/a.rs"] {
                let src = format!(
                    "commit 0abcdef0123456789abcdef0123456789abcdef\nAuthor: A (a at example.com)\n\n    a `b\n    {mid}\n    `<!--`\n\n"
                );
                let doc = Document::parse(&to_markdown(&src));
                assert!(
                    !doc.text.contains('\\'),
                    "{mid}: {:?}\n{}",
                    doc.text,
                    to_markdown(&src)
                );
                assert!(
                    doc.text.contains('`'),
                    "the unpaired backtick shows: {:?}",
                    doc.text
                );
            }
        }

        /// The hard break after a record line is not added before a list
        /// item, which ends the paragraph: the break showed as `\\`.
        #[test]
        fn no_hard_break_shows_before_a_list_item() {
            let src = "commit 0abcdef0123456789abcdef0123456789abcdef\nAuthor: A (a at example.com)\n\n    \t#\n    1.\t#\n\n";
            let doc = Document::parse(&to_markdown(src));
            assert!(
                !doc.text.contains('\\'),
                "{:?}\n{}",
                doc.text,
                to_markdown(src)
            );
        }

        /// A break the parser does not read as one is dropped, wherever
        /// the message's own markdown ended the paragraph first.
        #[test]
        fn no_hard_break_shows_where_the_message_ends_a_paragraph() {
            let src = "commit 0abcdef0123456789abcdef0123456789abcdef\nAuthor: A (a at example.com)\n\n    - #\n            #\n    +\n\n";
            let doc = Document::parse(&to_markdown(src));
            assert!(
                !doc.text.contains('\\'),
                "{:?}\n{}",
                doc.text,
                to_markdown(src)
            );
        }

        /// No escape may open a block the message does not have. (The
        /// layered pass planned with a stand-in for `<!--`, and a space
        /// there made `*<!--` a list item in the plan alone.)
        #[test]
        fn the_plan_never_opens_a_block_the_message_does_not_have() {
            for mid in ["x<!--", "*<!--", "-<!--", "+<!--", "1.<!--"] {
                let src = format!(
                    "commit 0abcdef0123456789abcdef0123456789abcdef\nAuthor: A (a at example.com)\n\n    a `b\n    {mid}\n    `<!--`\n\n"
                );
                let doc = Document::parse(&to_markdown(&src));
                assert!(
                    !doc.text.contains(['\u{2013}', '\u{2014}']),
                    "{mid}: {:?}",
                    doc.text
                );
            }
        }

        /// Shrunk by the property below (2026-11-14): `<!--` swallowed the
        /// next line's backtick in the plan, and escaping it undid that.
        #[test]
        fn an_html_comment_opener_does_not_turn_a_later_code_span_into_a_dash() {
            let src = "commit 0abcdef0123456789abcdef0123456789abcdef\nAuthor: A (a at example.com)\n\n    -<!--\n    `-->\n    `--x`\n\n";
            let doc = Document::parse(&to_markdown(src));
            assert!(
                !doc.text.contains(['\u{2013}', '\u{2014}']),
                "{:?}",
                doc.text
            );
            let (headings, structure) = stolen_structure(src);
            assert_eq!(headings, ["commit 0abcdef01234"]);
            assert!(structure.is_empty(), "{structure:?}");
        }

        /// Shapes where one escaping layer planned on a parse the next one
        /// changed, each found by the property below once it asserted "no
        /// backslash the author did not write" (2026-11-14). The first:
        /// `<!--` swallowed a backtick until its `--` was escaped, and the
        /// freed backtick opened a code span over escapes. The second and
        /// third: a wikilink shows its target raw, and smart-punctuates its
        /// label. The last: an inline HTML attribute's backtick paired
        /// with one on the line above once the comment was broken.
        ///
        /// Round two (a cold review of the settling pass): a span closed
        /// by a backtick the author escaped, which a backslash of ours in
        /// front of would free; a bare URL left unescaped that the reader
        /// then did not link (an empty-label wikilink after it joined its
        /// text run); an empty-label wikilink, which the reader shows as
        /// typed, holding a line-start escape.
        #[test]
        fn every_added_backslash_is_read_as_an_escape() {
            for (message, keeps) in [
                ("><!--\n```\n>\n``-->``", "-->"),
                ("<![CDATA[[x]:\n>\n]]>", "[x]:"),
                ("[[#|---|---|]]>", "---|---|"),
                ("1.<!--\n`-->\n\t<a x=\"--`\">", "--`"),
                ("<a x=\"--`\">\n>\\`--x`\n>\n`--x`", "--x"),
                ("<http://a--b>2)[[x|]]-->", "a--b"),
                ("-[[\n|]]>", "|]]"),
            ] {
                let mut src =
                    String::from("commit 0abcdef0123456789abcdef0123456789abcdef\nAuthor: A\n\n");
                for l in message.lines() {
                    let _ = writeln!(src, "    {l}");
                }
                let md = to_markdown(&src);
                let doc = Document::parse(&md);
                assert!(
                    !doc.text.contains(['\\', '\u{2013}', '\u{2014}']),
                    "{message:?}: {:?}\n{md}",
                    doc.text
                );
                assert!(doc.text.contains(keeps), "{message:?}: {:?}", doc.text);
            }
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(2000))]
            #[test]
            fn only_the_headings_it_wrote(commits in prop::collection::vec(commit(), 1..4)) {
                let mut src = String::new();
                let mut want = Vec::new();
                for (i, (lines, file)) in commits.iter().enumerate() {
                    let sha = format!("{i}abcdef0123456789abcdef0123456789abcdef");
                    let _ = write!(src, "commit {sha}\nAuthor: A (a at example.com)\n\n");
                    want.push(format!("commit {}", &sha[..12]));
                    for l in lines {
                        if l.trim().is_empty() {
                            src.push('\n');
                        } else {
                            let _ = writeln!(src, "    {l}");
                        }
                    }
                    src.push('\n');
                    if *file {
                        src.push_str("diff --git a/f b/f\n@@ -1 +1 @@\n-a\n+b\n");
                        want.push("f  +1 \u{2212}1".to_string());
                    }
                }
                let (headings, structure) = stolen_structure(&src);
                prop_assert_eq!(&headings, &want, "{}", to_markdown(&src));
                prop_assert!(structure.is_empty(), "{:?}\n{}", structure, to_markdown(&src));
                // No hyphen pair in a message becomes a dash.
                let doc = Document::parse(&to_markdown(&src));
                prop_assert!(
                    !doc.text.contains(['\u{2013}', '\u{2014}']),
                    "{:?}\n{}", doc.text, to_markdown(&src)
                );
                // Every escape and hard break this pass adds is consumed by
                // the parser: a backslash on screen is one the author wrote.
                prop_assert!(
                    doc.text.matches('\\').count() <= src.matches('\\').count(),
                    "{:?}\n{}", doc.text, to_markdown(&src)
                );
                // And no message text vanishes: not into a link reference
                // definition, and not into a tag the reader would strip —
                // a message is not HTML, and its `<sha>` is a word. (A link
                // hides its destination and a wikilink its target: the
                // author's own markup, which stays in its paragraph.)
                let hides = src.contains("](") || src.contains("[[");
                for word in ["word", "sha"].into_iter().filter(|_| !hides) {
                    prop_assert_eq!(
                        doc.text.matches(word).count(),
                        src.matches(word).count(),
                        "{:?}\n{}", doc.text, to_markdown(&src)
                    );
                }
            }
        }
    }

    /// A path is a heading's text, and the heading is what the outline and
    /// breadcrumb show: it must read as the file is named.
    #[test]
    fn a_file_heading_shows_its_path_literally() {
        for path in ["wip *draft* <pre>.md", "notes #", "a_b_ `c`.md"] {
            let src = format!("diff --git a/{path} b/{path}\n@@ -1 +1 @@\n-a\n+b\n");
            let doc = Document::parse(&to_markdown(&src));
            let heading = doc
                .nodes
                .iter()
                .find(|n| matches!(n.kind, crate::NodeKind::Heading { .. }))
                .map(|n| &doc.text[n.doc.start as usize..n.doc.end as usize])
                .unwrap();
            assert_eq!(heading, format!("{path}  +1 −1"));
        }
    }

    #[test]
    fn an_empty_diff_produces_nothing_alarming() {
        assert_eq!(to_markdown("").trim(), "");
        assert!(!looks_like_diff(""));
    }
}
