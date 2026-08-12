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
    rows.get(sub).map_or(0..0, |r| r.doc.clone())
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
            prop_assert!(
                top.contains(&before) || top.start == before,
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
