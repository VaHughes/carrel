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
    let (prose, bleed, _) = App::text_size(80, 24, true, false, DEFAULT_MEASURE, 0);
    assert_eq!(prose, 75);
    assert_eq!(prose, bleed, "under the measure the two budgets agree");
    assert_eq!(
        App::text_x(80, DEFAULT_MEASURE, 0),
        2,
        "just the plain left pad"
    );
}

#[test]
fn a_wide_terminal_pins_prose_to_the_measure_and_centres_it() {
    let (prose, bleed, _) = App::text_size(200, 24, true, false, DEFAULT_MEASURE, 0);
    assert_eq!(prose, 90);
    assert_eq!(bleed, 195, "200 - 1 scrollbar - 2 - 2");
    // PAD_LEFT + (195 - 90) / 2 = 2 + 52
    assert_eq!(App::text_x(200, DEFAULT_MEASURE, 0), 54);
}

#[test]
fn zero_reproduces_the_pre_measure_geometry_exactly() {
    let (prose, bleed, h) = App::text_size(200, 24, true, false, 0, 0);
    assert_eq!((prose, bleed), (195, 195));
    assert_eq!(h, 20, "24 - 2 chrome - 1 top - 1 bottom");
    assert_eq!(App::text_x(200, 0, 0), 2, "the old left edge, unmoved");
}

#[test]
fn a_terminal_narrower_than_its_own_chrome_does_not_underflow() {
    let (prose, bleed, h) = App::text_size(1, 1, true, false, DEFAULT_MEASURE, 0);
    assert_eq!((prose, bleed, h), (0, 0, 0));
    assert_eq!(App::text_x(1, DEFAULT_MEASURE, 0), 2);
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
    let x = App::text_x(200, DEFAULT_MEASURE, 0);
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
        let x = App::text_x(cols, app.max_width, 0);
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

/// The same guard, with the margin outline reserving columns.
///
/// **This is the one that makes the gutter safe.** The measure work taught
/// that hard-coding a left edge offsets every click by the margin and no
/// frame test can see it; a gutter is the same hazard again, so the same
/// round trip has to hold with one present.
#[test]
fn clicking_a_column_still_lands_on_its_character_with_the_margin_outline() {
    let src = format!(
        "# A heading

## Another

{PROSE}"
    );
    // The gutter needs 26 spare columns, so it appears from about 118.
    for cols in [130u16, 160, 200] {
        let mut app = app_at(cols, 24, &src);
        app.outline_margin = true;
        app.on_resize(cols, 24);
        assert!(app.gutter_w() > 0, "cols={cols}: precondition, no gutter");

        let buf = frame(&app, cols, 24);
        let x = App::text_x(cols, app.max_width, app.gutter_w());
        assert!(
            x >= carrel::app::PAD_LEFT + app.gutter_w(),
            "the text must start right of the gutter"
        );
        let mut checked = 0;
        for row in [app.text_y(), app.text_y() + 2] {
            for col in x..x + app.text_w() {
                let Some((start, end)) = app.doc_span_at(col, row) else {
                    continue;
                };
                let byte = &app.doc.text[start as usize..end as usize];
                let painted = buf[(col, row)].symbol();
                if painted == " " {
                    continue;
                }
                assert_eq!(
                    byte, painted,
                    "cols={cols} col={col} row={row}: pointer says {byte:?},                      cell paints {painted:?}"
                );
                checked += 1;
            }
        }
        assert!(checked > 10, "cols={cols}: checked almost nothing");
    }
}

/// A narrow terminal folds the gutter away rather than squeezing the measure.
#[test]
fn the_margin_outline_gives_way_before_the_measure_does() {
    let src = format!(
        "# A heading

{PROSE}"
    );
    let mut app = app_at(80, 24, &src);
    app.outline_margin = true;
    app.on_resize(80, 24);
    assert_eq!(app.gutter_w(), 0, "80 columns has none to spare");
    let narrow = app.text_w();

    app.on_resize(200, 24);
    assert!(app.gutter_w() > 0, "200 columns does");
    assert_eq!(
        app.text_w(),
        narrow.max(app.text_w()),
        "the measure is never squeezed to make room"
    );
    assert_eq!(app.text_w(), DEFAULT_MEASURE, "still the full measure");
}

#[test]
fn a_click_left_of_the_centred_column_is_outside_the_text() {
    let app = app_at(200, 24, PROSE);
    let x = App::text_x(200, DEFAULT_MEASURE, 0);
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
    let x = App::text_x(200, DEFAULT_MEASURE, 0);
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
        link.x >= App::text_x(200, DEFAULT_MEASURE, 0),
        "a link cannot start left of the text column"
    );
}

/// **The generic click-target guard.**
///
/// Every hand-written round-trip in this file checks one surface. This checks
/// them all at once, and every surface added later inherits it for free: for
/// each target the painter registered, read the cells it covers back off the
/// buffer and assert they show what that action claims to act on. A target
/// whose rectangle drifts one cell off its glyph fails here, which is the
/// failure no frame assertion can see.
#[test]
fn every_registered_target_covers_the_thing_it_acts_on() {
    use carrel::action::Action;

    // Two links, a folded section and an unfolded <details>: one document
    // that registers every kind of target step 2 paints.
    // `# Two` matters: folding `# One` must not hide the links and the
    // <details> block this test exists to check. No line continuations —
    // their leading indentation lands INSIDE the string, and fifteen
    // spaces in front of a tag makes markdown an indented code block.
    let src = "# One\n\nbody of one\n\n# Two\n\nsee [the docs](https://example.com) and [more](./more.md)\n\n<details>\n<summary>Open me</summary>\n\nhidden\n\n</details>\n";
    let mut app = app_at(120, 24, src);
    // Fold the first heading, so its ▸ marker is painted and registered too.
    carrel::app::update(&mut app, Action::FoldAt(0));

    let mut painted = carrel::render::Painted::default();
    let mut protocols = std::collections::HashMap::new();
    let mut t = Terminal::new(TestBackend::new(120, 24)).unwrap();
    t.draw(|f| carrel::render::draw_full(f, &app, &mut painted, &mut protocols))
        .unwrap();
    let buf = t.backend().buffer();

    let mut checked = 0;
    let mut link_text: Vec<String> = Vec::new();
    for target in painted.targets.as_slice() {
        let z = target.zone;
        assert!(
            z.x + z.w <= 120 && z.y + z.h <= 24,
            "a target must lie inside the frame it was painted in: {target:?}"
        );
        let painted: String = (z.x..z.x + z.w)
            .map(|x| buf[(x, z.y)].symbol().to_string())
            .collect();
        match target.action {
            Action::LinkOpen(i) => {
                assert!(
                    app.doc.links.get(i as usize).is_some(),
                    "link target {i} indexes past the document"
                );
                // EXACT, not "contains": a one-cell drift still lands inside
                // the document's text, and "contains" would shrug at it. That
                // drift is the entire failure this test exists to catch.
                link_text.push(painted.clone());
                checked += 1;
            }
            Action::FoldAt(_) => {
                assert!(
                    painted == "\u{25b8}" || painted == "\u{25be}",
                    "a fold target must cover a fold marker, found {painted:?}"
                );
                checked += 1;
            }
            other => panic!("unclassified target {other:?} — teach this test what it covers"),
        }
    }
    link_text.sort();
    assert_eq!(
        link_text,
        vec!["more".to_string(), "the docs".to_string()],
        "each link target must cover its own link text exactly, edge to edge"
    );
    assert!(
        checked >= 4,
        "the fixture must actually register targets, got {checked}"
    );
}

/// **The modal-precedence guard.**
///
/// A pane owns its rectangle the way it owns the keyboard. Before the target
/// registry there was no precedence at all on the pointer side: a click on an
/// open pane started a text selection in the document behind it. The `z`
/// ordering is what fixes that, and this is what proves it stays fixed.
#[test]
fn an_open_pane_takes_every_click_inside_it() {
    use carrel::action::Action;

    let src = "# Alpha\n\nbody\n\n# Beta\n\nmore body\n\n# Gamma\n\nlast\n";
    let mut app = app_at(120, 24, src);
    carrel::app::update(&mut app, Action::OutlineToggle);
    assert!(app.outline.is_some(), "the outline picker is up");

    let mut painted = carrel::render::Painted::default();
    let mut protocols = std::collections::HashMap::new();
    let mut t = Terminal::new(TestBackend::new(120, 24)).unwrap();
    t.draw(|f| carrel::render::draw_full(f, &app, &mut painted, &mut protocols))
        .unwrap();

    let jumps: Vec<_> = painted
        .targets
        .as_slice()
        .iter()
        .filter(|t| matches!(t.action, Action::OutlineJumpAt(_)))
        .collect();
    assert_eq!(jumps.len(), 3, "one target per heading in the picker");

    // Every row resolves to the entry painted on it, and to no other.
    let buf = t.backend().buffer();
    for (n, target) in jumps.iter().enumerate() {
        let z = target.zone;
        let painted_row: String = (z.x..z.x + z.w)
            .map(|x| buf[(x, z.y)].symbol().to_string())
            .collect();
        let want = ["Alpha", "Beta", "Gamma"][n];
        assert!(
            painted_row.contains(want),
            "row {n} should show {want}, shows {painted_row:?}"
        );
        assert_eq!(
            painted.targets.hit(z.x + 1, z.y).map(|h| h.action),
            Some(target.action),
            "a click on row {n} must resolve to row {n}"
        );
    }

    // The pane's own background absorbs. Take a cell on the pane's border
    // row, which is inside the rectangle and on no entry.
    let pane = jumps[0].zone;
    let border = painted
        .targets
        .hit(pane.x, pane.y.saturating_sub(1))
        .map(|h| h.action);
    assert_eq!(
        border,
        Some(Action::Absorb),
        "a click on the pane but not on a row is swallowed, not passed through"
    );

    // And nothing under the pane leaks out: no document target may win
    // anywhere inside it.
    for x in pane.x..pane.x + pane.w {
        let hit = painted.targets.hit(x, pane.y).expect("inside the pane");
        assert!(
            matches!(hit.action, Action::Absorb | Action::OutlineJumpAt(_)),
            "the pane leaked a {:?} at column {x}",
            hit.action
        );
    }
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

/// Every column a BLEED block paints into must resolve to the byte painted
/// there — not to the byte the prose column would have put there.
///
/// `block_area` paints tables, code, math and images against the bleed column
/// and re-centres a wide table; `doc_span_at` bounded every click by the prose
/// column instead. They agreed for prose and diverged for everything else
/// whenever bleed exceeded the measure — at 95 columns or wider with the
/// default 90-column measure, so any maximized terminal. Measured before the
/// fix at 160 columns: the table painted from column 22, clicks below 35 were
/// rejected outright, and a click at column 40 selected the cell painted at
/// column 22 — thirteen columns to the left of the pointer.
///
/// The existing sweep above could not catch it: it uses a prose document and
/// only walks `x..x + text_w()`, and every frame test in `render.rs` runs at
/// 60 columns or less, where nothing bleeds.
#[test]
fn clicking_a_wide_table_lands_on_the_cell_under_the_pointer() {
    let cells = [
        "AAAAAAAAAAAA",
        "BBBBBBBBBBBB",
        "CCCCCCCCCCCC",
        "DDDDDDDDDDDD",
        "EEEEEEEEEEEE",
        "FFFFFFFFFFFF",
        "GGGGGGGGGGGG",
        "HHHHHHHHHHHH",
    ];
    let row = format!("|{}|", cells.join("|"));
    let rule = format!("|{}|", ["------------"; 8].join("|"));
    let src = format!("# T\n\n{row}\n{rule}\n{row}\n");

    for cols in [100u16, 140, 160, 200] {
        let app = app_at(cols, 16, &src);
        // Precondition: this width really does make the table bleed past the
        // measure, or the test proves nothing.
        let (bx, bw) = app.block_span_x(carrel_core::BlockIdx(1));
        let prose_x = App::text_x(cols, DEFAULT_MEASURE, 0);
        assert!(
            bx < prose_x,
            "at {cols} cols the table must start left of the prose column"
        );

        let buf = frame(&app, cols, 16);
        // Find the row the table actually painted into rather than assuming
        // one — hardcoding a row number made an earlier version of this test
        // sweep a blank line and pass while asserting nothing.
        let table_row = (0..16u16)
            .find(|&y| (bx..bx + bw).any(|x| buf[(x, y)].symbol().starts_with('A')))
            .expect("the table must paint somewhere");
        let mut checked = 0;
        for col in bx..bx + bw {
            // What is painted here?
            let painted = buf[(col, table_row)].symbol().chars().next().unwrap_or(' ');
            if !painted.is_ascii_uppercase() {
                continue; // a separator or padding cell
            }
            let (start, _) = app.doc_span_at(col, table_row).unwrap_or_else(|| {
                panic!("col {col} at {cols} paints {painted:?} but hit-tests to nothing")
            });
            let hit = app.doc.text[start as usize..].chars().next().unwrap_or(' ');
            assert_eq!(
                hit, painted,
                "at {cols} cols, column {col} paints {painted:?} but resolves to {hit:?}"
            );
            checked += 1;
        }
        assert!(
            checked > 40,
            "only {checked} cells checked at {cols} cols — the sweep found nothing to assert on"
        );
    }
}
