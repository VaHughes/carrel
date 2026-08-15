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
