//! A peek: a footnote's text, shown where its mark is.
//!
//! Following a footnote means leaving the sentence you were in the middle
//! of, reading one line at the bottom of the document, and finding your way
//! back. `%` does that and `Back` returns, which is correct and is three
//! steps for a parenthesis. A peek is the parenthesis: click the mark, read
//! the text in place, carry on.
//!
//! It is not a pane. It owns no keyboard and blocks nothing: whatever the
//! reader does next closes it and then happens. The one thing it offers is
//! the long way round — `Enter`, or its button, goes to the footnote itself.
//!
//! NO RATATUI — `scripts/check-discipline.sh` rule 6. The geometry is in
//! terminal cells, as [`crate::menu`]'s is, and nothing here draws.

use crate::action::Zone;

/// The most rows of footnote a peek shows before it says there is more.
const MAX_LINES: usize = 8;
/// As wide as a comfortable line, and no wider than the window allows.
const MAX_W: u16 = 64;

/// An open peek.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Peek {
    /// The cell the mark was clicked at.
    pub at: (u16, u16),
    /// Where the footnote's own text starts, for "go to it".
    pub def: u32,
    /// The label, as written: `[^name]`.
    pub label: String,
    /// The footnote's text.
    pub text: String,
}

impl Peek {
    /// The rows of text for a box `w` cells wide in a window `rows` tall,
    /// the last one an ellipsis when the footnote is longer than the box
    /// can be. The window is part of the question: a short one holds fewer
    /// rows than a peek otherwise would, and the text used to stop there
    /// mid-sentence with nothing to say it had.
    #[must_use]
    pub fn lines(&self, w: u16, rows: u16) -> Vec<String> {
        let inner = w.saturating_sub(4).max(1);
        // Two borders and the button's row are not text.
        let max = MAX_LINES.min(usize::from(rows.saturating_sub(3))).max(1);
        let mut lines = crate::layout::wrap_ui_text(&self.text, inner);
        if lines.len() > max {
            lines.truncate(max);
            if let Some(last) = lines.last_mut() {
                *last = "…".to_string();
            }
        }
        if lines.is_empty() {
            lines.push(String::new());
        }
        lines
    }

    /// The box, clamped into a `cols × rows` viewport: under the mark, or
    /// above it when there is no room below — the menu's rule, for the
    /// menu's reason.
    #[must_use]
    pub fn zone(&self, cols: u16, rows: u16) -> Zone {
        let w = MAX_W.min(cols.saturating_sub(2)).max(1);
        // Borders, the text, and the row the button sits on.
        let h = u16::try_from(self.lines(w, rows).len())
            .unwrap_or(u16::MAX)
            .saturating_add(3)
            .min(rows.max(1));
        let (cx, cy) = self.at;
        let below = cy.saturating_add(1);
        let y = if below.saturating_add(h) <= rows {
            below
        } else {
            cy.saturating_sub(h)
        };
        Zone::new(
            cx.min(cols.saturating_sub(w)),
            y.min(rows.saturating_sub(h)),
            w,
            h,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peek(text: &str, at: (u16, u16)) -> Peek {
        Peek {
            at,
            def: 0,
            label: "[^n]".into(),
            text: text.into(),
        }
    }

    #[test]
    fn the_box_sits_under_its_mark_and_flips_above_at_the_bottom() {
        let p = peek("a short footnote", (10, 3));
        let z = p.zone(80, 24);
        assert_eq!((z.x, z.y), (10, 4));
        assert_eq!(z.h, 1 + 3, "one line, two borders, the button row");

        let low = peek("a short footnote", (10, 22)).zone(80, 24);
        assert_eq!(low.y + low.h, 22, "its last row is the one above the mark");
    }

    #[test]
    fn the_box_never_leaves_the_window() {
        let long = "word ".repeat(400);
        for (cols, rows) in [(80u16, 24u16), (30, 10), (12, 5), (4, 3), (1, 1)] {
            for at in [(0u16, 0u16), (cols - 1, rows - 1), (cols / 2, rows / 2)] {
                let z = peek(&long, at).zone(cols, rows);
                assert!(
                    z.x + z.w <= cols && z.y + z.h <= rows,
                    "{cols}x{rows}: {z:?}"
                );
            }
        }
    }

    #[test]
    fn a_long_footnote_is_cut_short_and_says_so() {
        let p = peek(&"word ".repeat(400), (0, 0));
        let lines = p.lines(40, 40);
        assert_eq!(lines.len(), MAX_LINES);
        assert_eq!(lines.last().map(String::as_str), Some("…"));
        assert_eq!(
            peek("", (0, 0)).lines(40, 40).len(),
            1,
            "never a box with no rows"
        );
    }

    /// A short window holds fewer rows than a peek otherwise shows, and the
    /// painter simply dropped the ones that did not fit.
    #[test]
    fn a_footnote_cut_short_by_the_window_says_so_too() {
        let p = peek(&"word ".repeat(60), (0, 0));
        assert!(
            p.lines(40, 40).len() > 5,
            "the fixture needs more rows than fit"
        );
        for rows in [10u16, 8, 6, 5] {
            let lines = p.lines(40, rows);
            let z = p.zone(40, rows);
            assert_eq!(
                lines.len(),
                usize::from(z.h - 3),
                "{rows}: every row is shown"
            );
            assert_eq!(lines.last().map(String::as_str), Some("…"), "{rows}");
        }
    }
}
