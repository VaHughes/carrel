//! Frontmatter: the metadata card, and its plain-text degradation.
//!
//! A file opening with `---\ntitle: x\n---` used to render as an H2 full of
//! YAML — the first thing on screen for every Obsidian, Hugo, Jekyll, Zola and
//! Quartz document. These tests pin the fix.

use carrel::app::App;
use carrel_core::Document;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

fn app(src: &str, cols: u16, rows: u16) -> App {
    App::new("t.md".into(), Document::parse(src), cols, rows)
}

fn frame(src: &str, cols: u16, rows: u16) -> String {
    let a = app(src, cols, rows);
    let mut t = Terminal::new(TestBackend::new(cols, rows)).expect("test backend");
    t.draw(|f| carrel::render::draw(f, &a)).expect("draw");
    t.backend()
        .buffer()
        .content()
        .chunks(cols as usize)
        .map(|row| {
            row.iter()
                .map(ratatui::buffer::Cell::symbol)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The real `--plain` contract (Q17): no escape bytes and no box drawing.
///
/// NOT "pure ASCII" — space 2 legitimately holds non-ASCII content, and has
/// since smart punctuation shipped: `"x"` is already U+201C/U+201D. The policy
/// is about what plain output ADDS, not what the document contains.
fn assert_plain_safe(out: &str) {
    assert!(
        !out.contains('\u{1b}'),
        "plain output never contains an escape byte:\n{out}"
    );
    assert!(
        !out.chars().any(|c| ('\u{2500}'..='\u{257f}').contains(&c)),
        "plain output never contains box drawing:\n{out}"
    );
}

#[test]
fn the_frontmatter_card_shows_keys_aligned_in_a_gutter() {
    let out = frame(
        "---\ntitle: My Note\ntags: a, b\n---\n\n# Heading\n",
        40,
        10,
    );
    assert!(out.contains("╭"), "the card opens:\n{out}");
    assert!(out.contains("╰"), "the card closes:\n{out}");
    assert!(out.contains("title"), "keys are painted:\n{out}");
    assert!(out.contains("My Note"), "values are painted:\n{out}");
    // The keys align: `title` and `tags` both start at the same column, and
    // both values start at the same column after the padded key gutter.
    let title_row = out
        .lines()
        .find(|l| l.contains("title"))
        .expect("title row");
    let tags_row = out.lines().find(|l| l.contains("tags")).expect("tags row");
    assert_eq!(
        title_row.find("My Note"),
        tags_row.find("a, b"),
        "values line up in one column:\n{out}"
    );
}

#[test]
fn the_frontmatter_delimiters_are_not_rendered() {
    let out = frame("---\ntitle: My Note\n---\n\nbody\n", 40, 10);
    assert!(
        !out.contains("---"),
        "the `---` fences are structure, not content:\n{out}"
    );
}

#[test]
fn an_unsplittable_line_renders_raw_under_its_parent() {
    let out = frame("---\ntags:\n  - a\n  - b\n---\n\ntext\n", 40, 10);
    assert!(out.contains("- a"), "list items survive verbatim:\n{out}");
    assert!(out.contains("- b"), "every one of them:\n{out}");
}

#[test]
fn plain_mode_renders_frontmatter_as_ascii_key_value_lines() {
    let doc = Document::parse("---\ntitle: My Note\n---\n\ntext\n");
    let out = carrel::plain::render(&doc, 72);
    assert!(
        out.contains("title: My Note"),
        "keys and values survive:\n{out}"
    );
    assert_plain_safe(&out);
}

// --- the card is a function of the layout that counts its rows ---

/// Long values, a flow list of tags, a key wider than the cap, CJK, a URL
/// too long for any narrow row, and a nested list.
const CARD: &str = "---\n\
title: A rather long title that will not fit on one narrow row at all\n\
tags: [alpha, beta, gamma, delta, epsilon, zeta, eta, theta, iota, kappa]\n\
a_key_wider_than_the_cap: value\n\
summary: 日本語の文章がここにあります、とても長いです\n\
url: https://example.com/a/very/long/path/that/cannot/break/anywhere/at/all\n\
aliases:\n  - one\n  - two\n\
---\n\n# Heading\n\nbody\n";

const TAGS: [&str; 10] = [
    "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "iota", "kappa",
];

const WIDTHS: [u16; 10] = [100, 80, 60, 50, 40, 34, 28, 24, 20, 16];

fn sized(src: &str, cols: u16, rows: u16) -> App {
    let mut a = app(src, cols, rows);
    a.breadcrumb = false;
    a.on_resize(cols, rows);
    a
}

fn paint(a: &App, cols: u16, rows: u16) -> (ratatui::buffer::Buffer, carrel::render::Painted) {
    let mut t = Terminal::new(TestBackend::new(cols, rows)).expect("test backend");
    let mut painted = carrel::render::Painted::default();
    t.draw(|f| {
        carrel::render::draw_full(f, a, &mut painted, &mut std::collections::HashMap::new());
    })
    .expect("draw");
    (t.backend().buffer().clone(), painted)
}

fn row_text(buf: &ratatui::buffer::Buffer, cols: u16, y: u16) -> String {
    (0..cols).map(|x| buf[(x, y)].symbol()).collect()
}

/// The rule that outranks everything, on the card: a cell shows exactly the
/// byte a click on it resolves to. The card used to paint each source line
/// clipped, with its value shifted right by padding that was not in the text
/// — so a click on a value landed on a different character, and every row
/// the layout wrapped onto was painted blank.
#[test]
fn every_cell_of_the_card_is_the_byte_a_click_on_it_resolves_to() {
    for cols in WIDTHS {
        let a = sized(CARD, cols, 60);
        let (buf, _) = paint(&a, cols, 60);
        let card_rows = a.layout.content_height(&a.doc, carrel_core::BlockIdx(0));
        let mut checked = 0;
        for y in a.text_y()..a.text_y() + u16::try_from(card_rows).unwrap() {
            for x in 0..cols {
                let Some((start, end)) = a.doc_span_at(x, y) else {
                    continue;
                };
                let painted = buf[(x, y)].symbol();
                if painted == " " || painted.is_empty() {
                    continue;
                }
                let byte = &a.doc.text[start as usize..end as usize];
                assert_eq!(
                    byte,
                    painted,
                    "cols={cols} ({x},{y}): a click resolves to {byte:?} but the \
                     cell paints {painted:?}\n{}",
                    row_text(&buf, cols, y)
                );
                checked += 1;
            }
        }
        assert!(
            checked > 50,
            "cols={cols}: checked almost nothing ({checked})"
        );
    }
}

/// Every tag is painted, whole, under a button that covers exactly it — at
/// every width, including the ones where the tags line wraps.
#[test]
fn every_tag_is_a_button_at_every_width() {
    for cols in WIDTHS {
        let a = sized(CARD, cols, 60);
        let (buf, painted) = paint(&a, cols, 60);
        let mut words: Vec<String> = painted
            .targets
            .as_slice()
            .iter()
            .filter(|t| matches!(t.action, carrel::action::Action::TagOpen(_)))
            .map(|t| {
                (t.zone.x..t.zone.x + t.zone.w)
                    .map(|x| buf[(x, t.zone.y)].symbol())
                    .collect()
            })
            .collect();
        words.sort();
        let mut want: Vec<String> = TAGS.iter().map(|s| (*s).to_string()).collect();
        want.sort();
        assert_eq!(words, want, "cols={cols}: every tag, each on its own word");
    }
}

/// The card's rows are its painted rows: no blank run after it where the
/// layout counted wrapped rows that were never drawn, and the rule runs
/// unbroken from `╭` to `╰`.
#[test]
fn the_card_is_followed_by_one_gap_and_then_the_document() {
    for cols in WIDTHS {
        let a = sized(CARD, cols, 80);
        let (buf, _) = paint(&a, cols, 80);
        let rows: Vec<String> = (0..80).map(|y| row_text(&buf, cols, y)).collect();
        let close = rows
            .iter()
            .position(|r| r.contains('╰'))
            .unwrap_or_else(|| panic!("cols={cols}: the card closes\n{}", rows.join("\n")));
        let heading = rows
            .iter()
            .position(|r| r.contains("Heading"))
            .expect("the heading");
        assert_eq!(
            heading,
            close + 2,
            "cols={cols}: one spacing row between card and heading\n{}",
            rows.join("\n")
        );
        let open = rows.iter().position(|r| r.contains('╭')).expect("opens");
        for (y, r) in rows.iter().enumerate().take(close).skip(open + 1) {
            assert!(r.contains('│'), "cols={cols}: row {y} lost its rule: {r:?}");
        }
    }
}

/// The rule's glyph on each of the card's rows, top to bottom.
fn rule_glyphs(src: &str, cols: u16) -> String {
    let a = sized(src, cols, 40);
    let (buf, _) = paint(&a, cols, 40);
    let card_rows = a.layout.content_height(&a.doc, carrel_core::BlockIdx(0));
    (a.text_y()..a.text_y() + u16::try_from(card_rows).unwrap())
        .map(|y| {
            row_text(&buf, cols, y)
                .chars()
                .find(|c| "╭│╰╶".contains(*c))
                .unwrap_or('?')
        })
        .collect()
}

/// Exactly one close, on the card's LAST row — a blank or whitespace-only
/// line at the end of the body is a row of the card like any other, and
/// every row from the last non-blank one on used to read as "last". A card
/// of one row has no top and no bottom, so it takes the rule's one-row form.
#[test]
fn the_card_rule_closes_once_on_its_last_row() {
    for (src, want) in [
        ("---\nk: v\n\n# c\nkk: w\n   \n---\n\nbody\n", "╭│││╰"),
        ("---\nk: v\nkk: w\n\n---\n\nbody\n", "╭│╰"),
        ("---\nk: v\nkk: w\n---\n\nbody\n", "╭╰"),
        ("---\nk: v\n---\n\nbody\n", "╶"),
    ] {
        assert_eq!(rule_glyphs(src, 40), want, "{src:?}");
    }
}

/// Keys wear their weight on their own cells, found through the same row
/// geometry as everything else: not the value beside them, and not the rows
/// a wrapped value continues on.
#[test]
fn only_the_key_is_bold() {
    use ratatui::style::Modifier;
    for cols in [80u16, 40, 20] {
        let a = sized(CARD, cols, 60);
        let (buf, _) = paint(&a, cols, 60);
        // A key too long for a narrow row wraps, and stays bold on both
        // rows, so the rows are joined with a `|` that only a key's own
        // continuation may leave inside a name.
        let bold: Vec<String> = (0..60u16)
            .map(|y| {
                (0..cols)
                    .filter(|&x| buf[(x, y)].modifier.contains(Modifier::BOLD))
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .filter(|s| !s.trim().is_empty())
            .collect();
        let joined = bold.join("|").replace("tha|n", "than");
        assert!(
            joined.starts_with("title|tags|a_key_wider_than_the_cap|summary|url|aliases|"),
            "cols={cols}: {joined:?}"
        );
    }
}

/// Search paints over the card like over any other block. The old card
/// painter returned before highlights, selection or saved notes ran.
#[test]
fn a_search_hit_in_the_card_is_highlighted_on_its_word() {
    for cols in [80u16, 24] {
        let mut a = sized(CARD, cols, 60);
        a.matches = Some(carrel_core::search(&a.doc, "epsilon", true));
        let (buf, _) = paint(&a, cols, 60);
        let lit = [
            carrel::theme::match_normal().bg,
            carrel::theme::match_current().bg,
        ];
        let words: String = (0..60u16)
            .flat_map(|y| (0..cols).map(move |x| (x, y)))
            .filter(|&(x, y)| lit.contains(&Some(buf[(x, y)].bg)))
            .map(|(x, y)| buf[(x, y)].symbol().to_string())
            .collect();
        assert_eq!(words, "epsilon", "cols={cols}");
    }
}

/// The value column is padding in the text, and a reader types what the
/// source says: `title: A` must still find `title:   A`, the way a phrase
/// finds itself across a table's padded cells.
#[test]
fn a_key_and_its_value_are_found_as_the_source_writes_them() {
    let doc = Document::parse(CARD);
    let hits = carrel_core::search(&doc, "title: A rather", true);
    assert_eq!(hits.ranges.len(), 1);
}
