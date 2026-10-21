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

use std::fmt::Write as _;

/// How far in to look for a diff marker before giving up.
const SNIFF_LINES: usize = 40;

/// Does this look like a unified diff, or `git log`/`git show` output?
///
/// **Narrow on purpose, and never applied to a `.md` file.** The caller
/// restricts this to piped input and to files named `.diff`/`.patch`; that
/// single rule removes the whole class of "my markdown document *about* diffs
/// got mangled" bugs. What is left is a strong marker in the first
/// [`SNIFF_LINES`] lines.
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
/// punctuation's dashes do not ([`keep_hyphens`]): `--` in a commit
/// message is a flag or an HTML comment, not an en dash.
fn emit_message(message: &[&str], out: &mut String) {
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
        if i == 0 && header {
            let lines: Vec<String> = p.iter().map(|l| record_safe(l.trim())).collect();
            out.push_str(&lines.join("\\\n"));
            out.push_str("\n\n");
            continue;
        }
        // The trailer run: the last paragraph's closing trailer-shaped
        // lines. Never in the subject's paragraph, whose own first line
        // has the shape whenever it is a conventional-commit subject.
        let trailers = if i + 1 == paras.len() && i > body {
            p.len()
                - p.iter()
                    .rev()
                    .take_while(|l| is_trailer_line(l.trim()))
                    .count()
        } else {
            p.len()
        };
        let record = |j: usize| {
            let l: &str = p[j];
            j >= trailers || indent_cols(l) >= base + 4 || l.trim_start().starts_with('#')
        };
        // Hyphens are kept over each run of lines that parses as ONE inline
        // context, not line by line: a code span may open on one line and
        // close on the next. A list marker opens a new item, and with it a
        // new context; any other line continues the one before it.
        let mut para = String::new();
        for (j, line) in p.iter().enumerate() {
            let shown = if record(j) {
                record_safe(line.trim())
            } else {
                block_safe(line.trim()).into_owned()
            };
            if list_marker(&shown).is_some() {
                out.push_str(&keep_hyphens(&para));
                para.clear();
            }
            para.push_str(&shown);
            // A hard break around a record — unless the line already ends
            // in one: an odd run of backslashes is the author's own break,
            // and one more would escape it into a literal backslash.
            let own_break = shown.bytes().rev().take_while(|&b| b == b'\\').count() % 2 == 1;
            if j + 1 < p.len() && (record(j) || record(j + 1)) && !own_break {
                para.push('\\');
            }
            para.push('\n');
        }
        out.push_str(&keep_hyphens(&para));
        out.push('\n');
    }
}

/// What sits between `<` and `>` in an autolink: a URI (a scheme of two to
/// 32 letters, digits, `+`, `.` or `-` opening with a letter, then `:`) or
/// a mail address — near enough to `CommonMark`'s rule that a hyphen pair
/// inside one is left alone and nothing else is.
fn is_autolink(t: &str) -> bool {
    if let Some((scheme, _)) = t.split_once(':') {
        return (2..=32).contains(&scheme.len())
            && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'.' | b'-'));
    }
    t.split_once('@').is_some_and(|(user, host)| {
        !user.is_empty() && host.contains(|c: char| c.is_alphanumeric())
    })
}

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
/// escaped ([`escape_inline`], which leaves no code span or autolink for a
/// hyphen escape to land inside), the few line starts it leaves able to
/// open a block — a list marker, a setext underline, a definition —
/// escaped too, and no hyphen pair left for smart punctuation.
fn record_safe(l: &str) -> String {
    let mut s = escape_inline(l);
    let digits = s.bytes().take_while(u8::is_ascii_digit).count();
    if s.starts_with(['-', '+', '=', ':']) {
        s.insert(0, '\\');
    } else if (1..=9).contains(&digits) && matches!(s.as_bytes().get(digits), Some(b'.' | b')')) {
        s.insert(digits, '\\');
    }
    keep_hyphens(&s).into_owned()
}

/// Escape hyphens so smart punctuation finds no pair to turn into a dash —
/// outside code spans, where a backslash is literal, and outside
/// autolinks, where it would change the address. An escaped hyphen ends a
/// run, so only a hyphen followed by another unescaped one needs it.
///
/// Every `$` is escaped too. A `$` in a commit message is shell (`$$`,
/// `$HOME`), and math is the one inline construct that outranks a code
/// span: `$$` lines around a backtick run swallowed it, so the span this
/// scan had skipped over was never a span and its `--` became a dash.
/// With no math possible, the spans found here are the spans the parser
/// finds.
fn keep_hyphens(s: &str) -> std::borrow::Cow<'_, str> {
    if !s.contains("--") && !s.contains('$') {
        return s.into();
    }
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len() + 8);
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            // An escape pair is copied whole: its character is literal.
            b'\\' if i + 1 < b.len() => {
                let n = 1 + s[i + 1..].chars().next().map_or(0, char::len_utf8);
                out.push_str(&s[i..i + n]);
                i += n;
            }
            // A code span — a run of backticks closed by a run of the same
            // length — is copied verbatim. An unclosed run is literal text.
            b'`' => {
                let run = b[i..].iter().take_while(|&&c| c == b'`').count();
                let ticks = &b[i..i + run];
                let close = (i + run..b.len()).find(|&k| {
                    b[k..].starts_with(ticks)
                        && b.get(k + run) != Some(&b'`')
                        && (k == 0 || b[k - 1] != b'`')
                });
                let end = close.map_or(i + run, |k| k + run);
                out.push_str(&s[i..end]);
                i = end;
            }
            // An autolink: `<`, no space, a scheme's colon or a mail's `@`,
            // `>`. Raw HTML and a bare `<!--` are not, and get escaped.
            b'<' => {
                let span = s[i + 1..]
                    .find(|c: char| c == '>' || c == '<' || c.is_whitespace())
                    .filter(|&k| b[i + 1 + k] == b'>')
                    .map(|k| &s[i + 1..i + 1 + k])
                    .filter(|t| is_autolink(t));
                let n = span.map_or(1, |t| t.len() + 2);
                out.push_str(&s[i..i + n]);
                i += n;
            }
            b'-' if b.get(i + 1) == Some(&b'-') => {
                out.push_str("\\-");
                i += 1;
            }
            b'$' => {
                out.push_str("\\$");
                i += 1;
            }
            _ => {
                let n = s[i..].chars().next().map_or(1, char::len_utf8);
                out.push_str(&s[i..i + n]);
                i += n;
            }
        }
    }
    out.into()
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

/// Backslash-escape every character that means something inline, so a
/// record line or a file path reads exactly as git wrote it: `<mail>` keeps
/// its brackets instead of becoming an autolink, `*` and `_` stay literal.
fn escape_inline(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        if matches!(
            c,
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '#' | '!' | '|' | '~' | '&' | '$'
        ) {
            out.push('\\');
        }
        out.push(c);
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
fn block_safe(line: &str) -> std::borrow::Cow<'_, str> {
    let mut rest = line;
    let mut markers = String::new();
    while let Some(marker_len) = list_marker(rest) {
        let (marker, after) = rest.split_at(marker_len);
        markers.push_str(marker.trim_end());
        markers.push(' ');
        rest = after.trim_start();
    }
    if markers.is_empty() {
        return neutralize(line);
    }
    markers.push_str(&neutralize(rest));
    markers.into()
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
fn neutralize(line: &str) -> std::borrow::Cow<'_, str> {
    if !line.is_empty() && line.chars().all(|c| "-=*_|: \t".contains(c)) {
        // Every glyph, not just the first: `\---` would still read `--` as
        // an en dash, so the line would print as `-–`.
        let mut out = String::with_capacity(line.len() * 2);
        for c in line.chars() {
            if c.is_ascii_punctuation() {
                out.push('\\');
            }
            out.push(c);
        }
        return out.into();
    }
    let opens_block = line.starts_with(['#', '>', '<', '|', ':'])
        || line.starts_with("```")
        || line.starts_with("~~~")
        // A reference definition — or the first line of one whose label
        // runs onto the next line, which vanishes all the same.
        || (line.starts_with('[') && (line.contains("]:") || !line.contains(']')));
    if opens_block {
        format!("\\{line}").into()
    } else {
        line.into()
    }
}

/// Turn a unified diff — or `git log`/`git show` output — into markdown.
///
/// Commits become `#` headings, files become `##` headings with their line
/// counts, and each file's hunks become one `diff`-tagged fence. Anything that
/// is neither (a commit message, `Author:`, trailing prose) passes through as
/// text — see [`emit_message`] for how it keeps its lines and its section —
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

    let flush_message = |message: &mut Vec<&str>, out: &mut String| {
        emit_message(message, out);
        message.clear();
    };

    for line in src.lines() {
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
            if line.starts_with('+') && !line.starts_with("+++") {
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
                // And no message text vanishes — into a link reference
                // definition, say. (Inline HTML may hide what it wraps, as
                // the author's own markup; it stays inside its paragraph.)
                // No hyphen pair in a message becomes a dash.
                let doc = Document::parse(&to_markdown(&src));
                prop_assert!(
                    !doc.text.contains(['\u{2013}', '\u{2014}']),
                    "{:?}\n{}", doc.text, to_markdown(&src)
                );
                if !src.contains('<') {
                    prop_assert_eq!(
                        doc.text.matches("word").count(),
                        src.matches("word").count(),
                        "{:?}", doc.text
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
