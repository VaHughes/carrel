//! The backend prints a run of changed cells without repositioning the cursor,
//! advancing its own bookkeeping one column per cell. Any cell whose glyph is
//! wider than that drifts every write after it until the next `MoveTo` — which
//! is how `automated` reached the screen as `automatd`.
//!
//! This walks a real diff exactly the way `CrosstermBackend::draw` does, but
//! advances the cursor by the TRUE display width, and asserts every cell still
//! lands on the column it was addressed to.

use carrel::action::{Action, Span};
use carrel::app::{App, update};
use carrel_core::{Document, display_width};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;

const SRC: &str = include_str!("corpus/wide-glyph.md");

fn render(app: &App, cols: u16, rows: u16) -> Buffer {
    let mut t = Terminal::new(TestBackend::new(cols, rows)).unwrap();
    t.draw(|f| {
        carrel::render::draw(f, app);
        carrel::render::declare_wide_cells(f);
    })
    .unwrap();
    t.backend().buffer().clone()
}

/// Replays `CrosstermBackend::draw`'s cursor rule against the true glyph width.
fn misplaced(prev: &Buffer, next: &Buffer) -> Vec<String> {
    let mut wrong = Vec::new();
    let mut last_pos: Option<(u16, u16)> = None;
    let (mut col, mut row) = (0u16, 0u16);

    for (x, y, cell) in prev.diff(next) {
        // The backend only repositions when the cell is not the previous one + 1.
        if !matches!(last_pos, Some((px, py)) if x == px + 1 && y == py) {
            col = x;
            row = y;
        }
        if (col, row) != (x, y) {
            wrong.push(format!(
                "cell {:?} addressed to col {x} row {y} would print at col {col} row {row}",
                cell.symbol()
            ));
        }
        last_pos = Some((x, y));
        col = col.saturating_add(display_width(cell.symbol()).max(1));
    }
    wrong
}

/// A previous frame in which *every* cell holds a letter, so a wide glyph's
/// trailing column is guaranteed to differ and the diff must deal with it.
/// Scrolling reaches this by accident; the guard should not have to.
fn dense(area: ratatui::layout::Rect) -> Buffer {
    let mut b = Buffer::empty(area);
    let row = "x".repeat(area.width as usize);
    for y in area.y..area.bottom() {
        b.set_stringn(
            area.x,
            y,
            &row,
            area.width as usize,
            ratatui::style::Style::default(),
        );
    }
    b
}

#[test]
fn a_wide_glyph_never_drifts_the_writes_that_follow_it() {
    let (cols, rows) = (78u16, 12u16);
    let mut app = App::new("t.md".into(), Document::parse(SRC), cols, rows);

    let mut problems = Vec::new();
    for _ in 0..40 {
        let next = render(&app, cols, rows);
        problems.extend(misplaced(&dense(*next.area()), &next));
        update(&mut app, Action::Scroll(Span::Line, 1));
    }

    assert!(
        problems.is_empty(),
        "wide glyphs drifted {} following writes, e.g.\n  {}",
        problems.len(),
        problems
            .iter()
            .take(4)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}
