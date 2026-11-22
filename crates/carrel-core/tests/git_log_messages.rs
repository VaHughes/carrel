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
    // The diffstat is a block of its own, as written; a mail with none
    // (`---` and then the diff) has no such block.
    assert_eq!(
        fences(src),
        [
            "---\n f.txt | 2 +-\n 1 file changed, 1 insertion(+), 1 deletion(-)\n",
            "@@ -1,2 +1,2 @@\n one\n-two\n+TWO\n",
            "@@ -1 +1 @@\n-x\n+y\n"
        ]
    );
    let t = diff_shown(src);
    // A mail reads as the commit it is: who and when on their own lines,
    // the subject as its title, then the message.
    for want in [
        "commit 111111111111\nAuthor: A <a@x>\nDate: Mon, 1 Jan 2026 00:00:00 +0000\nfirst thing\nBody text.\n",
        "commit 222222222222\nAuthor: A <a@x>\nsecond\n",
        "2.47.0",
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

/// Three commits as `git format-patch --stdout -3` writes them (the diffs
/// cut to a hunk each): a subject folded over two lines with trailers under
/// it, a body of paragraphs and a list, and a mail with no body at all.
const MBOX: &str = "\
From f674d5af35797e58b9588de27fac28762ea73ef7 Mon Sep 17 00:00:00 2001
From: A U Thor <author@example.com>
Date: Sat, 21 Nov 2026 18:07:26 -0500
Subject: [PATCH 1/3] A wikilink's hyperlink is the note's absolute file URI,
 on screen and in --render; an unresolved one gets none

Co-Authored-By: Some One <someone@example.com>
Claude-Session: https://example.com/session_01
---
 CHANGELOG.md                |  11 ++-
 crates/carrel/src/ansi.rs   | 162 +++++++++++++++++++++++++++++++++---
 crates/carrel/src/main.rs   |   2 +-
 3 files changed, 162 insertions(+), 13 deletions(-)

diff --git a/CHANGELOG.md b/CHANGELOG.md
index 6b95793..c1475e5 100644
--- a/CHANGELOG.md
+++ b/CHANGELOG.md
@@ -1,2 +1,2 @@
 one
-two
+TWO
--
2.56.0


From 827ab3500000000000000000000000000000abcd Mon Sep 17 00:00:00 2001
From: A U Thor <author@example.com>
Date: Sat, 21 Nov 2026 18:20:01 -0500
Subject: [PATCH 2/3] A double click keeps a word's combining marks

Three from the third pass. A file named bare has the empty path for
a folder, which resolves to nothing -- so `--force` did nothing.

- the first <thing>
- the second

Signed-off-by: A U Thor <author@example.com>
---
 crates/carrel/src/app.rs | 199 +++++++++++++++++++++++++++++++++-------
 1 file changed, 190 insertions(+), 9 deletions(-)

diff --git a/crates/carrel/src/app.rs b/crates/carrel/src/app.rs
index 111..222 100644
--- a/crates/carrel/src/app.rs
+++ b/crates/carrel/src/app.rs
@@ -9 +9 @@
-x
+y
--
2.56.0


From fe660bd000000000000000000000000000001234 Mon Sep 17 00:00:00 2001
From: A U Thor <author@example.com>
Date: Sat, 21 Nov 2026 19:02:44 -0500
Subject: [PATCH 3/3] STATUS: what is left

---
 docs/STATUS.md | 28 +++++++++++++++++++++++++++-
 1 file changed, 27 insertions(+), 1 deletion(-)

diff --git a/docs/STATUS.md b/docs/STATUS.md
index 111..222 100644
--- a/docs/STATUS.md
+++ b/docs/STATUS.md
@@ -1 +1,2 @@
 a
+b
--
2.56.0

";

/// The same three as `git log -p -3` writes them.
const LOG: &str = "\
commit f674d5af35797e58b9588de27fac28762ea73ef7
Author: A U Thor <author@example.com>
Date:   Sat Nov 21 18:07:26 2026 -0500

    A wikilink's hyperlink is the note's absolute file URI, on screen and in --render; an unresolved one gets none

    Co-Authored-By: Some One <someone@example.com>
    Claude-Session: https://example.com/session_01

diff --git a/CHANGELOG.md b/CHANGELOG.md
index 6b95793..c1475e5 100644
--- a/CHANGELOG.md
+++ b/CHANGELOG.md
@@ -1,2 +1,2 @@
 one
-two
+TWO

commit 827ab3500000000000000000000000000000abcd
Author: A U Thor <author@example.com>
Date:   Sat Nov 21 18:20:01 2026 -0500

    A double click keeps a word's combining marks

    Three from the third pass. A file named bare has the empty path for
    a folder, which resolves to nothing -- so `--force` did nothing.

    - the first <thing>
    - the second

    Signed-off-by: A U Thor <author@example.com>

diff --git a/crates/carrel/src/app.rs b/crates/carrel/src/app.rs
index 111..222 100644
--- a/crates/carrel/src/app.rs
+++ b/crates/carrel/src/app.rs
@@ -9 +9 @@
-x
+y

commit fe660bd000000000000000000000000000001234
Author: A U Thor <author@example.com>
Date:   Sat Nov 21 19:02:44 2026 -0500

    STATUS: what is left

diff --git a/docs/STATUS.md b/docs/STATUS.md
index 111..222 100644
--- a/docs/STATUS.md
+++ b/docs/STATUS.md
@@ -1 +1,2 @@
 a
+b
";

/// A mail of `git format-patch` reads as `git log -p` shows the same
/// commit: the author and date each on a line, the subject as the title,
/// the message under it with its trailers on their own lines. Its headers
/// were one run-on paragraph with the diffstat another.
#[test]
fn a_format_patch_mail_reads_as_git_log_shows_the_commit() {
    // What only one of the two holds: the date, which each writes its own
    // way; the mail's diffstat and signature.
    let comparable = |text: &str| -> Vec<String> {
        text.lines()
            .filter(|l| !l.starts_with("Date:") && *l != "-- 2.56.0")
            .map(str::to_string)
            .collect()
    };
    let mail = Document::parse(&to_markdown(MBOX));
    let mut mail_text = mail.text.clone();
    let stats: Vec<String> = fences(MBOX)
        .into_iter()
        .filter(|f| f.starts_with("---\n"))
        .collect();
    assert_eq!(stats.len(), 3, "{stats:?}");
    for stat in &stats {
        mail_text = mail_text.replacen(&format!("{stat}\n"), "", 1);
    }
    let log = diff_shown(LOG);
    assert_eq!(comparable(&mail_text), comparable(&log), "{}", mail.text);
    assert_eq!(headings(MBOX), headings(LOG));
    // The diffstat is as written, column for column.
    assert_eq!(
        stats[0],
        "---\n CHANGELOG.md                |  11 ++-\n crates/carrel/src/ansi.rs   | 162 +++++++++++++++++++++++++++++++++---\n crates/carrel/src/main.rs   |   2 +-\n 3 files changed, 162 insertions(+), 13 deletions(-)\n"
    );
    // Each header is a line of its own, shown as written.
    assert!(
        mail.text.contains(
            "commit f674d5af3579\nAuthor: A U Thor <author@example.com>\nDate: Sat, 21 Nov 2026 18:07:26 -0500\nA wikilink\u{2019}s hyperlink is the note\u{2019}s absolute file URI, on screen and in --render; an unresolved one gets none\nCo-Authored-By: Some One <someone@example.com>\nClaude-Session: https://example.com/session_01\n"
        ),
        "{:?}",
        mail.text
    );
    // Headers git does not write in a log stay, each on its line.
    let mime = MBOX.replacen(
        "Subject: [PATCH 2/3] A double",
        "MIME-Version: 1.0\nContent-Type: text/plain; charset=UTF-8\nSubject: [PATCH v2 2/3] A double",
        1,
    );
    let t = diff_shown(&mime);
    assert!(
        t.contains("Date: Sat, 21 Nov 2026 18:20:01 -0500\nMIME-Version: 1.0\nContent-Type: text/plain; charset=UTF-8\nA double click keeps"),
        "{t:?}"
    );
}

/// A commit message is not HTML: `&amp;` is those five characters, and an
/// entity the reader would decode — named, decimal, hex — is shown as
/// typed, in a subject, a body, a list item and a record line alike. Where
/// nothing is decoded (a code span, a name that is no entity) no backslash
/// is added.
#[test]
fn an_entity_in_a_message_is_shown_as_typed() {
    let msg = "Math shows &nbsp; for a tie &amp; drops &lt;b&gt;\n\n\
               Body with &amp; and &#35; and &#x23; and &copy;, but `&amp;` in code,\n\
               &notanentity; and AT&T; and a lone & and R&D stay too.\n\n\
               - an item &mdash; with one\n\n\
               \x20   indented &amp; record\n\
               Reviewed-by: R&amp;D <r@x.y>";
    let t = shown(msg);
    for want in [
        "Math shows &nbsp; for a tie &amp; drops &lt;b&gt;",
        "Body with &amp; and &#35; and &#x23; and &copy;, but &amp; in code, &notanentity; and AT&T; and a lone & and R&D stay too.",
        "an item &mdash; with one",
        "indented &amp; record",
        "Reviewed-by: R&amp;D <r@x.y>",
    ] {
        assert!(t.contains(want), "{want:?} not in {t:?}");
    }
    assert!(!t.contains('\\'), "{t:?}");
    // A file's name too.
    let t = diff_shown("diff --git a/a&amp;b.md b/a&amp;b.md\n@@ -1 +1 @@\n-a\n+b\n");
    assert!(t.starts_with("a&amp;b.md  +1 \u{2212}1"), "{t:?}");
}

/// A merge's combined diff (`git show` of a merge, `git diff --cc`) names
/// its file on a `diff --cc` line: the file is a section like any other,
/// with no counts beside it — a line there carries a sigil per parent, and
/// which of them is "added" is not one number.
#[test]
fn a_combined_diff_names_its_file() {
    let src = "commit 1111111111111111111111111111111111111111\nMerge: 2222222 3333333\nAuthor: A <a@x>\n\n    Merge branch 'topic'\n\n\
diff --cc src/lib.rs\nindex 1111111,2222222..3333333\n--- a/src/lib.rs\n+++ b/src/lib.rs\n\
@@@ -1,3 -1,3 +1,4 @@@\n  one\n- two\n -deux\n++both\n +kept\n--- item\n\
diff --combined docs/a b.md\nindex 1,2..3\n--- a/docs/a b.md\n+++ b/docs/a b.md\n@@@ -1 -1 +1 @@@\n++x\n\
diff --git a/g b/g\n--- a/g\n+++ b/g\n@@ -1 +1 @@\n-p\n+q\n";
    assert_eq!(
        headings(src),
        [
            "commit 111111111111",
            "src/lib.rs",
            "docs/a b.md",
            "g  +1 \u{2212}1"
        ]
    );
    assert_eq!(
        fences(src),
        [
            "@@@ -1,3 -1,3 +1,4 @@@\n  one\n- two\n -deux\n++both\n +kept\n--- item\n",
            "@@@ -1 -1 +1 @@@\n++x\n",
            "@@ -1 +1 @@\n-p\n+q\n"
        ]
    );
}

/// A link reference definition shows nothing, so message text that reads
/// as one is gone without a trace. Which lines do is the parser's to say:
/// a label may hold an escaped `\]` and run over lines, and a line-by-line
/// guess that counted `\]` as the label's end let both lines through.
#[test]
fn message_text_never_vanishes_into_a_link_definition() {
    for msg in [
        "Subject line\n\n[skip\\] marker handling: the old code\ntreated the tag as closed (see notes]: fixed",
        "[word\\]\nword]: /u",
        "[word\\]\n]:\n/u",
        "S\n\n[a\\]b\nc\nd]: <x y> 'title\nmore'",
        "S\n\n- [a\\]\n  b]: /u\n- [c]: /v \"t\"",
        "S\n\n[a]: /one\n[a]: /two\n[b\\]]: /three",
        "S\n\n   [x\\]\n   y]:\n   /u 'w'\ntail",
    ] {
        let t = shown(msg);
        let flat = |s: &str| {
            s.replace('\\', "")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        };
        let (want, got) = (flat(&msg.replace("- ", "")), flat(&t));
        let body = want.split_once(' ').map_or(want.as_str(), |(_, rest)| rest);
        assert!(
            got.replace(['\u{2018}', '\u{2019}'], "'")
                .replace(['\u{201c}', '\u{201d}'], "\"")
                .contains(body),
            "{msg:?}: {t:?}"
        );
    }
}

/// A hunk whose header counts fewer lines than follow it keeps them: a
/// line that still reads as hunk content belongs to the hunk above, and a
/// count that lies must not turn code into a run-on paragraph. What ends
/// it is what is plainly not content: a header, a signature, prose.
#[test]
fn lines_past_a_short_count_stay_in_their_hunk() {
    let src = "diff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -1,2 +1,2 @@\n ctx1\n-old1\n+new1\n SURPLUS-CTX\n-SURPLUS-OLD\n+SURPLUS-NEW\n- # not a heading\n\\ No newline at end of file\ndiff --git a/g.txt b/g.txt\n--- a/g.txt\n+++ b/g.txt\n@@ -1 +1 @@\n-g old\n+g new\n";
    assert_eq!(
        headings(src),
        ["f.txt  +2 \u{2212}3", "g.txt  +1 \u{2212}1"]
    );
    assert_eq!(
        fences(src),
        [
            "@@ -1,2 +1,2 @@\n ctx1\n-old1\n+new1\n SURPLUS-CTX\n-SURPLUS-OLD\n+SURPLUS-NEW\n- # not a heading\n\\ No newline at end of file\n",
            "@@ -1 +1 @@\n-g old\n+g new\n"
        ]
    );
    // Still not content, short count or true: the signature, with the
    // version under it or the mail's end; the next file's header pair; a
    // line with no sigil.
    for (after, kept) in [
        ("-- \n2.47.0\n", ""),
        ("-- \n", ""),
        (
            "-- \n\nFrom 2222222222222222222222222222222222222222 Mon Sep 17 00:00:00 2001\n",
            "",
        ),
        ("--- a/h\n+++ b/h\n@@ -1 +1 @@\n-p\n+q\n", ""),
        ("Prose.\n- and a list\n", ""),
        ("\n-after a blank\n", ""),
        // But a removed `-- ` with more of the hunk under it is content,
        // and so is a removed `-- comment` (`--- comment`) with no `+++`.
        ("-- \n-more\n", "-- \n-more\n"),
        (
            "--- old comment\n+-- new comment\n",
            "--- old comment\n+-- new comment\n",
        ),
        ("+++i;\n", "+++i;\n"),
    ] {
        for header in ["@@ -1 +1 @@", "@@ -1,2 +1,2 @@"] {
            let src =
                format!("diff --git a/f b/f\n--- a/f\n+++ b/f\n{header}\n one\n-x\n+y\n{after}");
            let first = fences(&src).remove(0);
            assert_eq!(
                first,
                format!("{header}\n one\n-x\n+y\n{kept}"),
                "{after:?} under {header}"
            );
        }
    }
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

        /// A header that counts too many lines or too few costs no line
        /// and moves none out of its fence, and the counts beside the
        /// file's name are still of the lines shown.
        #[test]
        fn a_lying_count_costs_no_line(
            files in prop::collection::vec(
                prop::collection::vec((hunk(), 0usize..=8, 0usize..=8), 1..3),
                1..3,
            ),
            in_a_log in any::<bool>(),
        ) {
            let mut src = String::new();
            let mut want = String::new();
            if in_a_log {
                src.push_str("commit 0abcdef0123456789abcdef0123456789abcdef\nAuthor: A\n\n    Subject\n\n");
                want.push_str("commit 0abcdef01234\nAuthor: A\nSubject\n");
            }
            for (i, hunks) in files.iter().enumerate() {
                let _ = write!(src, "diff --git a/f{i} b/f{i}\n--- a/f{i}\n+++ b/f{i}\n");
                let (mut added, mut removed) = (0, 0);
                let mut body = String::new();
                for (lines, old_off, new_off) in hunks {
                    let old = lines.iter().filter(|(sigil, _)| *sigil != '+').count();
                    let new = lines.iter().filter(|(sigil, _)| *sigil != '-').count();
                    // Up to four lines over the truth, or four under.
                    let said = |true_count: usize, off: usize| (true_count + off).saturating_sub(4);
                    // A header that counts nothing is believed about
                    // nothing, and is another test's.
                    let (said_old, said_new) = match (said(old, *old_off), said(new, *new_off)) {
                        (0, 0) => (old, new),
                        said => said,
                    };
                    // Past a short count, `--- x` over `+++ y` reads as the
                    // next file's header, as it must.
                    prop_assume!(!lines.windows(2).any(|w| {
                        w[0].0 == '-' && w[0].1.starts_with("-- ")
                            && w[1].0 == '+' && w[1].1.starts_with("++ ")
                    }));
                    let _ = writeln!(body, "@@ -1,{said_old} +1,{said_new} @@");
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

/// A mail's headers and message around `between`, the lines from
/// `Subject:` to the first line that is plainly the message.
fn mail(between: &str) -> String {
    format!(
        "From 0123456789012345678901234567890123456789 Mon Sep 17 00:00:00 2001\n\
         From: A Person <a@example.com>\n\
         Date: Mon, 1 Jan 2026 00:00:00 +0000\n\
         {between}\
         Third line.\n---\n f | 2 +-\n 1 file changed, 1 insertion(+), 1 deletion(-)\n\n\
         diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1 +1 @@\n-a\n+b\n-- \n2.43.0\n\n"
    )
}

const MAIL_TOP: &str =
    "commit 012345678901\nAuthor: A Person <a@example.com>\nDate: Mon, 1 Jan 2026 00:00:00 +0000\n";

/// A malformed mail loses nothing an author wrote. A header written twice
/// keeps every occurrence — the first `Subject:` is the title, the next a
/// line of its own under the author — where only the last was shown.
#[test]
fn a_repeated_mail_header_keeps_every_occurrence() {
    let t = diff_shown(&mail(
        "Subject: [PATCH] first subject\nSubject: [PATCH] second subject\n\n",
    ));
    assert!(
        t.starts_with(&format!(
            "{MAIL_TOP}Subject: [PATCH] second subject\nfirst subject\nThird line.\n"
        )),
        "{t:?}"
    );
    let t = diff_shown(&mail(
        "From: B Person <b@example.com>\nSubject: one\nSubject: two\nSubject: three\n\n",
    ));
    assert!(
        t.starts_with(&format!(
            "{MAIL_TOP}From: B Person <b@example.com>\nSubject: two\nSubject: three\none\nThird line.\n"
        )),
        "{t:?}"
    );
}

/// The header block ends at its blank line, or failing one at the first
/// line that is neither a header nor the continuation of one: what is
/// under it is the message. Read as headers, those lines were flowed into
/// one paragraph with the author and date, above the subject.
#[test]
fn mail_headers_end_at_the_first_line_that_is_not_one() {
    let t = diff_shown(&mail(
        "Subject: [PATCH] the subject\nbody starts with no blank line\nsecond body line\n\n",
    ));
    assert!(
        t.starts_with(&format!(
            "{MAIL_TOP}the subject\nbody starts with no blank line second body line\nThird line.\n"
        )),
        "{t:?}"
    );
    // A folded header is still one header, and a line shaped like a header
    // under the blank line is still the message.
    let t = diff_shown(&mail(
        "Subject: [PATCH] a subject\n folded onto a second line\n\nSubject: not a header\nFrom: nobody\n\n",
    ));
    assert!(
        t.starts_with(&format!(
            "{MAIL_TOP}a subject folded onto a second line\nSubject: not a header From: nobody\nThird line.\n"
        )),
        "{t:?}"
    );
    // A `---` straight under the headers still opens the diffstat.
    let src = "From 0123456789012345678901234567890123456789 Mon Sep 17 00:00:00 2001\nFrom: A <a@example.com>\nSubject: s\nbody\n---\n f | 2 +-\n 1 file changed, 1 insertion(+), 1 deletion(-)\n\ndiff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1 +1 @@\n-a\n+b\n";
    assert_eq!(
        fences(src)[0],
        "---\n f | 2 +-\n 1 file changed, 1 insertion(+), 1 deletion(-)\n"
    );
    let t = diff_shown(src);
    assert!(
        t.starts_with("commit 012345678901\nAuthor: A <a@example.com>\ns\nbody\n"),
        "{t:?}"
    );
}

/// One commit of `git log`, its message, and `tail` under it as git
/// writes what follows a message.
fn log_with(tail: &str) -> String {
    format!(
        "commit 0123456789012345678901234567890123456789\nAuthor: A Person <a@example.com>\nDate:   Mon Jan 1 00:00:00 2024 +0000\n\n    Fix the thing\n\n    A body of *two*\n    lines.\n\n    Signed-off-by: A Person <a@example.com>\n{tail}\ncommit 1123456789012345678901234567890123456789\nAuthor: A Person <a@example.com>\n\n    Second\n"
    )
}

/// `git log --stat` and its relatives write a block under each message:
/// it keeps its lines and columns in a fence, as a mail's diffstat does.
/// It was the message's last paragraph — flowed into a run of words, the
/// trailers above it no longer the message's closing lines, and (where the
/// block is unindented) every line of the message a record.
#[test]
fn a_logs_diffstat_keeps_its_lines() {
    let message = "Fix the thing\nA body of two lines.\nSigned-off-by: A Person <a@example.com>\n";
    // (The reader shows a tab as the columns it reaches.)
    for (tail, fence) in [
        (
            "\n src/f.txt    | 2 +-\n src/other.rs | 9 +++++----\n 2 files changed, 6 insertions(+), 5 deletions(-)\n",
            " src/f.txt    | 2 +-\n src/other.rs | 9 +++++----\n 2 files changed, 6 insertions(+), 5 deletions(-)\n",
        ),
        // `-p --stat`: a `---` straight under the message, then the block.
        (
            "---\n src/f.txt | 2 +-\n 1 file changed, 1 insertion(+), 1 deletion(-)\n\ndiff --git a/src/f.txt b/src/f.txt\n--- a/src/f.txt\n+++ b/src/f.txt\n@@ -1 +1 @@\n-a\n+b\n",
            "---\n src/f.txt | 2 +-\n 1 file changed, 1 insertion(+), 1 deletion(-)\n",
        ),
        (
            "\n 1 file changed, 1 insertion(+)\n",
            " 1 file changed, 1 insertion(+)\n",
        ),
        (
            "\n 3 files changed, 2 insertions(+), 1 deletion(-)\n create mode 100644 a b.txt\n rename x => y (100%)\n",
            " 3 files changed, 2 insertions(+), 1 deletion(-)\n create mode 100644 a b.txt\n rename x => y (100%)\n",
        ),
        (
            "\n img.png | Bin 0 -> 120 bytes\n s.txt   |   0\n",
            " img.png | Bin 0 -> 120 bytes\n s.txt   |   0\n",
        ),
        (
            "\n6\t5\tsrc/f.txt\n-\t-\timg.png\n",
            "6   5   src/f.txt\n-   -   img.png\n",
        ),
        ("\n1\t1\tsrc/f.txt\n", "1   1   src/f.txt\n"),
        (
            "\nM\tsrc/f.txt\nR100\told name\tnew name\n",
            "M   src/f.txt\nR100    old name    new name\n",
        ),
        ("\nA\tsrc/f.txt\n", "A   src/f.txt\n"),
        ("\nsrc/f.txt\nMakefile\n", "src/f.txt\nMakefile\n"),
        (
            "\n:100644 100644 ffb6e80 911aaa9 M\tCHANGELOG.md\n  31.7% crates/carrel/src/\n",
            ":100644 100644 ffb6e80 911aaa9 M    CHANGELOG.md\n  31.7% crates/carrel/src/\n",
        ),
    ] {
        let src = log_with(tail);
        let found = fences(&src);
        assert_eq!(found.first().map(String::as_str), Some(fence), "{tail:?}");
        let t = diff_shown(&src);
        assert!(
            t.starts_with(&format!(
                "commit 012345678901\nAuthor: A Person <a@example.com>\nDate:   Mon Jan 1 00:00:00 2024 +0000\n{message}{fence}"
            )),
            "{tail:?}: {t:?}"
        );
        assert_eq!(headings(&src)[0], "commit 012345678901");
        assert!(headings(&src).contains(&"commit 112345678901".to_string()));
    }
}

/// Only a block git wrote is fenced: a message's own closing paragraph is
/// prose whatever it holds, and so is text around a bare diff that is not
/// every line a diffstat's.
#[test]
fn message_prose_is_never_a_diffstat() {
    // In the message (indented as git indents it): a pasted stat, a table
    // row, a status letter, a path.
    for msg in [
        "Subject\n\n f | 2 +-\n 1 file changed, 1 insertion(+), 1 deletion(-)",
        "Subject\n\nM\tsrc/f.txt\nA\tsrc/g.txt",
        "Subject\n\n1\t2\tsrc/f.txt\n3\t4\tsrc/g.txt",
        "Subject\n\nsrc/f.txt\nsrc/g.txt",
        "Subject\n\nbody\n---\n f | 2 +-",
    ] {
        assert_eq!(fences(&commit(msg)), Vec::<String>::new(), "{msg:?}");
    }
    // Under the message, unindented, but not a stat block.
    for tail in [
        "\nsome words here\n",
        "\n f | 2 +-\nand a sentence\n",
        "\n a | b\n c | d\n",
        "\n 2 files changed my mind\n",
        "\nNotes:\n    a note\n",
        "---\n",
        "---\nnot a stat\n",
        "\nM src/f.txt\n",
    ] {
        assert_eq!(fences(&log_with(tail)), Vec::<String>::new(), "{tail:?}");
    }
    // With no indented message above it, one shaped line is not enough.
    for src in [
        "commit 0123456789012345678901234567890123456789\nAuthor: A\n\nM\tf\n",
        "commit 0123456789012345678901234567890123456789\nAuthor: A\n\nsrc/f.txt\nsrc/g.txt\n",
        "commit 0123456789012345678901234567890123456789\nAuthor: A\n\n f | 2 +-\n",
    ] {
        assert_eq!(fences(src), Vec::<String>::new(), "{src:?}");
    }
}
