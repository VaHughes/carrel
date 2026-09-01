//! The Q16 conformance suite: what carrel does with every markdown construct
//! it is asked about — **including the ones it deliberately does not support**.
//!
//! Q16 stood at ◐ from 2026-08-11 to 2026-08-15 because "all of markdown" was
//! never a defined set. This file is the definition, and it is executable: a
//! construct that starts or stops working changes a result here.
//!
//! The "will not support" cases are asserted just as firmly as the supported
//! ones. Rendering `==highlight==` as literal text is a decision, not an
//! oversight, and a test is how a decision stays one.

use carrel_core::Document;

const CORPUS: &str = include_str!("corpus/conformance.md");

fn doc() -> Document {
    Document::parse(CORPUS)
}

fn plain() -> String {
    carrel::plain::render(&doc(), 72)
}

fn kinds() -> Vec<String> {
    let d = doc();
    (0..d.block_count())
        .map(|i| {
            format!(
                "{:?}",
                d.node_for_block(carrel_core::BlockIdx(i as u32)).kind
            )
        })
        .collect()
}

fn has_kind(prefix: &str) -> bool {
    kinds().iter().any(|k| k.starts_with(prefix))
}

// --- supported: structure ---

#[test]
fn frontmatter_is_a_metadata_block_and_its_fences_are_gone() {
    assert!(has_kind("Metadata"), "kinds: {:?}", kinds());
    let out = plain();
    assert!(out.contains("title: Conformance"), "{out}");
    assert!(
        out.contains("- one"),
        "a nested list line survives raw:\n{out}"
    );
    assert!(
        !out.contains("\n---\ntitle"),
        "the fences are structure, not content:\n{out}"
    );
}

#[test]
fn the_commonmark_block_set_is_present() {
    for kind in ["Heading", "Paragraph", "CodeBlock", "Rule", "Item"] {
        assert!(has_kind(kind), "missing {kind} in {:?}", kinds());
    }
    // A block quote is a CONTAINER, so it never appears in `layout_order` --
    // the depth rides on the enclosed paragraph instead, which is why
    // `quote_depth` is a field rather than an ancestor walk.
    let d = doc();
    assert!(
        (0..d.block_count()).any(|i| d
            .node_for_block(carrel_core::BlockIdx(i as u32))
            .quote_depth
            >= 2),
        "the nested quote reaches depth 2"
    );
}

#[test]
fn gfm_tables_alerts_tasks_and_footnotes_are_present() {
    assert!(has_kind("Table"), "kinds: {:?}", kinds());
    assert!(has_kind("AlertLabel"), "GFM alert label: {:?}", kinds());
    let out = plain();
    assert!(out.contains("strikethrough"), "{out}");
    assert!(out.contains("a done task"), "{out}");
    assert!(out.contains("The footnote definition"), "{out}");
}

#[test]
fn definition_lists_render_as_term_and_details() {
    assert!(has_kind("DefTerm"), "kinds: {:?}", kinds());
    assert!(has_kind("DefDetails"), "kinds: {:?}", kinds());
    assert!(
        !plain().contains(": The definition"),
        "the `:` marker is consumed, not rendered as prose"
    );
}

// --- supported: inline ---

#[test]
fn autolinks_resolve_in_both_forms() {
    let d = doc();
    let urls: Vec<&str> = d.links.iter().map(|u| &**u).collect();
    assert!(
        urls.contains(&"https://example.test"),
        "angle-bracket autolink: {urls:?}"
    );
    assert!(
        urls.contains(&"http://www.example.com"),
        "bare www autolink, http per GFM: {urls:?}"
    );
    // The comma legitimately follows the link in the rendered prose; what
    // matters is that it is outside the LINK RUN, which the core test
    // `bare_www_becomes_a_link_and_trailing_punctuation_stays_out_of_it`
    // pins directly against the inline ranges.
    assert!(plain().contains("www.example.com, with"), "{}", plain());
}

#[test]
fn word_boundary_scripts_consume_their_markers() {
    let out = plain();
    assert!(out.contains("x 2 and log 2 n"), "markers consumed:\n{out}");
}

#[test]
fn inline_math_enters_the_display_text_already_rendered() {
    let out = plain();
    assert!(
        out.contains("E = mc\u{b2}"),
        "the Unicode form is what the reader sees, and what search matches:\n{out}"
    );
    assert!(
        out.contains('\u{3b1}') && out.contains('\u{2265}'),
        "\\alpha and \\ge resolve:\n{out}"
    );
    assert!(!out.contains("$E ="), "the delimiters are consumed:\n{out}");
}

#[test]
fn display_math_is_its_own_block_holding_the_latex_source() {
    assert!(has_kind("Math"), "kinds: {:?}", kinds());
    let out = plain();
    assert!(
        out.contains("\\frac{a+b}{c}"),
        "plain mode emits speakable source, not a box rule (Q17):\n{out}"
    );
    assert_eq!(
        carrel_core::search(&doc(), "frac", true).len(),
        1,
        "display math source is searchable"
    );
}

#[test]
fn a_wikilink_resolves_to_a_link_not_literal_brackets() {
    let out = plain();
    assert!(out.contains("other-note"), "{out}");
    assert!(!out.contains("[[other-note]]"), "brackets consumed:\n{out}");
}

// --- deliberately NOT supported ---
//
// Each of these renders as literal text. That is the documented decision, with
// its reason recorded in the design spec's "will not support" table. These
// assertions exist so the decision cannot be reversed by accident.

#[test]
fn highlight_syntax_stays_literal() {
    // Neither CommonMark nor GFM; no upstream parser support.
    assert!(plain().contains("==highlight=="), "{}", plain());
}

#[test]
fn emoji_shortcodes_stay_literal() {
    // Not CommonMark; would need a ~2,000-entry table or a new dependency.
    assert!(plain().contains(":smile:"), "{}", plain());
}

/// **A real interaction between two extensions, found by this suite.**
///
/// `:::` directives are not supported — but they do not survive as literal
/// text either, because enabling `ENABLE_DEFINITION_LIST` makes a line
/// beginning with `:` a definition marker. `::: note` therefore parses as a
/// definition whose body is `:: note`.
///
/// This is the honest consequence of supporting definition lists, and it is
/// recorded rather than papered over. Anyone wanting literal `:::` blocks has
/// to choose between the two syntaxes; carrel chose definition lists, which
/// are CommonMark-adjacent and far more widely used.
#[test]
fn directive_blocks_are_eaten_by_the_definition_list_extension() {
    let out = plain();
    assert!(
        !out.contains(":::"),
        "the marker is consumed by the definition-list parser:\n{out}"
    );
    assert!(
        out.contains("note") && out.contains("A directive"),
        "no text is lost, only the marker:\n{out}"
    );
}

#[test]
fn attached_scripts_render_even_though_upstream_declines_them() {
    // pulldown-cmark 0.13.4 only opens ^…^ and ~…~ at a word boundary, so
    // `x^2^` and `H~2~O` -- the forms people actually write -- reach us as
    // literal text. carrel closes that gap itself (`attached_scripts`), so
    // the markers are consumed here exactly as the parsed form's are.
    let out = plain();
    assert!(out.contains("x2"), "superscript not applied: {out}");
    assert!(out.contains("H2O"), "subscript not applied: {out}");
    assert!(!out.contains("x^2^"), "markers survived: {out}");
    assert!(!out.contains("H~2~O"), "markers survived: {out}");
}

/// The other half of the same feature: what must NOT be read as a script.
#[test]
fn the_attached_script_scan_leaves_paths_urls_and_strikethrough_alone() {
    let out = plain();
    assert!(out.contains("struck"), "strikethrough still renders: {out}");
    assert!(!out.contains("~~"), "strikethrough markers survived: {out}");
    assert!(out.contains("~/Work"), "a home path was eaten: {out}");
    assert!(
        out.contains("https://e.com/a^b^"),
        "a url was rewritten: {out}"
    );
    assert!(
        out.contains("x^a b^"),
        "whitespace content was eaten: {out}"
    );
}

// --- whole-document invariants ---

#[test]
fn plain_output_adds_no_escape_bytes_and_no_box_drawing() {
    let out = plain();
    assert!(!out.contains('\u{1b}'), "no escape bytes");
    assert!(
        !out.chars().any(|c| ('\u{2500}'..='\u{257f}').contains(&c)),
        "no box drawing"
    );
}

/// The corpus above contains no control byte, so the assertion in
/// `plain_output_adds_no_escape_bytes_and_no_box_drawing` could never have
/// failed. This one supplies a document that does — which is what a hostile
/// markdown file supplies — and checks both linear exporters.
///
/// A raw ESC in the source survives `Document::parse` into `Document::text`,
/// so an unfiltered copy lets a file you merely READ set the terminal title
/// of whatever `--plain` was piped into.
#[test]
fn neither_exporter_passes_the_documents_control_bytes_through() {
    let src = concat!(
        "# t\n\n",
        "\u{1b}[31mRED\u{1b}[0m \u{7}bell \u{1b}]0;PWNED\u{7}\n\n",
        "- \u{0}nul item\n\n",
        "> \u{1b}]8;;http://evil\u{1b}\\quoted\n\n",
        "text[^e\u{1b}x]\n\n[^e\u{1b}x]: the definition\n",
    );
    let d = Document::parse(src);
    assert!(d.text.contains('\u{1b}'), "the corpus must carry one");

    let plain = carrel::plain::render(&d, 72);
    for c in plain.chars() {
        assert!(
            !c.is_control() || c == '\n',
            "control byte {c:?} in --plain output: {plain:?}"
        );
    }
    assert!(plain.contains("RED"), "the text survives: {plain:?}");

    // `--render` may emit ITS OWN escapes; what it may not do is pass the
    // document's through. The OSC-title INTRODUCER is the tell, not the word
    // `PWNED` — with the escape byte gone the payload is ordinary printable
    // text, which is exactly the right outcome: nothing is silently deleted
    // from the page, it simply stops being a command.
    let rendered = carrel::ansi::render_with(&d, 72, true);
    assert!(
        !rendered.contains("\u{1b}]0;"),
        "an OSC title reached --render output: {rendered:?}"
    );
    assert!(
        !rendered.contains('\u{7}') && !rendered.contains('\u{0}'),
        "--render passed a BEL or a NUL through: {rendered:?}"
    );
    assert!(
        rendered.contains("PWNED"),
        "the text survives: {rendered:?}"
    );
}

/// `ansi`'s header promises that `NO_COLOR` reduces `--render` to `--plain`'s
/// output "exactly". Over the whole corpus is where that promise is worth
/// something: the unit test beside it can only carry the constructs someone
/// remembered to put in its string, and it was blockquotes — in the corpus
/// here from the start — that the two walks had already disagreed about.
#[test]
fn no_color_render_is_byte_identical_to_plain_over_the_whole_corpus() {
    let d = doc();
    for w in [20u16, 40, 72, 200] {
        assert_eq!(
            carrel::ansi::render_with(&d, w, false),
            carrel::plain::render(&d, w),
            "width {w}"
        );
    }
}

/// Wide enough that `plain::render`'s own `width.max(20)` floor does not
/// swallow the case.
#[test]
fn the_whole_corpus_renders_without_panicking_at_every_plausible_width() {
    let d = doc();
    for w in [20u16, 40, 72, 200] {
        let out = carrel::plain::render(&d, w);
        assert!(!out.is_empty(), "width {w} produced nothing");
    }
}

/// The narrow end, which `plain::render` cannot reach: it opens with
/// `width.max(20)`, so widths 1, 2, 3 and 10 all became 20 and the loop above
/// ran width 20 four times. Layout itself has no such floor — `App` clamps
/// elsewhere — so it is driven directly here, which is the only way to
/// exercise arithmetic like `card_gutter`'s `.min(width / 3).max(2)` where
/// the gutter can exceed the whole width.
#[test]
fn layout_survives_widths_below_the_plain_renderers_floor() {
    let d = doc();
    let mut rows = Vec::new();
    for w in 1u16..20 {
        for wrap_tables in [false, true] {
            let layout = carrel::layout::Layout::with_images(
                &d,
                w,
                std::collections::HashMap::new(),
                wrap_tables,
            );
            for i in 0..d.block_count() {
                let b = carrel_core::BlockIdx(i as u32);
                layout.rows_for(&d, b, &mut rows);
                for row in &rows {
                    assert!(
                        row.doc.start <= row.doc.end && (row.doc.end as usize) <= d.text.len(),
                        "width {w}: row range {:?} is not a slice of the text",
                        row.doc
                    );
                }
            }
        }
    }
}

#[test]
fn the_section_index_holds_over_the_whole_corpus() {
    use carrel_core::NodeKind;
    let d = doc();
    // Every byte yields a path without panicking, and its levels strictly
    // increase — the corpus has real nesting to exercise this.
    let len = u32::try_from(d.text.len()).unwrap();
    for at in (0..=len).step_by(7) {
        let levels: Vec<u8> = d
            .section_path(at)
            .into_iter()
            .map(|id| match d.nodes[id.0 as usize].kind {
                NodeKind::Heading { level } => level,
                _ => unreachable!("paths hold only headings"),
            })
            .collect();
        assert!(
            levels.windows(2).all(|w| w[0] < w[1]),
            "at {at}: {levels:?}"
        );
    }
    // Every heading's section contains at least the heading itself.
    for n in &d.nodes {
        if matches!(n.kind, NodeKind::Heading { .. }) {
            assert!(
                d.section_end(n.id) >= n.doc.end,
                "a section cannot end before its own heading"
            );
        }
    }
}
