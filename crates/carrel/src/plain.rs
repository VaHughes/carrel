//! The document as plain, linear text — the accessible rendering (Q17).
//!
//! Linux terminal screen readers (Orca via AT-SPI, BRLTTY) read output in
//! insertion order; a cell-repainting TUI cannot express itself in that
//! model. This module renders the SAME reflow rows the screen paints as
//! honest lines: `carrel --plain doc.md | less` is the screen-reader path,
//! and a bare pipe gets it by default.
//!
//! ASCII-safe by policy: quote bars are `> `, no box drawing, and never an
//! escape byte — nor any other control character, which [`push_text`] strips
//! from the document's own text on the way out. Tables stay aligned (never
//! cards — card gutter labels are paint decoration, which plain output by
//! definition omits). NO RATATUI — `scripts/check-discipline.sh` rule 6.

use std::collections::HashMap;

use carrel_core::{BlockIdx, Document, Node, Row, RowKind};

use crate::layout::Layout;

/// Append document text to an output line with control characters removed.
///
/// The contract above promises never an escape byte, but what this module
/// copies out is the DOCUMENT's own text, and a raw ESC in a markdown file
/// survives `Document::parse` into `Document::text` untouched. The screen is
/// safe only incidentally — `ratatui::text::Span` drops control characters —
/// and a pipe inherits nothing of the sort, so without this a file you merely
/// READ could set the title of the terminal `--plain` was piped into, or
/// repaint it. The filter lives at the single point where document text
/// becomes an output line, rather than at each call site that might forget;
/// [`crate::ansi`] copies the same text out and shares it.
///
/// Tabs need no exception here: `carrel_core`'s `expand_tabs` turns them into
/// spaces before they can reach `Document::text`. Newlines are kept, because
/// nothing later in the walk would put back a line ending that a row's own
/// text carried.
pub(crate) fn push_text(out: &mut String, text: &str) {
    out.extend(text.chars().filter(|c| !c.is_control() || *c == '\n'));
}

/// Open one text row: push the block separator into `out`, and return the
/// opening of the line itself — quote bars, indent, and the list prefix.
///
/// [`crate::ansi`] walks these same rows, and its header defends duplicating
/// the walk on the grounds that the two differ in every line BODY. They do —
/// but this part of it does not, and the copies had already drifted: that
/// module grew no quote bars at all, while documenting its `NO_COLOR` output
/// as reducing to this module's exactly. Only the identical part is shared;
/// the two body loops stay separate, as that argument intends.
///
/// The separator goes into `out` while the opening comes back as a value
/// because the caller trims each finished line's tail, and a blank separator
/// must survive that trim — it belongs to the output, not to the line.
pub(crate) fn open_row(
    out: &mut String,
    pending_blanks: &mut u32,
    node: &Node,
    row: &Row,
    first_text: bool,
) -> String {
    // Blank separators only BETWEEN content, never trailing, and never piled.
    if *pending_blanks > 0 {
        out.push('\n');
    }
    *pending_blanks = 0;
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
    // The prefix occupies the tail of the first row's inset, exactly as the
    // painter places it. It is document text too — a footnote definition's
    // label is `[^name]: ` with the name taken from the file — so it is
    // filtered like any other.
    if first_text && let Some(p) = &node.prefix {
        let at = row.indent.saturating_sub(p.width) as usize;
        line.truncate(at);
        push_text(&mut line, &p.text);
    }
    line
}

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
                    let mut line = open_row(&mut out, &mut pending_blanks, node, row, first_text);
                    push_text(
                        &mut line,
                        &doc.text[row.doc.start as usize..row.doc.end as usize],
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

/// The `--tasks` report: one checkbox line per GFM task, in reading order,
/// ASCII-safe exactly like the rest of this module. A reader's answer to
/// "what is left to do in here", without ever writing to the file.
#[must_use]
pub fn task_report(doc: &carrel_core::Document) -> String {
    let mut out = String::new();
    for t in doc.tasks() {
        let node = doc.node_for_block(doc.block_at_doc(carrel_core::DocByte(t.at)));
        let first = doc.text[node.doc.start as usize..node.doc.end as usize]
            .lines()
            .next()
            .unwrap_or("")
            .trim();
        out.push_str(if t.done { "- [x] " } else { "- [ ] " });
        out.push_str(first);
        out.push('\n');
    }
    out
}

/// A corpus that actually CONTAINS control bytes, shared with [`crate::ansi`]
/// because both exporters copy the same document text out.
///
/// Every existing `!contains('\u{1b}')` assertion in this crate ran over a
/// corpus with no escape byte anywhere in it, so it passed whatever the
/// renderer did with one. A raw ESC survives `Document::parse` into
/// `Document::text`; a footnote's label — which the document names — reaches
/// the output through `node.prefix.text` rather than through a row, so both
/// paths are exercised here.
#[cfg(test)]
pub(crate) const CONTROL_BYTES: &str = concat!(
    "# t\n\n",
    "\u{1b}[31mRED\u{1b}[0m \u{7}bell \u{1b}]0;PWNED\u{7}\n\n",
    "- \u{0}nul item\n\n",
    "> \u{1b}]8;;http://evil\u{1b}\\quoted\n\n",
    "| a\u{1b}b | c |\n| --- | --- |\n| \u{7}d | e |\n\n",
    "text[^e\u{1b}x]\n\n[^e\u{1b}x]: the definition\n",
);

#[cfg(test)]
mod task_report_tests {
    #[test]
    fn the_report_is_checkbox_lines_in_reading_order() {
        let doc = carrel_core::Document::parse("- a\n- [ ] open\n- [x] done\n");
        let out = super::task_report(&doc);
        assert_eq!(out, "- [ ] open\n- [x] done\n", "{out}");
    }
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
    fn control_bytes_in_the_document_never_reach_the_output() {
        let doc = Document::parse(crate::plain::CONTROL_BYTES);
        assert!(
            doc.text.contains('\u{1b}'),
            "the corpus must actually carry an escape byte, or this proves nothing"
        );
        let s = render(&doc, 40);
        for c in s.chars() {
            assert!(
                !c.is_control() || c == '\n',
                "control byte {c:?} reached plain output: {s:?}"
            );
        }
        // The surrounding text is kept — this filters bytes, it does not drop
        // the line that carried them.
        assert!(s.contains("RED"), "{s:?}");
        assert!(s.contains("nul item"), "{s:?}");
        assert!(s.contains("the definition"), "{s:?}");
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
