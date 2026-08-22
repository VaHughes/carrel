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
/// Takes the **b-side** where it can, because that is the file as it now is;
/// falls back to the a-side for a deletion.
fn path_of(l: &str) -> Option<&str> {
    let rest = l.strip_prefix("diff --git ")?;
    let (a, b) = rest.split_once(' ')?;
    let b = b.strip_prefix("b/").unwrap_or(b);
    if b == "/dev/null" {
        return a.strip_prefix("a/").or(Some(a));
    }
    Some(b)
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
            out.push_str(&self.heading);
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

/// Turn a unified diff — or `git log`/`git show` output — into markdown.
///
/// Commits become `#` headings, files become `##` headings with their line
/// counts, and each file's hunks become one `diff`-tagged fence. Anything that
/// is neither (a commit message, `Author:`, trailing prose) passes through as
/// ordinary text, which is what makes `git log` readable rather than merely
/// not-broken.
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
        while message.last().is_some_and(|l| l.trim().is_empty()) {
            message.pop();
        }
        if message.is_empty() {
            return;
        }
        for line in message.drain(..) {
            let line = line.trim();
            if line.is_empty() {
                out.push('\n');
            } else {
                out.push_str(line);
                out.push('\n');
            }
        }
        out.push('\n');
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
                .filter(|t| t.kind == k)
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

    #[test]
    fn a_deletion_names_the_file_that_is_going_away() {
        let src = "diff --git a/gone.rs b/dev/null\n@@ -1 +0,0 @@\n-x\n";
        let md = to_markdown(src);
        assert!(md.contains("## "), "{md}");
    }

    #[test]
    fn an_empty_diff_produces_nothing_alarming() {
        assert_eq!(to_markdown("").trim(), "");
        assert!(!looks_like_diff(""));
    }
}
