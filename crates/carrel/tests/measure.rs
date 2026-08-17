//! The reading measure: prose binds, wide blocks bleed.
//!
//! Carrel used to render a 200-column paragraph on a 200-column terminal,
//! which is genuinely bad reading — past roughly 90 characters the eye loses
//! the line return. Prose now caps at a measure and centres; tables, code,
//! math and images may still use the whole terminal.
//!
//! The load-bearing test here is [`clicking_a_column_lands_on_the_character_painted_there`].
//! A one-cell offset between paint and hit-testing is invisible to every frame
//! assertion and instantly visible to anyone dragging a selection.

use carrel::app::App;
use carrel::config::DEFAULT_MEASURE;
use carrel_core::{BlockIdx, Document};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;

const PROSE: &str = "The measure is the length of a line of type, and the comfortable one runs to \
roughly ninety characters; past that the eye loses its place on the return sweep and reading \
becomes work rather than pleasure.";

fn app_at(cols: u16, rows: u16, src: &str) -> App {
    let mut app = App::new("t.md".into(), Document::parse(src), cols, rows);
    app.on_resize(cols, rows);
    app
}

fn frame(app: &App, cols: u16, rows: u16) -> Buffer {
    let mut t = Terminal::new(TestBackend::new(cols, rows)).unwrap();
    t.draw(|f| carrel::render::draw(f, app)).unwrap();
    t.backend().buffer().clone()
}

// --- geometry ---

#[test]
fn a_narrow_terminal_is_untouched_by_the_measure() {
    // 80 columns: usable text is 80 - 1 scrollbar - 2 - 2 = 75, under 90.
    let (prose, bleed, _) = App::text_size(80, 24, true, false, DEFAULT_MEASURE);
    assert_eq!(prose, 75);
    assert_eq!(prose, bleed, "under the measure the two budgets agree");
    assert_eq!(
        App::text_x(80, DEFAULT_MEASURE),
        2,
        "just the plain left pad"
    );
}

#[test]
fn a_wide_terminal_pins_prose_to_the_measure_and_centres_it() {
    let (prose, bleed, _) = App::text_size(200, 24, true, false, DEFAULT_MEASURE);
    assert_eq!(prose, 90);
    assert_eq!(bleed, 195, "200 - 1 scrollbar - 2 - 2");
    // PAD_LEFT + (195 - 90) / 2 = 2 + 52
    assert_eq!(App::text_x(200, DEFAULT_MEASURE), 54);
}

#[test]
fn zero_reproduces_the_pre_measure_geometry_exactly() {
    let (prose, bleed, h) = App::text_size(200, 24, true, false, 0);
    assert_eq!((prose, bleed), (195, 195));
    assert_eq!(h, 20, "24 - 2 chrome - 1 top - 1 bottom");
    assert_eq!(App::text_x(200, 0), 2, "the old left edge, unmoved");
}

#[test]
fn a_terminal_narrower_than_its_own_chrome_does_not_underflow() {
    let (prose, bleed, h) = App::text_size(1, 1, true, false, DEFAULT_MEASURE);
    assert_eq!((prose, bleed, h), (0, 0, 0));
    assert_eq!(App::text_x(1, DEFAULT_MEASURE), 2);
}

// --- what binds and what bleeds ---

#[test]
fn prose_wraps_at_the_measure_on_a_wide_terminal() {
    let app = app_at(200, 24, PROSE);
    // 195 columns would hold this on two rows; 90 needs more.
    let bound = app.layout.height(BlockIdx(0));
    let mut wide = app_at(200, 24, PROSE);
    wide.max_width = 0;
    wide.on_resize(200, 24);
    let unbound = wide.layout.height(BlockIdx(0));
    assert!(
        bound > unbound,
        "the measure must actually bind: bound={bound} unbound={unbound}"
    );
}

/// The height of block 0 with the measure on, and with it off, at 200 columns.
/// Equal means the block ignored the measure — which is what bleeding means.
fn bound_vs_unbound(src: &str) -> (u32, u32) {
    let bound = app_at(200, 24, src);
    let mut unbound = app_at(200, 24, src);
    unbound.max_width = 0;
    unbound.on_resize(200, 24);
    (
        bound.layout.height(BlockIdx(0)),
        unbound.layout.height(BlockIdx(0)),
    )
}

#[test]
fn a_code_block_bleeds_past_the_measure() {
    let long = "x".repeat(150);
    let src = format!("```\nlet v = \"{long}\";\n```\n");
    let (bound, unbound) = bound_vs_unbound(&src);
    assert_eq!(
        bound, unbound,
        "a 150-column code line must lay out the same whether or not prose is \
         capped at 90 — code uses the bleed budget"
    );
}

#[test]
fn a_table_that_fits_the_terminal_does_not_card_because_prose_got_narrower() {
    let src = "| alpha | beta | gamma | delta | epsilon |\n\
               |---|---|---|---|---|\n\
               | a value here | b value here | c value here | d value here | e value |\n";
    let (bound, unbound) = bound_vs_unbound(src);
    assert_eq!(
        bound, unbound,
        "this table fits 195 columns; capping prose at 90 must not transpose \
         it into cards — table_overflows must test the bleed width"
    );
    // And prove the table really would card at a genuinely narrow terminal,
    // so the assertion above is not vacuous.
    let narrow = app_at(40, 24, src);
    assert!(
        narrow.layout.height(BlockIdx(0)) > bound,
        "precondition: this table does card when the TERMINAL is narrow"
    );
}

#[test]
fn every_node_kind_is_classified_by_the_layout() {
    // The classifier's match is exhaustive, so this is really a compile-time
    // guarantee; the assertion is that both budgets are actually reachable.
    let src = "# Heading\n\npara\n\n- item\n\n> quote\n\n```\ncode\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n---\n";
    let app = app_at(200, 24, src);
    let mut saw_measure = false;
    let mut saw_bleed = false;
    for i in 0..app.doc.block_count() {
        let kind = &app.doc.node_for_block(BlockIdx(i as u32)).kind;
        let w = app.layout.block_width(kind);
        if w == app.layout.measure() {
            saw_measure = true;
        }
        if w == 195 {
            saw_bleed = true;
        }
    }
    assert!(saw_measure, "no block bound to the measure");
    assert!(saw_bleed, "no block used the bleed");
}

// --- paint ---

#[test]
fn a_wide_frame_centres_the_prose_column() {
    let app = app_at(200, 12, "hello measure");
    let buf = frame(&app, 200, 12);
    let x = App::text_x(200, DEFAULT_MEASURE);
    assert_eq!(buf[(x, 1)].symbol(), "h", "text starts at the centred edge");
    assert_eq!(
        buf[(2, 1)].symbol(),
        " ",
        "and nothing is painted at the old left pad"
    );
}

#[test]
fn a_table_wider_than_the_measure_is_not_clipped_by_it() {
    // Found by looking at a real 160-column terminal, not by a test: the
    // paint rect was the PROSE width, so a table laid out against the bleed
    // budget had its last columns cut off. Every column must survive.
    let src = "| column one | column two | column three | column four | column five | \
               column six | column seven | column eight |\n\
               |---|---|---|---|---|---|---|---|\n\
               | a | b | c | d | e | f | g | h |\n";
    let app = app_at(160, 12, src);
    let buf = frame(&app, 160, 12);
    let row: String = (0..160).map(|c| buf[(c, 1)].symbol()).collect();
    assert!(
        row.contains("column eight"),
        "the last column was clipped; row painted: {row:?}"
    );
    // And it is centred on the page axis rather than jammed against a wall.
    let lead = row.len() - row.trim_start().len();
    assert!(lead > 2, "a wide table should still be centred, x={lead}");
}

#[test]
fn measure_off_paints_exactly_where_it_used_to() {
    let mut app = app_at(200, 12, "hello measure");
    app.max_width = 0;
    app.on_resize(200, 12);
    let buf = frame(&app, 200, 12);
    assert_eq!(buf[(2, 1)].symbol(), "h");
}

// --- the one that matters ---

#[test]
fn clicking_a_column_lands_on_the_character_painted_there() {
    for cols in [40u16, 60, 80, 100, 140, 200, 300] {
        let app = app_at(cols, 24, PROSE);
        let buf = frame(&app, cols, 24);
        let x = App::text_x(cols, app.max_width);
        let mut checked = 0;
        for col in x..x + app.text_w() {
            let Some((start, end)) = app.doc_span_at(col, 1) else {
                continue;
            };
            let byte = &app.doc.text[start as usize..end as usize];
            let painted = buf[(col, 1)].symbol();
            if painted == " " {
                continue; // trailing cells past the end of a wrapped row
            }
            assert_eq!(
                byte, painted,
                "cols={cols} col={col}: the pointer resolved to {byte:?} \
                 but the cell paints {painted:?}"
            );
            checked += 1;
        }
        assert!(checked > 10, "cols={cols}: test checked almost nothing");
    }
}

#[test]
fn a_click_left_of_the_centred_column_is_outside_the_text() {
    let app = app_at(200, 24, PROSE);
    let x = App::text_x(200, DEFAULT_MEASURE);
    assert!(x > 2, "precondition: the column is actually inset");
    assert_eq!(app.doc_span_at(x - 1, 1), None, "the margin is not text");
    assert!(app.doc_span_at(x, 1).is_some(), "the first cell is");
}

// --- the two risks the spec flagged (§8) ---

#[test]
fn a_selection_highlights_the_cells_the_text_is_painted_in() {
    // Highlight columns come from `cols_for_doc_range` and are BLOCK-relative;
    // if the rect they are painted into keeps the page's x, the highlight
    // lands in the left margin instead of on the words.
    let mut app = app_at(200, 12, "hello measure");
    app.selection = Some(0..5); // "hello"
    let buf = frame(&app, 200, 12);
    let x = App::text_x(200, DEFAULT_MEASURE);
    // The mouse selection is REVERSED, not a background colour — it has to
    // read as a selection in all 17 palettes and under NO_COLOR.
    let reversed = |c: u16| {
        buf[(c, 1)]
            .style()
            .add_modifier
            .contains(ratatui::style::Modifier::REVERSED)
    };
    assert!(reversed(x), "the selection starts where the text starts");
    assert!(reversed(x + 4), "and covers the whole word");
    assert!(
        !reversed(x + 6),
        "the word after the selection must not be highlighted"
    );
    assert!(
        !reversed(2),
        "and neither must the margin the text used to start in"
    );
}

#[test]
fn an_osc8_link_is_emitted_at_the_column_its_text_is_painted_in() {
    // The OSC 8 pass re-emits link text at ABSOLUTE columns after the frame.
    // An underline two cells left of its own text is exactly the kind of
    // thing that ships unnoticed.
    let app = app_at(200, 12, "see [the docs](https://example.com) here");
    let mut links = Vec::new();
    let mut t = Terminal::new(TestBackend::new(200, 12)).unwrap();
    t.draw(|f| carrel::render::draw_with_links(f, &app, &mut links))
        .unwrap();
    let buf = t.backend().buffer();
    let link = links.first().expect("the document has one link");
    assert_eq!(
        buf[(link.x, link.y)].symbol(),
        &link.text[..1],
        "the OSC 8 run must start on the cell its first character occupies"
    );
    assert!(
        link.x >= App::text_x(200, DEFAULT_MEASURE),
        "a link cannot start left of the text column"
    );
}

// --- the invariant ---

#[test]
fn the_anchor_survives_a_resize_that_crosses_the_measure() {
    let src = format!("{PROSE}\n\n{PROSE}\n\n{PROSE}\n\n{PROSE}\n");
    let mut app = app_at(200, 24, &src);
    let h = app.text_h();
    app.view.scroll_to(&app.doc, &app.layout, 6, h);
    let anchor = app.view.anchor;

    app.on_resize(60, 24); // 200 was measure-bound at 90; 60 is not bound at all
    let top = app.layout.block_at_row(app.view.scroll_row);
    let anchor_row = app
        .layout
        .visual_row_of(&app.doc, top, anchor, carrel_core::Affinity::Right);
    assert_eq!(
        app.view.scroll_row,
        app.layout.row_start(top) + anchor_row,
        "the row holding the anchor must still be the top row"
    );
}

#[test]
fn a_search_hit_is_the_same_byte_bound_or_unbound() {
    let hits = |max_width: u16| {
        let mut app = app_at(200, 24, PROSE);
        app.max_width = max_width;
        app.on_resize(200, 24);
        carrel_core::search(&app.doc, "measure", false)
            .ranges
            .iter()
            .map(|r| r.start)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        hits(DEFAULT_MEASURE),
        hits(0),
        "search state is doc bytes; the measure cannot touch it"
    );
}
