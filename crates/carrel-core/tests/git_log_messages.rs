//! `git log` messages through `to_markdown`, and the wikilink shape that
//! broke every document: what a cold review and a black-box tester found
//! in the one-pass escape settling (2026-11-14), each break-checked.

use carrel_core::{Document, NodeKind, to_markdown};
use std::time::Instant;

/// One commit whose message is `msg`, indented as git indents it.
fn commit(msg: &str) -> String {
    let mut s = String::from("commit 0abcdef0123456789abcdef0123456789abcdef\nAuthor: A\n\n");
    for l in msg.split('\n') {
        s.push_str("    ");
        s.push_str(l);
        s.push('\n');
    }
    s
}

fn shown(msg: &str) -> String {
    Document::parse(&to_markdown(&commit(msg))).text
}

fn last_line(msg: &str) -> String {
    shown(msg).trim_end().lines().last().unwrap().to_string()
}

/// Breaking a code span's opener (for a `\<`, `\>`, `\|` the line start
/// needs) left its closer to pair with the NEXT span's opener, so every
/// later span in the paragraph was misread: words glued, text vanished.
#[test]
fn breaking_a_code_span_does_not_repair_later_backticks() {
    for start in ["<c>", ">c", "|c", "[x]: y"] {
        let line = last_line(&format!("a `b\n{start}` d `e` f"));
        assert!(
            line.ends_with(" d e f") || line.ends_with("` d `e` f"),
            "{start:?}: {line:?}"
        );
    }
}

/// The real commit (in another repository) that showed it: `<pid>`
/// vanished and spaces were lost.
#[test]
fn a_real_paragraph_keeps_its_text() {
    let msg = "That is what makes `owlery\n<host>` from another terminal open a tab, and\n\
               launching it tests the *old* one. `readlink /proc/<pid>/exe` printing `(deleted)`\n\
               is the quickest way.";
    let t = shown(msg);
    assert!(t.contains("<pid>"), "{t:?}");
    assert!(
        t.contains("<host>` from") || t.contains("<host> from"),
        "{t:?}"
    );
    assert!(!t.contains("exeprinting"), "{t:?}");
}

/// `settle` re-parsed the whole paragraph once per construct it broke, so
/// a long message of them took seconds.
#[test]
fn a_50kb_message_settles_in_well_under_a_second() {
    for (name, msg) in [
        ("comment cascade", "<!--\n`-->\n".repeat(2800)),
        (
            "backtick ladder",
            (0..2900).fold(String::new(), |mut s, i| {
                s.push_str(&"`".repeat(i % 7 + 1));
                s.push_str(" x\n#\n");
                s
            }),
        ),
        (
            "records with breaks",
            "text\n        indented `x\n- item `y\n".repeat(1500),
        ),
    ] {
        let src = commit(msg.trim_end());
        let t = Instant::now();
        let _ = Document::parse(&to_markdown(&src));
        let e = t.elapsed();
        assert!(
            e.as_millis() < 500,
            "{name}: {} bytes took {e:?}",
            src.len()
        );
    }
}

/// A lone CR inside a message line is a line ending to the parser but not
/// to `str::lines`, so the line start after it was never neutralized.
#[test]
fn a_carriage_return_inside_a_message_line_cannot_open_a_fence() {
    let src = "commit 1111111111111111111111111111111111111111\nAuthor: A\n\n    body\r# stolen\r```\n\n\
               commit 2222222222222222222222222222222222222222\nAuthor: B\n\n    Second\n";
    let d = Document::parse(&to_markdown(src));
    let headings: Vec<_> = d
        .nodes
        .iter()
        .filter(|n| matches!(n.kind, NodeKind::Heading { .. }))
        .map(|n| d.text[n.doc.start as usize..n.doc.end as usize].to_string())
        .collect();
    assert_eq!(
        headings,
        ["commit 111111111111", "commit 222222222222"],
        "{}",
        to_markdown(src)
    );
    assert!(d.text.contains("body # stolen ```"), "{:?}", d.text);
}

/// A carrel-added `\-` cut a bare URL out of the autolink scan: no link.
/// And the URL shows its hyphens, not a dash.
#[test]
fn a_bare_url_with_a_flag_in_a_message_still_links() {
    for (url, link) in [
        ("https://x.example/a--b", "https://x.example/a--b"),
        ("www.a--b.example", "http://www.a--b.example"),
        ("https://x.example/$HOME", "https://x.example/$HOME"),
    ] {
        let d = Document::parse(&to_markdown(&commit(&format!("see {url} ok"))));
        assert_eq!(d.links.iter().map(|l| &**l).collect::<Vec<_>>(), [link]);
        assert!(d.text.contains(&format!("see {url} ok")), "{:?}", d.text);
    }
    // Where the reader does not link it after all, the `--` is still two
    // hyphens: inside a link label, and after an autolink's `>`.
    for msg in [
        "[see https://x/a--b more](y)",
        "#<http://a--b>--force",
        "<http://a--b>--force",
    ] {
        let t = shown(msg);
        assert!(
            !t.contains(['\u{2013}', '\u{2014}', '\\']),
            "{msg:?}: {t:?}"
        );
    }
}

/// An item's wrapped line, indented to the item's text, is the item — not
/// an indented record line, which shows its `code` with the backticks.
#[test]
fn a_list_items_second_line_keeps_its_code_spans() {
    let msg = "Subject\n\nIntro:\n\n  - First line has `code1` here and\n    a second line with `code2` in it.\n  - Another item `code3`.\n\n\
               - Top-level `a` and\n  wrapped `b`.";
    let t = shown(msg);
    assert!(!t.contains('`'), "{t:?}");
    assert!(t.contains("a second line with code2 in it."), "{t:?}");
    assert!(t.contains("wrapped b."), "{t:?}");
    // A line indented four past the item's text is still a record.
    let t = shown("Subject\n\n- item\n      deep `x`");
    assert!(t.contains("deep `x`"), "{t:?}");
}

/// pulldown-cmark 0.13.4 emits the text after an empty-label wikilink
/// twice, and misparses the copy, in any document.
#[test]
fn an_empty_label_wikilink_does_not_duplicate_text() {
    for (src, text, links) in [
        ("[[x|]] abc\n", "[[x|]] abc", &[][..]),
        ("q [[x|]] abc [[y]] d\n", "q [[x|]] abc y d", &["y"][..]),
        ("*[[x|]] a* b\n", "[[x|]] a b", &[][..]),
        ("[[x|]]\n[[x|]]\n[word#\n", "[[x|]] [[x|]] [word#", &[][..]),
        ("[[a\\|]] b\n", "[[a|]] b", &[][..]),
        ("- [[x|]] a\n- b\n", "[[x|]] a", &[][..]),
    ] {
        let d = Document::parse(src);
        assert!(d.text.contains(text), "{src:?}: {:?}", d.text);
        assert_eq!(d.text.matches(text).count(), 1, "{src:?}: {:?}", d.text);
        assert_eq!(
            d.links.iter().map(|l| &**l).collect::<Vec<_>>(),
            links,
            "{src:?}"
        );
    }
}

/// In any document, a bare URL shows the hyphens it was typed with, and a
/// quote closing it is not part of it — in either curl.
#[test]
fn a_bare_url_shows_as_typed_and_drops_a_closing_quote() {
    let d = Document::parse("see https://x.example/a--b ok\n");
    assert!(d.text.contains("https://x.example/a--b"), "{:?}", d.text);
    assert_eq!(&*d.links[0], "https://x.example/a--b");
    let d = Document::parse("see https://a.example/x=\"\n");
    assert_eq!(&*d.links[0], "https://a.example/x=");
    assert!(d.text.trim_end().ends_with("x=\u{201c}"), "{:?}", d.text);
}

/// The guard changes nothing about any other wikilink.
#[test]
fn every_other_wikilink_reads_as_before() {
    for (src, text, links) in [
        ("[[x]] abc\n", "x abc", &["x"][..]),
        ("[[x|y]] abc\n", "y abc", &["x"][..]),
        ("[[x| ]] abc\n", "  abc", &["x"][..]),
        ("[[|]] abc\n", "[[|]] abc", &[][..]),
        ("a [[x|y]] b [[z]] c\n", "a y b z c", &["x", "z"][..]),
        ("*[[x|y]]* b\n", "y b", &["x"][..]),
    ] {
        let d = Document::parse(src);
        assert!(d.text.contains(text), "{src:?}: {:?}", d.text);
        assert_eq!(
            d.links.iter().map(|l| &**l).collect::<Vec<_>>(),
            links,
            "{src:?}"
        );
    }
}

/// An embed wikilink `![[…]]` shows its target raw, like a plain one, so
/// an escape inside it showed as a backslash.
#[test]
fn an_embed_wikilink_shows_no_added_backslash() {
    for (msg, want) in [
        ("Use ![[my--notes]] and ![[cost $5]] here", "my--notes"),
        ("see ![[a\n> quoted]] end", "quoted"),
        ("x ![[#|---]] y", "---"),
        ("![[a--b]]", "a--b"),
    ] {
        let t = shown(msg);
        assert!(
            !t.contains(['\\', '\u{2013}', '\u{2014}']),
            "{msg:?}: {t:?}"
        );
        assert!(t.contains(want), "{msg:?}: {t:?}");
    }
}

/// A bare URL typed on into a wikilink was one URL with the escapes in
/// and a wikilink holding math with them out, so the escapes were dropped
/// for a link the reader never made.
#[test]
fn a_url_typed_on_into_a_wikilink_keeps_its_text() {
    for msg in [
        "see http://a[[$|f$]]-- and --force",
        "www.a.b/[[$|f$]]--",
        "http://a[[$|f$]]--",
        "http://a![[$|f$]]--",
        "http://a[[b--c]]--",
    ] {
        let t = shown(msg);
        assert!(t.contains(msg), "{msg:?}: {t:?}");
    }
}

/// The file headings and fenced bodies of a diff, as the reader shows them.
fn diff_shown(src: &str) -> String {
    Document::parse(&to_markdown(src)).text
}

/// Inside a hunk a line is content whatever it starts with: a removed
/// `-- comment` is written `--- comment`, and an added `++ i;` is `+++ i;`.
/// Both were taken for a file header and dropped, and the counts with them.
#[test]
fn a_hunk_keeps_lines_that_look_like_file_headers() {
    let src = "diff --git a/x.sql b/x.sql\n--- a/x.sql\n+++ b/x.sql\n@@ -1,3 +1,3 @@\n keep\n--- old comment\n+-- new comment\n+++ i;\n";
    let t = diff_shown(src);
    assert!(t.contains("x.sql  +2 \u{2212}1"), "{t:?}");
    assert!(
        t.contains("--- old comment\n+-- new comment\n+++ i;"),
        "{t:?}"
    );
    // The header pair itself is still dropped, in every file of a diff, and
    // so is the plumbing before a hunk.
    let two = format!(
        "{src}diff --git a/y b/y\nindex 1..2 100644\n--- a/y\n+++ b/y\n@@ -1 +1 @@\n-a\n+b\n"
    );
    let t = diff_shown(&two);
    assert!(!t.contains("--- a/"), "{t:?}");
    assert!(!t.contains("+++ b/"), "{t:?}");
    assert!(!t.contains("index 1..2"), "{t:?}");
    assert!(t.contains("y  +1 \u{2212}1"), "{t:?}");
    // A hunk header with no count means one line; `\ No newline` is no line.
    let src = "diff --git a/z b/z\n--- a/z\n+++ b/z\n@@ -1 +1 @@\n--- a\n\\ No newline at end of file\n+++ b\n\\ No newline at end of file\n";
    let t = diff_shown(src);
    assert!(t.contains("z  +1 \u{2212}1"), "{t:?}");
    assert!(t.contains("--- a\n"), "{t:?}");
    assert!(t.contains("+++ b\n"), "{t:?}");
    // A line of the hunk that reads like plumbing is content too.
    let src = "diff --git a/w b/w\n--- a/w\n+++ b/w\n@@ -1,2 +1,2 @@\n index 3 of 4\n-old mode on\n+new mode off\n";
    let t = diff_shown(src);
    assert!(
        t.contains(" index 3 of 4\n-old mode on\n+new mode off"),
        "{t:?}"
    );
}

/// The fenced blocks of a diff as the reader holds them, in order.
fn fences(src: &str) -> Vec<String> {
    let doc = Document::parse(&to_markdown(src));
    doc.nodes
        .iter()
        .filter(|n| matches!(n.kind, NodeKind::CodeBlock { .. }))
        .map(|n| doc.text[n.doc.start as usize..n.doc.end as usize].to_string())
        .collect()
}

fn headings(src: &str) -> Vec<String> {
    let doc = Document::parse(&to_markdown(src));
    doc.nodes
        .iter()
        .filter(|n| matches!(n.kind, NodeKind::Heading { .. }))
        .map(|n| doc.text[n.doc.start as usize..n.doc.end as usize].to_string())
        .collect()
}

/// `git format-patch` writes a mail per commit: a `From <sha>` line, mail
/// headers, the message, the diff, and a `-- ` signature. A hunk ends where
/// its `@@` counts say, so the signature is no removed line and the next
/// mail is no part of the file before it.
#[test]
fn a_format_patch_mbox_is_a_commit_per_mail() {
    let src = "From 1111111111111111111111111111111111111111 Mon Sep 17 00:00:00 2001\n\
From: A <a@x>\nDate: Mon, 1 Jan 2026 00:00:00 +0000\nSubject: [PATCH 1/2] first thing\n\n\
Body text.\n---\n f.txt | 2 +-\n 1 file changed, 1 insertion(+), 1 deletion(-)\n\n\
diff --git a/f.txt b/f.txt\nindex 111..222 100644\n--- a/f.txt\n+++ b/f.txt\n\
@@ -1,2 +1,2 @@\n one\n-two\n+TWO\n-- \n2.47.0\n\n\
From 2222222222222222222222222222222222222222 Mon Sep 17 00:00:00 2001\n\
From: A <a@x>\nSubject: [PATCH 2/2] second\n\n---\n\
diff --git a/g.txt b/g.txt\nindex 111..222 100644\n--- a/g.txt\n+++ b/g.txt\n\
@@ -1 +1 @@\n-x\n+y\n-- \n2.47.0\n\n";
    assert_eq!(
        headings(src),
        [
            "commit 111111111111",
            "f.txt  +1 \u{2212}1",
            "commit 222222222222",
            "g.txt  +1 \u{2212}1"
        ]
    );
    assert_eq!(
        fences(src),
        [
            "@@ -1,2 +1,2 @@\n one\n-two\n+TWO\n",
            "@@ -1 +1 @@\n-x\n+y\n"
        ]
    );
    let t = diff_shown(src);
    // Nothing of a mail is lost: its headers, its message, its signature.
    for want in [
        "From: A <a@x>\nDate: Mon, 1 Jan 2026 00:00:00 +0000\nSubject: [PATCH 1/2] first thing",
        "Body text.",
        "1 file changed, 1 insertion(+), 1 deletion(-)",
        "2.47.0",
        "Subject: [PATCH 2/2] second",
    ] {
        assert!(t.contains(want), "{want:?} not in {t:?}");
    }
    assert_eq!(t.matches("-- 2.47.0").count(), 2, "{t:?}");
    assert!(carrel_core::looks_like_diff(
        "From 1111111111111111111111111111111111111111 Mon Sep 17 00:00:00 2001\nFrom: A\n"
    ));
    // A line of prose that opens with `From` is no mail.
    assert_eq!(
        headings("commit 1111111111111111111111111111111111111111\n\nFrom 2222222 we learned\n"),
        ["commit 111111111111"]
    );
}

/// What follows a diff's last hunk is not the diff: a patch's signature,
/// or prose under plain `diff -u` output.
#[test]
fn text_after_the_last_hunk_is_outside_the_fence() {
    // One patch, as `git am` takes it.
    let src = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1 +1 @@\n-x\n+y\n-- \n2.43.0\n";
    assert_eq!(headings(src), ["f  +1 \u{2212}1"]);
    assert_eq!(fences(src), ["@@ -1 +1 @@\n-x\n+y\n"]);
    assert!(
        diff_shown(src).contains("-- 2.43.0"),
        "{:?}",
        diff_shown(src)
    );
    // `\\ No newline` belongs to the hunk it follows, and a second hunk to
    // its file.
    let src = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1 +1 @@\n-x\n+y\n\\ No newline at end of file\n@@ -9 +9 @@\n-p\n+q\n\nThat is all.\n";
    assert_eq!(headings(src), ["f  +2 \u{2212}2"]);
    assert_eq!(
        fences(src),
        ["@@ -1 +1 @@\n-x\n+y\n\\ No newline at end of file\n@@ -9 +9 @@\n-p\n+q\n"]
    );
    assert!(
        diff_shown(src).ends_with("That is all.\n"),
        "{:?}",
        diff_shown(src)
    );
    // Plain `diff -u`, two files, then a remark.
    let src = "--- a.txt\t2026-01-01\n+++ b.txt\t2026-01-02\n@@ -1,3 +1,3 @@\n one\n-two\n+TWO\n three\n--- c.txt\n+++ d.txt\n@@ -1 +1 @@\n-x\n+y\nSee the - list above.\n- one more\n";
    let fenced = fences(src);
    assert_eq!(fenced.len(), 1, "{fenced:?}");
    assert!(fenced[0].ends_with("@@ -1 +1 @@\n-x\n+y\n"), "{fenced:?}");
    assert!(
        diff_shown(src).contains("See the - list above."),
        "{:?}",
        diff_shown(src)
    );
    // The lines other tools write between files stay with the diff.
    for src in [
        "Index: f.txt\n=====\n--- f.txt\t(revision 1)\n+++ f.txt\t(working copy)\n@@ -1 +1 @@\n-a\n+b\nIndex: g.txt\n=====\n--- g.txt\n+++ g.txt\n@@ -1 +1 @@\n-x\n+y\n",
        "diff -ru a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\nOnly in b: new.txt\ndiff -ru a/y b/y\n--- a/y\n+++ b/y\n@@ -1 +1 @@\n-x\n+y\n",
    ] {
        let fenced = fences(src);
        assert_eq!(fenced.len(), 1, "{fenced:?}");
        assert!(
            fenced[0].ends_with("--- a/y\n+++ b/y\n@@ -1 +1 @@\n-x\n+y\n")
                || fenced[0].ends_with("+++ g.txt\n@@ -1 +1 @@\n-x\n+y\n"),
            "{fenced:?}"
        );
    }
    // A header that cannot be believed costs no line and no count: a count
    // too large to hold, and one that counts nothing over lines that exist.
    for header in ["@@ -1,99999999999999999999 +1,2 @@", "@@ -0,0 +0,0 @@"] {
        let src = format!("diff --git a/f b/f\n--- a/f\n+++ b/f\n{header}\n-two\n+TWO\n");
        assert_eq!(headings(&src), ["f  +1 \u{2212}1"], "{header}");
        assert_eq!(fences(&src), [format!("{header}\n-two\n+TWO\n")]);
    }
    // A hunk cut short by the next file loses nothing and counts what is there.
    let src = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1,5 +1,5 @@\n one\n-two\n+TWO\ndiff --git a/g b/g\n--- a/g\n+++ b/g\n@@ -1 +1 @@\n-g1\n+g2\n";
    assert_eq!(headings(src), ["f  +1 \u{2212}1", "g  +1 \u{2212}1"]);
}

/// The counts beside a file's name are of the lines shown: `---i;` is a
/// removed line and `+++++` an added one, and neither was counted.
#[test]
fn a_files_counts_are_of_the_hunk_lines_shown() {
    let src = "diff --git a/x.c b/x.c\n--- a/x.c\n+++ b/x.c\n@@ -1,4 +1,4 @@\n for (;;) {\n---i;\n-----\n+++i;\n+++++\n }\n";
    let t = diff_shown(src);
    assert!(t.starts_with("x.c  +2 \u{2212}2\n"), "{t:?}");
    assert!(t.contains("\n---i;\n-----\n+++i;\n+++++\n }"), "{t:?}");
}

mod hunks {
    use super::*;
    use proptest::prelude::*;
    use std::fmt::Write as _;

    /// What a line of a hunk may say after its sigil: everything the
    /// adapter looks for at the start of a line outside one.
    const CONTENT: &[&str] = &[
        "",
        "x",
        "- note",
        "-- note",
        "+ more",
        "++ i;",
        "-- a/x",
        "++ b/x",
        "--",
        "++",
        "index 1..2 100644",
        "new file mode 100644",
        "rename from a",
        "commit 0abcdef0123456789",
        "diff --git a/q b/q",
        "@@ -1 +1 @@",
        "```",
        "~~~~",
        "# heading",
        " indented",
        "\\ No newline at end of file",
    ];

    fn hunk() -> impl Strategy<Value = Vec<(char, &'static str)>> {
        prop::collection::vec(
            (
                prop::sample::select(vec![' ', '-', '+']),
                prop::sample::select(CONTENT),
            ),
            1..12,
        )
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(2000))]
        /// Every line of every hunk comes back, in order and whole, and
        /// the counts are the hunk's own arithmetic.
        #[test]
        fn every_hunk_line_is_shown_and_counted(
            files in prop::collection::vec(prop::collection::vec(hunk(), 1..3), 1..3),
            in_a_log in any::<bool>(),
        ) {
            let mut src = String::new();
            let mut want = String::new();
            if in_a_log {
                src.push_str("commit 0abcdef0123456789abcdef0123456789abcdef\nAuthor: A\n\n    Subject\n\n");
                want.push_str("commit 0abcdef01234\nAuthor: A\nSubject\n");
            }
            for (i, hunks) in files.iter().enumerate() {
                let _ = write!(
                    src,
                    "diff --git a/f{i} b/f{i}\nindex 1..2 100644\n--- a/f{i}\n+++ b/f{i}\n"
                );
                let (mut added, mut removed) = (0, 0);
                let mut body = String::new();
                for lines in hunks {
                    let old = lines.iter().filter(|(sigil, _)| *sigil != '+').count();
                    let new = lines.iter().filter(|(sigil, _)| *sigil != '-').count();
                    let _ = writeln!(body, "@@ -1,{old} +1,{new} @@");
                    for (sigil, content) in lines {
                        let _ = writeln!(body, "{sigil}{content}");
                    }
                    added += lines.len() - old;
                    removed += lines.len() - new;
                }
                src.push_str(&body);
                let _ = write!(want, "f{i}");
                if added + removed > 0 {
                    let _ = write!(want, "  +{added} \u{2212}{removed}");
                }
                let _ = write!(want, "\n{body}\n");
            }
            prop_assert_eq!(diff_shown(&src), want, "{}", src);
        }
    }
}

/// A path is shown as it is named: no dash, ellipsis or curled quote.
#[test]
fn a_file_heading_is_not_smart_punctuated() {
    for path in [
        "my--file...x.sql",
        "it's \"quoted\".md",
        "a---b.md",
        "x^2^.md",
        "H~2~O.md",
        "a..b....c",
        "dir/-- notes --/x",
        "www.a--b.example/it's...x.html",
        "a\\b ^c^ [[x|]] $y$ ![[z--w]]",
        "---",
        "...",
        "\u{e9}...\u{fc}--\u{2014}",
    ] {
        let src = format!("diff --git a/{path} b/{path}\n@@ -1 +1 @@\n-a\n+b\n");
        let t = diff_shown(&src);
        assert!(
            t.starts_with(&format!("{path}  +1 \u{2212}1")),
            "{path:?}: {t:?}"
        );
    }
}

/// A record line — a header, a trailer, an indented line, a `#` comment —
/// is shown exactly as written: no superscript, no ellipsis, no curled
/// quote.
#[test]
fn a_record_line_is_shown_as_written() {
    let msg = "Subject\n\n    x = a^b^c and H~2~O \"q\" it's... done\n\nSigned-off-by: A^x^ \"B\" <a@b.c>";
    let t = shown(msg);
    assert!(
        t.contains("x = a^b^c and H~2~O \"q\" it's... done"),
        "{t:?}"
    );
    assert!(t.contains("Signed-off-by: A^x^ \"B\" <a@b.c>"), "{t:?}");
    let src = "commit 0abcdef0123456789abcdef0123456789abcdef\nAuthor: O'Neil \"Red\" ... <o@n.il>\n\n    S\n\n    # a 'b' c.. d... e^f^\n";
    let t = Document::parse(&to_markdown(src)).text;
    assert!(t.contains("Author: O'Neil \"Red\" ... <o@n.il>"), "{t:?}");
    assert!(t.contains("# a 'b' c.. d... e^f^"), "{t:?}");
}

/// A line that is only a list marker is the text it is, not an empty item.
#[test]
fn a_lone_list_marker_is_text() {
    for marker in ["+", "1.", "2)", "-", "*", "123456789."] {
        let src = commit(&format!("Subject\n\n{marker}\ntext after"));
        let d = Document::parse(&to_markdown(&src));
        assert!(
            d.text.contains(&format!("{marker} text after")),
            "{marker:?}: {:?}",
            d.text
        );
        assert!(
            !d.nodes.iter().any(|n| matches!(n.kind, NodeKind::Item)),
            "{marker:?} opened a list"
        );
        // Under a marker with text, which is a list, it is still shown.
        let t = shown(&format!("Subject\n\n- item\n{marker}\n"));
        assert!(t.contains(marker), "{marker:?}: {t:?}");
    }
}

/// A commit message is not HTML: `<sha>` and `Vec<String>` are text, and
/// the reader's tag stripping deleted them.
#[test]
fn angle_brackets_in_a_message_are_shown() {
    let msg = "Use gh release create --target <sha> when needed\n\n\
               Sampling /proc/<pid>/stat showed that Vec<String> and From<Intent> are fine.\n\
               A file named README<ESC>c.md could reset it. See <details> folds, <b>bold</b>,\n\
               <a href=\"x--y\">, <!-- a note -->, <?php x ?> and </div>.\n\
               Control: a 3 < 4 comparison and an email <a@b.co> stay.\n\n\
               \x20   indented <kbd>\n\
               Reviewed-by: R <r@x.y> <tag>";
    let t = shown(msg);
    for want in [
        "--target <sha> when",
        "/proc/<pid>/stat",
        "Vec<String> and From<Intent> are",
        "README<ESC>c.md",
        "See <details> folds, <b>bold</b>,",
        "<a href=\u{201c}x--y\u{201d}>, <!-- a note -->, <?php x ?> and </div>.",
        "a 3 < 4 comparison",
        "indented <kbd>",
        "Reviewed-by: R <r@x.y> <tag>",
    ] {
        assert!(t.contains(want), "{want:?} not in {t:?}");
    }
    assert!(!t.contains('\\'), "{t:?}");
    // An autolink the author wrote is still a link.
    let d = Document::parse(&to_markdown(&commit(
        "see <https://x.example/a--b> and <b>",
    )));
    assert_eq!(
        d.links.iter().map(|l| &**l).collect::<Vec<_>>(),
        ["https://x.example/a--b"]
    );
    assert!(d.text.contains("and <b>"), "{:?}", d.text);
}
