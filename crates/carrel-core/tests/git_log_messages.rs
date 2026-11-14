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
