//! The document as plain, linear text — the accessible rendering (Q17).
//!
//! Linux terminal screen readers (Orca via AT-SPI, BRLTTY) read output in
//! insertion order; a cell-repainting TUI cannot express itself in that
//! model. This module renders the SAME reflow rows the screen paints as
//! honest lines: `carrel --plain doc.md | less` is the screen-reader path,
//! and a bare pipe gets it by default.
//!
//! ASCII-safe by policy: quote bars are `> `, no box drawing, and never an
//! escape byte. Tables stay aligned (never cards — card gutter labels are
//! paint decoration, which plain output by definition omits). NO RATATUI —
//! `scripts/check-discipline.sh` rule 6.

use std::collections::HashMap;
use std::fmt::Write as _;

use carrel_core::{BlockIdx, Document, Row, RowKind};

use crate::layout::Layout;

/// Render the whole document at `width` as plain text lines.
#[must_use]
pub fn render(doc: &Document, width: u16) -> String {
    let width = width.max(20);
    // wrap_tables = true: aligned tables; a card's labels exist only in paint.
    let layout = Layout::with_images(doc, width, HashMap::new(), true);
    let mut out = String::new();
    let mut rows: Vec<Row> = Vec::new();
    let mut pending_blanks = 0u32;

    for i in 0..doc.block_count() {
        let b = BlockIdx(i as u32);
        let node = doc.node_for_block(b);
        layout.rows_for(doc, b, &mut rows);
        let mut first_text = true;
        for row in &rows {
            match row.kind {
                RowKind::Decoration => pending_blanks += 1,
                RowKind::Text { .. } => {
                    // Blank separators only BETWEEN content, never trailing.
                    for _ in 0..pending_blanks.min(1) {
                        out.push('\n');
                    }
                    pending_blanks = 0;
                    let mut line = String::new();
                    // Quote bars, speech-friendly, at their recorded columns.
                    for &col in &node.quote_cols {
                        while line.len() < col as usize {
                            line.push(' ');
                        }
                        line.push_str("> ");
                    }
                    while line.len() < row.indent as usize {
                        line.push(' ');
                    }
                    // The prefix occupies the tail of the first row's inset,
                    // exactly as the painter places it.
                    if first_text && let Some(p) = &node.prefix {
                        let at = row.indent.saturating_sub(p.width) as usize;
                        line.truncate(at);
                        line.push_str(&p.text);
                    }
                    let _ = write!(
                        line,
                        "{}",
                        &doc.text[row.doc.start as usize..row.doc.end as usize]
                    );
                    out.push_str(line.trim_end());
                    out.push('\n');
                    first_text = false;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_output_carries_structure_and_never_an_escape_byte() {
        let doc = Document::parse(
            "# Title\n\n- first item long enough to wrap at a narrow width for sure\n\n> quoted words\n\n> [!NOTE]\n> advice\n\n![a chart](x.png)\n",
        );
        let s = render(&doc, 30);
        assert!(s.contains("Title"), "{s}");
        assert!(s.contains("- first item"), "{s}");
        assert!(s.contains("> quoted words"), "{s}");
        assert!(s.contains("Note"), "alert label spoken: {s}");
        assert!(s.contains("a chart"), "alt text spoken: {s}");
        assert!(!s.contains('\u{1b}'), "no escape bytes, ever");
        assert!(!s.contains('│'), "ASCII-safe quoting");

        // The wrapped item's continuation hangs under its text.
        let lines: Vec<&str> = s.lines().collect();
        let item = lines.iter().position(|l| l.starts_with("- ")).unwrap();
        assert!(
            lines[item + 1].starts_with("  ") && !lines[item + 1].trim().is_empty(),
            "hanging indent survives: {:?}",
            &lines[item..=item + 1]
        );
    }

    #[test]
    fn blank_lines_separate_blocks_but_never_pile_up() {
        let doc = Document::parse("one\n\ntwo\n\nthree\n");
        let s = render(&doc, 40);
        assert_eq!(
            s, "one\n\ntwo\n\nthree\n",
            "exactly one blank between blocks"
        );
    }
}
