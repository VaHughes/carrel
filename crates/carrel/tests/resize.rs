//! `StableViewport` as an invariant: a resize must not move the reader.
//!
//! This is the TUI-side twin of `carrel-core`'s `search_results_survive_reflow`
//! and the reason the project exists. `architecture.md` §3.5.

use carrel::action::{Action, Span};
use carrel::app::{App, update};
use carrel_core::Document;
use proptest::prelude::*;

// The table is 3 columns (nothing else in the tree exercises ncols > 2, and
// `idx % ncols` in render.rs's gutter-label lookup needs a case where that
// matters) with a max-content width of 73 (5 + 49 + 13 + 3*2, per
// `table_overflows`): inside the 8..=120 width range under test, so
// properties see both the aligned form (width >= 73) and the card form
// (width < 73) as `w1`/`w2` vary. "the" — the word `matches_survive_a_resize`
// searches for — appears twice inside body cells, so a match can land inside
// a table row on either side of the flip.
const SRC: &str = "# Heading one\n\nalpha beta gamma delta epsilon zeta eta theta iota kappa \
lambda mu nu xi omicron pi rho sigma tau upsilon\n\n- a list item long enough to wrap at most \
widths under test\n- another one\n\n> a quoted paragraph that also wraps when narrow\n\n\
| name | description | note |\n|---|---|---|\n\
| alpha | a value long enough to overflow the narrow widths | the first one |\n\
| beta | another lengthy entry that keeps going | short |\n\n\
Final paragraph with 日本語 and a supercalifragilisticexpialidocious word.\n";

/// The doc range actually painted on the top row of the viewport.
///
/// Asserting `app.view.anchor` is unchanged would prove nothing — `restore`
/// never writes it. The real invariant is that `scroll_row` is re-derived so
/// the row CONTAINING the anchor is the one at the top: after reflow the
/// anchor is usually mid-row, because the row that holds it starts earlier at
/// the new width. Showing that row is exactly what `StableViewport` means.
fn top_visible_row(app: &App) -> std::ops::Range<u32> {
    let b = app.layout.block_at_row(app.view.scroll_row);
    let mut rows = Vec::new();
    app.layout.rows_for(&app.doc, b, &mut rows);
    let sub = (app.view.scroll_row - app.layout.row_start(b)) as usize;
    let row = rows.get(sub).map_or(0..0, |r| r.doc.clone());
    // The gap under a block is named by the separator byte after it, not by
    // a byte of its (empty) range: `view::row_anchor` says which.
    match carrel::view::row_anchor(&app.doc, &app.layout, app.view.scroll_row) {
        Some(at) if at != row.start => at..at,
        _ => row,
    }
}

/// Whether `anchor` is a table byte past the end of row `top` but still on
/// its line — padding an aligned row elides, or the line's own `\n` (a card
/// rule's byte) — which names the line it ends.
fn ends_its_line(app: &App, top: &std::ops::Range<u32>, anchor: u32) -> bool {
    let b = app.doc.block_at_doc(carrel_core::DocByte(anchor));
    matches!(
        app.doc.node_for_block(b).kind,
        carrel_core::NodeKind::Table { .. }
    ) && top.end <= anchor
        && app
            .doc
            .text
            .get(top.end as usize..anchor as usize)
            .is_some_and(|t| t.chars().all(char::is_whitespace) && !t.contains('\n'))
}

proptest! {
    /// The reader keeps looking at the same text across a width change.
    #[test]
    fn the_reader_does_not_move_on_resize(
        scroll in 0i32..40,
        w1 in 8u16..=120,
        w2 in 8u16..=120,
        h in 4u16..=40,
    ) {
        let doc = Document::parse(SRC);
        let mut app = App::new("t.md".into(), doc, w1, h);
        update(&mut app, Action::Scroll(Span::Line, scroll));
        let before = app.view.anchor;
        prop_assert_eq!(top_visible_row(&app).start, before, "precondition");

        app.on_resize(w2, h);
        prop_assert_eq!(app.view.anchor, before, "the anchor is authoritative");
        // Clamping at the end of a short document can pull the top ABOVE the
        // anchor to keep the screen full; it must never fall past it.
        let top = top_visible_row(&app);
        prop_assert!(
            top.start <= before,
            "top row {:?} starts past the anchor {} at width {}", top, before, w2,
        );
        if app.view.scroll_row < app.layout.max_scroll(app.text_h()) {
            // A card rule is named by the `\n` ending the card above it,
            // which the column view's rows do not hold either: there it is
            // the line that ends at it, just above the next card's line.
            let line_end = ends_its_line(&app, &top, before);
            prop_assert!(
                top.contains(&before) || top.start == before || line_end,
                "anchor {} is not on the top row {:?} at width {}", before, top, w2,
            );
        }

        app.on_resize(w1, h);
        prop_assert_eq!(app.view.anchor, before, "anchor moved on resize back");
        prop_assert_eq!(
            top_visible_row(&app).start, before,
            "returning to the original width must restore the original top row",
        );
    }

    /// scroll_row is always a legal position at the current width.
    #[test]
    fn scroll_row_stays_within_the_document(
        scroll in 0i32..200, w in 8u16..=120, h in 4u16..=40,
    ) {
        let doc = Document::parse(SRC);
        let mut app = App::new("t.md".into(), doc, w, h);
        update(&mut app, Action::Scroll(Span::Line, scroll));
        prop_assert!(app.view.scroll_row <= app.layout.max_scroll(app.text_h()));
    }

    /// A resize leaves the match set and the current index untouched.
    /// mdfried #52, at the level a user would actually hit it.
    #[test]
    fn matches_survive_a_resize(w1 in 8u16..=120, w2 in 8u16..=120, h in 4u16..=40) {
        use carrel::action::{Direction, SearchKey};
        let doc = Document::parse(SRC);
        let mut app = App::new("t.md".into(), doc, w1, h);
        update(&mut app, Action::SearchOpen(Direction::Forward));
        for c in "the".chars() {
            update(&mut app, Action::SearchKey(SearchKey::Char(c)));
        }
        update(&mut app, Action::SearchKey(SearchKey::Accept));

        let ranges = app.matches.as_ref().map(|m| m.ranges.clone());
        let current = app.matches.as_ref().and_then(|m| m.current);

        app.on_resize(w2, h);

        prop_assert_eq!(app.matches.as_ref().map(|m| m.ranges.clone()), ranges);
        prop_assert_eq!(app.matches.as_ref().and_then(|m| m.current), current);
    }
}

/// A card rule on top comes back as itself — after a resize to the same
/// size, and after a round trip through the column view and back. A rule
/// was named by the next card's first byte, which that card's first row
/// also holds, so a rule on top came back one row lower, and the trailing
/// rule a whole card higher (found by fuzzing, 2026-11-14).
#[test]
fn every_card_row_survives_a_resize_and_a_trip_through_the_column_view() {
    let long = "a value easily long enough to overflow any narrow terminal";
    let srcs = [
        format!(
            "lead\n\n| name | description |\n|---|---|\n| alpha | {long} |\n| beta | short |\n\nafter\n"
        ),
        format!("| a | b | c |\n|---|---|---|\n| x | {long} | |\n| y | | |\n| z |\n\nafter\n"),
        format!("> | k | v |\n> |---|---|\n> | one | {long} |\n> | two | |\n\nafter\n"),
        format!("intro\n\n| k | v |\n|---|---|\n| one | {long} |\n| two | |"),
        format!("intro\n\n| k | v |\n|---|---|\n| one | {long} |\n| two | {long} |\n\nafter\n"),
        format!(
            "intro\n\n- item\n\n  | k | v |\n  |---|---|\n  | one | {long} |\n  | two | {long} |\n"
        ),
    ];
    // Enough below every table that the column view can put it on top:
    // a short page clamps, and the in-between check would prove nothing.
    let srcs = srcs.map(|s| format!("{s}\n\n{}", "more\n\n".repeat(12)));
    let h = 6;
    let mut failures = Vec::new();
    let mut between_failures = Vec::new();
    let mut rules = 0;
    for (i, src) in srcs.iter().enumerate() {
        for w1 in [20u16, 28, 40, 60] {
            // The same size, then far past the card threshold and back.
            for w2 in [w1, 200] {
                let probe = App::new("t.md".into(), Document::parse(src), w1, h);
                for row in 0..=probe.layout.max_scroll(probe.text_h()) {
                    let mut app = App::new("t.md".into(), Document::parse(src), w1, h);
                    let th = app.text_h();
                    app.view.scroll_to(&app.doc, &app.layout, row, th);
                    let b = app.layout.block_at_row(row);
                    let node = app.doc.node_for_block(b);
                    if !matches!(node.kind, carrel_core::NodeKind::Table { .. }) {
                        continue;
                    }
                    let mut rows = Vec::new();
                    app.layout.rows_for(&app.doc, b, &mut rows);
                    let r = &rows[(row - app.layout.row_start(b)) as usize];
                    let rule = matches!(r.kind, carrel_core::RowKind::Decoration)
                        && r.doc.start != node.doc.start;
                    rules += usize::from(rule);
                    let anchor = app.view.anchor;
                    app.on_resize(w2, h);
                    // In between, too: the column view's top row is the one
                    // that holds the anchor — or, for a rule's `\n`, the
                    // line that ends at it. The trailing rule's `\n` once
                    // landed on the gap BELOW the table, scrolling it off.
                    let top = top_visible_row(&app);
                    let between = top.contains(&anchor)
                        || top.start == anchor
                        || ends_its_line(&app, &top, anchor);
                    if !between {
                        between_failures.push((i, w1, w2, row, rule, anchor, top));
                    }
                    app.on_resize(w1, h);
                    if app.view.scroll_row != row {
                        failures.push((i, w1, w2, row, rule, app.view.scroll_row));
                    }
                }
            }
        }
    }
    assert!(rules > 20, "the fixture must reach card rules: {rules}");
    assert!(
        between_failures.is_empty(),
        "(src, w1, w2, row, rule, anchor, top row at w2): {between_failures:?}"
    );
    assert!(
        failures.is_empty(),
        "(src, w1, w2, row, rule, came back at): {failures:?}"
    );
}
