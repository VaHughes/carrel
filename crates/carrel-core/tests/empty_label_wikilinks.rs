//! An empty-label wikilink (`[[x|]]`) and the pulldown-cmark 0.13.4 bug it
//! trips: the rest of the paragraph replayed inside a bogus link and again
//! after it. The oracle throughout is the same text with the wikilink
//! escaped (`\[\[x|\]\]`), which the parser reads correctly.

use carrel_core::Document;

fn links(d: &Document) -> Vec<&str> {
    d.links.iter().map(|l| &**l).collect()
}

/// The styled runs, with adjacent runs of one style merged: how the text
/// splits into runs is not what this is about.
fn styles(d: &Document) -> Vec<(u32, u32, u8, bool)> {
    let mut out: Vec<(u32, u32, u8, bool)> = Vec::new();
    for i in d.nodes.iter().flat_map(|n| n.inlines.iter()) {
        let run = (i.doc.start, i.doc.end, i.style.0, i.link.is_some());
        match out.last_mut() {
            Some(l) if l.1 == run.0 && (l.2, l.3) == (run.2, run.3) => l.1 = run.1,
            _ => out.push(run),
        }
    }
    out
}

/// `src` with every empty-label wikilink the parser reads escaped
/// (`\[\[x|\]\]`), the form it reads correctly — found by parsing, so a
/// `[[x|]]` in code, math or HTML, which is no link, is left as it is.
/// Repeated, since escaping one can uncover another the replay hid.
/// Whether an escaped empty-label wikilink (`\[\[x|\]\]`) in `s` sits inside
/// a wikilink the parser reads.
fn inside_a_wikilink(s: &str) -> bool {
    use pulldown_cmark::{Event, LinkType, Options, Parser, Tag};
    let options = Options::ENABLE_WIKILINKS | Options::ENABLE_GFM;
    Parser::new_ext(s, options)
        .into_offset_iter()
        .any(|(ev, r)| {
            matches!(
                ev,
                Event::Start(Tag::Link {
                    link_type: LinkType::WikiLink { .. },
                    ..
                })
            ) && s[r].contains("\\[\\[")
        })
}

fn escaped(src: &str) -> String {
    use pulldown_cmark::{Event, LinkType, Options, Parser, Tag};
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_GFM
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_SMART_PUNCTUATION
        | Options::ENABLE_WIKILINKS
        | Options::ENABLE_MATH
        | Options::ENABLE_DEFINITION_LIST
        | Options::ENABLE_SUPERSCRIPT
        | Options::ENABLE_SUBSCRIPT;
    let mut s = src.to_owned();
    for _ in 0..8 {
        let mut found: Vec<std::ops::Range<usize>> = Parser::new_ext(&s, options)
            .into_offset_iter()
            .filter(|(ev, r)| {
                matches!(
                    ev,
                    Event::Start(Tag::Link {
                        link_type: LinkType::WikiLink { has_pothole: true },
                        ..
                    })
                ) && s[r.clone()].ends_with("|]]")
                    && s[r.clone()].find('|') == Some(r.len() - 3)
            })
            .map(|(_, r)| r)
            .collect();
        if found.is_empty() {
            break;
        }
        found.sort_by_key(|r| std::cmp::Reverse(r.start));
        found.dedup();
        for r in found {
            s.replace_range(r.end - 2..r.end, "\\]\\]");
            s.replace_range(r.start..r.start + 2, "\\[\\[");
        }
    }
    s
}

/// A label that ENDS in `|` is not empty, and pulldown-cmark parses it
/// with no replay. A guard keyed on `|]]` turned it into literal text and
/// lost its link.
#[test]
fn a_label_ending_in_a_pipe_is_still_a_wikilink() {
    for (src, text, link) in [
        ("see [[notes|a|]] here\n", "see a| here", "notes"),
        ("pipe [[x||]] here\n", "pipe | here", "x"),
        ("[[notes|label *a*|]]\n", "label a|", "notes"),
        ("[[a|\\|]] b\n", "| b", "a"),
    ] {
        let d = Document::parse(src);
        assert_eq!(links(&d), [link], "{src:?}: {:?}", d.text);
        assert!(d.text.contains(text), "{src:?}: {:?}", d.text);
    }
}

/// Wrapped in emphasis, the bogus link holds the emphasis's CLOSER as
/// text, then the rest of the paragraph, so the style ran to its end.
#[test]
fn an_empty_label_wikilink_in_emphasis_closes_the_emphasis() {
    for src in [
        "see *[[x|]]* now\n",
        "a **[[x|]]** b\n",
        "a ~~[[x|]]~~ b\n",
        "_[[x|]]_\n",
        "***[[x|]]*** z [[y]] w\n",
        "*[[x|]] a* b\n",
    ] {
        let a = Document::parse(src);
        let b = Document::parse(&escaped(src));
        assert_eq!(a.text, b.text, "{src:?}");
        assert_eq!(styles(&a), styles(&b), "{src:?}");
        assert_eq!(links(&a), links(&b), "{src:?}");
        assert!(!a.text.contains(['*', '~', '_']), "{src:?}: {:?}", a.text);
    }
}

/// The replay can open a container with no leaf in it — an empty link
/// text, an empty image alt — and that slipped through as a phantom link.
#[test]
fn no_phantom_link_from_the_replay() {
    let d = Document::parse("[[x|]] a [](http://u)\n");
    assert_eq!(links(&d), ["http://u"]);
    let d = Document::parse("[[x|]] a ![](i.png) b\n");
    assert_eq!(d.text.matches(" b").count(), 1, "{:?}", d.text);
}

/// The same with a dash typed elsewhere in the paragraph: one `—` in the
/// sentence used to turn the repair off for every unpaired URL in it. A
/// dash typed in the URL itself is still the author's.
#[test]
fn an_unpaired_bare_url_keeps_its_hyphens_beside_a_typed_dash() {
    for dash in ['\u{2014}', '\u{2013}'] {
        let d = Document::parse(&format!(
            "see https://x.example/a--b?c&amp;d ok {dash} done\n"
        ));
        assert_eq!(&*d.links[0], "https://x.example/a--b?c&d", "{dash}");
        assert!(
            d.text
                .contains(&format!("see https://x.example/a--b?c&d ok {dash} done")),
            "{:?}",
            d.text
        );
    }
    let d = Document::parse(
        "x \u{2014} https://x.example/a---b--c?d&amp;e and www.q\u{2013}r.example/z&amp;y\n",
    );
    assert_eq!(
        d.links.iter().map(|l| &**l).collect::<Vec<_>>(),
        [
            "https://x.example/a---b--c?d&e",
            "http://www.q\u{2013}r.example/z&y"
        ]
    );
    assert!(
        d.text
            .contains("https://x.example/a---b--c?d&e and www.q\u{2013}r.example/z&y"),
        "{:?}",
        d.text
    );
}

/// A bare URL that cannot be paired with the source (an entity in it)
/// still showed AND copied an en dash for the `--` typed in it.
#[test]
fn an_unpaired_bare_url_copies_its_hyphens() {
    let d = Document::parse("see https://x.example/a--b?c&amp;d ok\n");
    assert_eq!(&*d.links[0], "https://x.example/a--b?c&d");
    assert!(
        d.text.contains("https://x.example/a--b?c&d ok"),
        "{:?}",
        d.text
    );
    let d = Document::parse("see https://x.example/a---b?c&amp;d ok\n");
    assert_eq!(&*d.links[0], "https://x.example/a---b?c&d");
    // Curly quotes stay curly, as in a paired URL.
    let d = Document::parse("see https://x.example/it's?c&amp;d ok\n");
    assert_eq!(&*d.links[0], "https://x.example/it\u{2019}s?c&d");
}

/// Inside a code span or math nothing is a wikilink, so nothing changes.
#[test]
fn code_and_math_keep_the_wikilink_as_typed() {
    for (src, text) in [
        ("a `[[x|]]` b [[y]]\n", "a [[x|]] b y"),
        ("a $[[x|]]$ b\n", "b"),
        ("```\n[[x|]] abc\n```\n", "[[x|]] abc"),
    ] {
        let d = Document::parse(src);
        assert!(d.text.contains(text), "{src:?}: {:?}", d.text);
        assert_eq!(
            d.text.matches(" b").count(),
            usize::from(src.contains(" b")),
            "{src:?}"
        );
    }
}

#[test]
fn a_word_typed_around_it_reads_as_one() {
    // A URL typed on into it links what was typed, as any `[` would.
    let d = Document::parse("https://a.b/c[[x|]] z\n");
    assert_eq!(d.text, "https://a.b/c[[x|]] z\n");
    assert_eq!(links(&d), ["https://a.b/c[[x|]]"]);
    // An attached script takes it whole, as it takes `H~[2]~O`.
    let d = Document::parse("a~[[x|]]~ z\n");
    assert_eq!(d.text, "a[[x|]] z\n");
}

/// A `'` after the link is an apostrophe, as after any `]`.
#[test]
fn a_quote_after_it_reads_as_after_a_bracket() {
    for src in ["it [[x|]]'s\n", "'a [[x|]]'b'\n", "[[x|]]''\n"] {
        let a = Document::parse(src);
        assert_eq!(a.text, Document::parse(&escaped(src)).text, "{src:?}");
    }
    assert_eq!(Document::parse("[[x|]]'s\n").text, "[[x|]]\u{2019}s\n");
}

#[test]
fn search_finds_guarded_text_and_dash_urls() {
    for src in [
        "q [[x|]] abc and https://x.example/a--b ok\n",
        "q [[x|]] abc and https://x.example/a--b?c&amp;d ok\n",
    ] {
        let d = Document::parse(src);
        assert_eq!(carrel_core::search(&d, "[[x|]] abc", false).len(), 1);
        assert_eq!(carrel_core::search(&d, "a\u{2013}b", false).len(), 1);
        assert_eq!(carrel_core::search(&d, "a--b", false).len(), 1);
    }
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(20000))]
    /// Every document with `[[x|]]` reads as the same text escaped.
    #[test]
    fn reads_as_the_escaped_form(
        parts in proptest::collection::vec(proptest::sample::select(&[
            "a", " ", "[[x|]]", "[[y]]", "*", "**", "_", "\n", "\n\n", "- ", "> ", "# ",
            "[t](u)", "![](i)", "[^1]", "\n\n[^1]: ", "https://a.b/c", "&amp;", "<b>", "~~",
            "`", "$m$", "$", "'", "\"", "--", "...", "<!--", "-->", "\\",
            "[[n|a|]]", "|", "^", "~", "[", "]", "| a |\n|---|\n",
        ][..]), 0..14)
    ) {
        let src = parts.concat();
        // A word holding the wikilink is one text run here and several in
        // the oracle, whose escapes split it, and the reader's own passes
        // over a run — a bare URL cut at its edge, a `~x~` or `^x^` script
        // — read a split word differently: the two differ for reasons of
        // their own. `a_word_typed_around_it_reads_as_one` covers those.
        let glued = src
            .split(|c: char| c.is_whitespace() || c == '<')
            .any(|w| w.contains("[[x|]]") && (w.contains("https://") || w.contains(['~', '^'])));
        if glued {
            return Ok(());
        }
        // Inside another wikilink the oracle is false: a wikilink's target
        // is raw, so the oracle's own escapes show there as backslashes
        // (CI drew `[[[[x|]]a]]`). Nested wikilinks are not markup anyone
        // writes on purpose; neither reading is promised.
        if inside_a_wikilink(&escaped(&src)) {
            return Ok(());
        }
        let a = Document::parse(&src);
        let b = Document::parse(&escaped(&src));
        proptest::prop_assert_eq!(&a.text, &b.text, "src {:?}", src);
        proptest::prop_assert_eq!(styles(&a), styles(&b), "src {:?}", src);
        proptest::prop_assert_eq!(links(&a), links(&b), "src {:?}", src);
    }
}

/// The property's one skip is exactly the nested case, never the plain one.
#[test]
fn the_oracle_skips_only_a_wikilink_inside_a_wikilink() {
    assert!(inside_a_wikilink(&escaped("[[[[x|]]a]]")));
    assert!(!inside_a_wikilink(&escaped("see [[x|]] here [[y]]")));
    assert!(!inside_a_wikilink(&escaped("*[[x|]]* now")));
}
