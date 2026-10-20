//! A peek: a footnote's text, shown where its mark is.
//!
//! Following a footnote means leaving the sentence you were in the middle
//! of, reading one line at the bottom of the document, and finding your way
//! back. `%` does that and `Back` returns, which is correct and is three
//! steps for a parenthesis. A peek is the parenthesis: click the mark, read
//! the text in place, carry on.
//!
//! It is not a pane. It owns no keyboard and blocks nothing: whatever the
//! reader does next closes it and then happens. The two things it offers
//! are the long way round — `Enter`, or its button, goes to the footnote
//! itself — and the wheel over the box, which scrolls a footnote too long
//! for it.
//!
//! **It shows the whole footnote**: every paragraph, list, quote and code
//! block in the definition, wrapped by the core's own `wrap_with` at the
//! box's width and in the page's hyphenation, so every row is a slice of
//! `Document::text` and the search hits, links and footnote marks inside it
//! are the document's own. What is painted that is not in the text is
//! decoration only — list markers, quote bars, the code continuation mark,
//! the `-` or `↩` that ends a row cut mid-word, the label on the border
//! and the "more" count — exactly as in the reader.
//!
//! The hyphenation is passed in, never kept: it is the reader's preference
//! and the document's language, asked of `App::hyphenation` at each call
//! as the page's own layout asks it.
//!
//! Its scroll position is a doc byte (`top`), never a row: rows are derived
//! from `(document, width)` here as everywhere else.
//!
//! NO RATATUI — `scripts/check-discipline.sh` rule 6. The geometry is in
//! terminal cells, as [`crate::menu`]'s is, and nothing here draws.

use std::ops::Range;

use carrel_core::{BlockIdx, DocByte, Document, Hyphenation, Row, cluster_width, wrap_with};

use crate::action::Zone;

/// The most rows of footnote a peek shows before it scrolls.
pub const MAX_LINES: usize = 12;
/// As wide as a comfortable line, and no wider than the window allows.
const MAX_W: u16 = 64;
/// The window's last column: the scrollbar, which the box never covers.
const SCROLLBAR: u16 = 1;

/// An open peek.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Peek {
    /// The cell the mark was clicked at.
    pub at: (u16, u16),
    /// Where the footnote's own text starts, for "go to it".
    pub def: u32,
    /// The whole definition, in doc space.
    pub span: Range<u32>,
    /// The label, as written: `[^name]`.
    pub label: String,
    /// A byte of the first row shown. Starts at `def`; the wheel moves it.
    pub top: u32,
}

/// One row of the box: a wrapped row of the footnote, or the blank between
/// two of its blocks.
#[derive(Clone, Debug)]
pub struct Line {
    pub row: Option<Row>,
    /// For a row of drawn math: which row of the art it is. The rows of a
    /// drawing are all the same empty slice at the block's start, as on the
    /// page, so only this says which line of the art goes here.
    pub nth: u16,
}

impl Line {
    const GAP: Self = Self { row: None, nth: 0 };

    fn row(row: Row) -> Self {
        Self {
            row: Some(row),
            nth: 0,
        }
    }

    /// A row the box may start at: text, a card rule, or the FIRST row of a
    /// drawing — never a gap, and never partway down a drawing, whose rows
    /// share one slice and so one scroll position.
    fn is_top(&self) -> bool {
        self.row.is_some() && self.nth == 0
    }
}

/// How tall a block that is drawn rather than wrapped is — display math,
/// whose height is its art's, not its text's — at `avail` cells. `None`
/// for a block wrapped as text. The page answers this from its own state
/// (`App`); the box asks the same question so the two draw alike.
pub trait Drawn {
    fn drawn_rows(&self, block: BlockIdx, avail: u16) -> Option<u32>;
}

/// What the box is laid out with: the page's hyphenation, and what it draws.
#[derive(Clone, Copy)]
pub struct Fit<'a> {
    pub hy: Hyphenation,
    pub drawn: Option<&'a dyn Drawn>,
}

impl std::fmt::Debug for Fit<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fit")
            .field("hy", &self.hy)
            .field("drawn", &self.drawn.is_some())
            .finish()
    }
}

impl From<Hyphenation> for Fit<'_> {
    fn from(hy: Hyphenation) -> Self {
        Self { hy, drawn: None }
    }
}

/// What the box shows at one size.
#[derive(Clone, Debug)]
pub struct View {
    /// The rows on screen, top to bottom. Never empty.
    pub lines: Vec<Line>,
    /// Rows of footnote above the first one shown.
    pub above: usize,
    /// Rows of footnote below the last one shown.
    pub below: usize,
}

impl Peek {
    /// A peek of the definition starting at `def`, opened at cell `at`.
    #[must_use]
    pub fn new(doc: &Document, at: (u16, u16), def: u32, label: String) -> Self {
        let span = doc.footnote_def_range(def).unwrap_or_else(|| {
            // A definition the parse did not record: its own block, at least.
            let n = doc.node_for_block(doc.block_at_doc(DocByte(def)));
            n.doc.start..n.doc.end
        });
        Self {
            at,
            def,
            span,
            label,
            top: def,
        }
    }

    /// The blocks the definition is made of.
    fn blocks(&self, doc: &Document) -> Range<u32> {
        let first = doc.block_at_doc(DocByte(self.span.start)).0;
        let last = doc
            .block_at_doc(DocByte(
                self.span.end.saturating_sub(1).max(self.span.start),
            ))
            .0;
        first..last.max(first) + 1
    }

    /// The first block — the one wearing the `[^name]: ` label, which the
    /// box shows on its border instead.
    #[must_use]
    pub fn first_block(&self, doc: &Document) -> BlockIdx {
        BlockIdx(self.blocks(doc).start)
    }

    /// How far the definition sits in from the page: its label's width plus
    /// whatever encloses it. The box takes it off every row, so the footnote
    /// starts at the box's own edge and a list inside it keeps its nesting.
    #[must_use]
    pub fn inset(&self, doc: &Document) -> u16 {
        doc.node_for_block(self.first_block(doc)).indent
    }

    /// Every row of the footnote for text `inner` cells wide, blocks
    /// separated by one blank row — the core's `wrap_with`, dividing words
    /// as `hy` says, so each row is a slice of the text. Stops at a block
    /// boundary once `limit` rows are in hand.
    #[must_use]
    pub fn rows<'a>(
        &self,
        doc: &Document,
        fit: impl Into<Fit<'a>>,
        inner: u16,
        limit: usize,
    ) -> Vec<Line> {
        let Fit { hy, drawn } = fit.into();
        let inset = self.inset(doc);
        let width = inner.max(1).saturating_add(inset);
        let blocks = self.blocks(doc);
        let mut out = Vec::new();
        for b in blocks.clone() {
            if out.len() >= limit {
                break;
            }
            if b != blocks.start {
                out.push(Line::GAP);
            }
            // Display math the page draws is drawn here too: its rows are
            // the art's, each the block's empty start, as on the page.
            let node = doc.node_for_block(BlockIdx(b));
            let avail = width.saturating_sub(node.indent);
            if let Some(n) = drawn.and_then(|d| d.drawn_rows(BlockIdx(b), avail)) {
                for nth in 0..n.max(1) {
                    out.push(Line {
                        row: Some(Row {
                            block: BlockIdx(b),
                            doc: node.doc.start..node.doc.start,
                            indent: node.indent,
                            kind: carrel_core::RowKind::Decoration,
                        }),
                        nth: u16::try_from(nth).unwrap_or(u16::MAX),
                    });
                }
                continue;
            }
            let mut push = |r| out.push(Line::row(r));
            // A table the box is too narrow for is cards, as it would be
            // on a page this wide; wrapped as prose, its padded columns
            // flowed into misaligned text.
            if crate::layout::table_overflows(doc.node_for_block(BlockIdx(b)), width) {
                crate::layout::card_rows(doc, BlockIdx(b), width, &mut push);
            } else {
                wrap_with(doc, BlockIdx(b), width, &cluster_width, hy, &mut push);
            }
        }
        out
    }

    /// The box's width in a `cols`-wide window, and the width of the text
    /// inside it (two borders and a cell of air each side). The window's
    /// last column is the scrollbar's, which the box leaves showing.
    #[must_use]
    pub fn widths(cols: u16) -> (u16, u16) {
        let w = MAX_W.min(cols.saturating_sub(3)).max(1);
        (w, w.saturating_sub(4).max(1))
    }

    /// How many rows of text a `rows`-tall window leaves the box. Two
    /// borders and the button's row are not text.
    fn room(rows: u16) -> usize {
        MAX_LINES.min(usize::from(rows.saturating_sub(3))).max(1)
    }

    /// The index of the row `top` falls on.
    fn top_index(&self, lines: &[Line]) -> usize {
        lines
            .iter()
            .position(|l| {
                l.is_top()
                    && l.row
                        .as_ref()
                        .is_some_and(|r| r.doc.end > self.top || r.doc.start >= self.top)
            })
            .or_else(|| lines.iter().rposition(Line::is_top))
            .unwrap_or(0)
    }

    /// What the box shows in a `cols × rows` window.
    #[must_use]
    pub fn view<'a>(&self, doc: &Document, fit: impl Into<Fit<'a>>, cols: u16, rows: u16) -> View {
        let (_, inner) = Self::widths(cols);
        let room = Self::room(rows);
        let all = self.rows(doc, fit, inner, usize::MAX);
        let first = self.top_index(&all).min(all.len().saturating_sub(room));
        let shown = all.len().saturating_sub(first).min(room);
        let below = all.len() - first - shown;
        let mut lines: Vec<Line> = all.into_iter().skip(first).take(shown).collect();
        if lines.is_empty() {
            lines.push(Line::GAP);
        }
        View {
            lines,
            above: first,
            below,
        }
    }

    /// Move the box's text by `delta` rows, never past either end. A blank
    /// between blocks is never the top row, so a step that lands on one
    /// goes one further.
    pub fn scroll<'a>(
        &mut self,
        doc: &Document,
        fit: impl Into<Fit<'a>>,
        delta: i32,
        cols: u16,
        rows: u16,
    ) {
        let (_, inner) = Self::widths(cols);
        let all = self.rows(doc, fit, inner, usize::MAX);
        let max = all.len().saturating_sub(Self::room(rows));
        let now = self.top_index(&all).min(max);
        let to = now
            .saturating_add_signed(isize::try_from(delta).unwrap_or(0))
            .min(max);
        if let Some(r) = nearest_row(&all, to, delta < 0).and_then(|i| all[i].row.as_ref()) {
            self.top = r.doc.start;
        }
    }

    /// The box, clamped into a `cols × rows` viewport: under the mark, or
    /// above it when there is no room below — the menu's rule, for the
    /// menu's reason.
    #[must_use]
    pub fn zone<'a>(&self, doc: &Document, fit: impl Into<Fit<'a>>, cols: u16, rows: u16) -> Zone {
        let (w, inner) = Self::widths(cols);
        let room = Self::room(rows);
        let shown = self.rows(doc, fit, inner, room).len().clamp(1, room);
        // Borders, the text, and the row the button sits on.
        let h = u16::try_from(shown)
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
            cx.min(cols.saturating_sub(w.saturating_add(SCROLLBAR))),
            y.min(rows.saturating_sub(h)),
            w,
            h,
        )
    }
}

/// The row nearest `to` that is text and not a blank between blocks: the
/// first one in the direction of travel (`back` for upward), else the first
/// the other way, else `None` when there is none at all.
///
/// Two bounded scans, never a walk that turns round: the loop this replaces
/// stepped back from a blank and forward again from index 0, and only the
/// core's "every block is at least one row" kept two blanks from ever
/// opening a footnote and bouncing it forever.
fn nearest_row(lines: &[Line], to: usize, back: bool) -> Option<usize> {
    let to = to.min(lines.len().checked_sub(1)?);
    let is_row = |i: &usize| lines[*i].is_top();
    let behind = (0..=to).rev().find(is_row);
    let ahead = (to..lines.len()).find(is_row);
    if back {
        behind.or(ahead)
    } else {
        ahead.or(behind)
    }
}

/// What the button row says about rows the box is not showing, or nothing
/// when it shows them all.
#[must_use]
pub fn more(view: &View) -> Option<String> {
    match (view.above, view.below) {
        (0, 0) => None,
        (above, 0) => Some(format!("\u{2191} {above} above")),
        (_, below) => Some(format!("\u{2193} {below} more")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc_with(footnote: &str) -> Document {
        Document::parse(&format!("Body[^n].\n\n[^n]: {footnote}\n\nAfter.\n"))
    }

    fn peek(doc: &Document, at: (u16, u16)) -> Peek {
        let (_, def) = doc.footnote_defs()[0].clone();
        Peek::new(doc, at, def, "[^n]".into())
    }

    fn texts(doc: &Document, v: &View) -> Vec<String> {
        v.lines
            .iter()
            .map(|l| {
                l.row.as_ref().map_or(String::new(), |r| {
                    doc.text[r.doc.start as usize..r.doc.end as usize].to_string()
                })
            })
            .collect()
    }

    #[test]
    fn the_box_sits_under_its_mark_and_flips_above_at_the_bottom() {
        let doc = doc_with("a short footnote");
        let z = peek(&doc, (10, 3)).zone(&doc, Hyphenation::Off, 80, 24);
        assert_eq!((z.x, z.y), (10, 4));
        assert_eq!(z.h, 1 + 3, "one line, two borders, the button row");

        // Pushed in from the right edge, it stops short of the scrollbar's
        // column rather than painting its border over it.
        let right = peek(&doc, (79, 3)).zone(&doc, Hyphenation::Off, 80, 24);
        assert_eq!(right.x + right.w, 79, "{right:?}");

        let low = peek(&doc, (10, 22)).zone(&doc, Hyphenation::Off, 80, 24);
        assert_eq!(low.y + low.h, 22, "its last row is the one above the mark");
    }

    #[test]
    fn the_box_never_leaves_the_window() {
        let doc = doc_with(&"word ".repeat(400));
        for (cols, rows) in [(80u16, 24u16), (30, 10), (12, 5), (4, 3), (1, 1)] {
            for at in [(0u16, 0u16), (cols - 1, rows - 1), (cols / 2, rows / 2)] {
                let z = peek(&doc, at).zone(&doc, Hyphenation::Off, cols, rows);
                assert!(
                    z.x + z.w <= cols && z.y + z.h <= rows,
                    "{cols}x{rows}: {z:?}"
                );
            }
        }
    }

    /// The known limit this lifts: the box showed the first paragraph and
    /// nothing of the rest.
    #[test]
    fn every_block_of_the_footnote_is_in_the_box() {
        let doc = doc_with(
            "First paragraph.\n\n    Second paragraph.\n\n    - one\n    - two\n\n    \
             > quoted\n\n    ```\n    code line\n    ```",
        );
        let p = peek(&doc, (0, 0));
        let v = p.view(&doc, Hyphenation::Off, 80, 40);
        assert_eq!((v.above, v.below), (0, 0));
        let t = texts(&doc, &v);
        for part in [
            "First paragraph.",
            "Second paragraph.",
            "one",
            "two",
            "quoted",
            "code line",
        ] {
            assert!(t.iter().any(|l| l == part), "{part:?} not a row of {t:?}");
        }
        assert!(!t.iter().any(|l| l.contains("After")), "{t:?}");
        // Blocks keep their blank between them, and only between them.
        assert_eq!(t[1], "", "{t:?}");
        assert!(!t.last().unwrap().is_empty(), "{t:?}");
        // The definition's own inset is what the painter takes off: its
        // first row starts at the box's edge, and the list inside it keeps
        // its marker's room.
        let inset = p.inset(&doc);
        assert_eq!(v.lines[0].row.as_ref().unwrap().indent, inset);
        let item = v
            .lines
            .iter()
            .filter_map(|l| l.row.as_ref())
            .find(|r| &doc.text[r.doc.start as usize..r.doc.end as usize] == "one")
            .unwrap();
        assert!(item.indent > inset, "the list hangs under its marker");
    }

    /// Rows are derived at the box's width, so a narrow window wraps the
    /// footnote narrower rather than clipping it.
    #[test]
    fn a_narrow_box_wraps_every_word_in() {
        let words = "alpha beta gamma delta epsilon zeta eta theta iota kappa";
        let doc = doc_with(words);
        let p = peek(&doc, (0, 0));
        let (_, inner) = Peek::widths(30);
        let v = p.view(&doc, Hyphenation::Off, 30, 40);
        let t = texts(&doc, &v);
        assert!(t.len() > 1, "{t:?}");
        for l in &t {
            assert!(
                carrel_core::display_width(l) <= inner,
                "{l:?} wider than {inner}"
            );
        }
        assert_eq!(t.join(" "), words);
    }

    #[test]
    fn a_long_footnote_scrolls_and_says_how_much_is_left() {
        let body = (0..500)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n\n    ");
        let doc = doc_with(&body);
        let mut p = peek(&doc, (0, 0));
        let v = p.view(&doc, Hyphenation::Off, 80, 40);
        assert_eq!(v.lines.len(), MAX_LINES);
        assert_eq!(v.above, 0);
        assert!(v.below > 900, "{}", v.below);
        assert_eq!(more(&v), Some(format!("\u{2193} {} more", v.below)));

        p.scroll(&doc, Hyphenation::Off, 1, 80, 40);
        let v = p.view(&doc, Hyphenation::Off, 80, 40);
        assert_eq!(texts(&doc, &v)[0], "line 1", "a blank is never the top row");
        p.scroll(&doc, Hyphenation::Off, -1, 80, 40);
        assert_eq!(
            texts(&doc, &p.view(&doc, Hyphenation::Off, 80, 40))[0],
            "line 0"
        );

        // To the end and no further: the last row is the footnote's last.
        p.scroll(&doc, Hyphenation::Off, 10_000, 80, 40);
        let v = p.view(&doc, Hyphenation::Off, 80, 40);
        assert_eq!(v.below, 0);
        assert_eq!(texts(&doc, &v).last().map(String::as_str), Some("line 499"));
        assert_eq!(more(&v), Some(format!("\u{2191} {} above", v.above)));
        p.scroll(&doc, Hyphenation::Off, -10_000, 80, 40);
        assert_eq!(p.top, p.def);
    }

    /// The step off a blank always lands, whatever the blanks around it.
    /// Hand-built, because the core never wraps a block to zero rows and so
    /// no document can open a footnote with two blanks — the old loop
    /// bounced 0 → 1 → 0 forever on exactly that, upward, and was only safe
    /// by that coincidence. Run on a thread with a deadline, so a regression
    /// fails instead of hanging the suite.
    #[test]
    fn the_step_off_a_blank_always_lands() {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let doc = doc_with("x");
            let mut v = Vec::new();
            carrel_core::wrap(&doc, BlockIdx(0), 80, &cluster_width, |r| v.push(r));
            let row = || Line::row(v[0].clone());
            let gap = || Line::GAP;
            let lines = vec![gap(), gap(), row(), gap(), gap(), row(), gap()];
            for (to, back, want) in [
                (0, true, Some(2)),
                (1, true, Some(2)),
                (1, false, Some(2)),
                (4, true, Some(2)),
                (4, false, Some(5)),
                (6, false, Some(5)),
                (99, false, Some(5)),
                (99, true, Some(5)),
            ] {
                assert_eq!(nearest_row(&lines, to, back), want, "{to} {back}");
            }
            assert_eq!(nearest_row(&[gap(), gap()], 0, true), None);
            assert_eq!(nearest_row(&[], 3, true), None);
            tx.send(()).unwrap();
        });
        rx.recv_timeout(std::time::Duration::from_secs(10))
            .expect("did not return");
    }

    /// A short window holds fewer rows than a peek otherwise shows, and the
    /// rest is still counted.
    #[test]
    fn a_footnote_cut_short_by_the_window_says_so_too() {
        let doc = doc_with(&"word ".repeat(60));
        let p = peek(&doc, (0, 0));
        for rows in [10u16, 8, 6, 5] {
            let v = p.view(&doc, Hyphenation::Off, 40, rows);
            let z = p.zone(&doc, Hyphenation::Off, 40, rows);
            assert_eq!(
                v.lines.len(),
                usize::from(z.h - 3),
                "{rows}: every row is shown"
            );
            assert!(v.below > 0, "{rows}");
        }
    }
}
