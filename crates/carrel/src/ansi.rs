//! The document as *styled* linear text — `--render`, for embedding carrel's
//! reading view in another tool's output.
//!
//! Where [`crate::plain`] is ASCII-safe by policy (screen readers must never
//! meet an escape byte), this module is the opposite contract: attributes and
//! hyperlinks ON, colours OFF. Only SGR *attributes* are emitted — weight,
//! slant, strike, dim — never an SGR colour, because a non-interactive pipe
//! has no palette to inherit and guessing one would fight whatever theme the
//! surrounding tool already chose. `NO_COLOR` strips even those, reducing
//! the output to [`crate::plain`]'s exactly.
//!
//! The row walk is [`crate::plain`]'s skeleton, deliberately duplicated
//! rather than parameterised: the two differ in every line body, and a mode
//! flag threaded through ten match arms would be harder to read than two
//! honest functions. NO RATATUI — rule 6.

use std::collections::HashMap;

use carrel_core::{BlockIdx, Document, Inline, NodeKind, Row, RowKind};

use crate::layout::Layout;

/// Attribute bits this renderer emits, accumulated per segment. Four
/// independent facts are four bools — an enum would invent states that
/// cannot occur (the same call app.rs makes for its toggles).
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
struct Attrs {
    bold: bool,
    italic: bool,
    strikethrough: bool,
    dim: bool,
}

impl Attrs {
    fn from_style(style: carrel_core::Style) -> Self {
        Self {
            bold: style.contains(carrel_core::Style::STRONG),
            italic: style.contains(carrel_core::Style::EMPHASIS),
            strikethrough: style.contains(carrel_core::Style::STRIKETHROUGH),
            // Inline code and math read as secondary material in a stream of
            // prose; dim is the quietest honest mark without a palette.
            dim: style.contains(carrel_core::Style::CODE)
                || style.contains(carrel_core::Style::MATH),
        }
    }

    /// The SGR sequence for these attributes, or "" when none apply.
    fn sgr(self) -> String {
        let mut codes: Vec<u8> = Vec::new();
        if self.bold {
            codes.push(1);
        }
        if self.dim {
            codes.push(2);
        }
        if self.italic {
            codes.push(3);
        }
        if self.strikethrough {
            codes.push(9);
        }
        if codes.is_empty() {
            return String::new();
        }
        let list: Vec<String> = codes.iter().map(ToString::to_string).collect();
        format!("\u{1b}[{}m", list.join(";"))
    }
}

const RESET: &str = "\u{1b}[0m";
const OSC8_CLOSE: &str = "\u{1b}]8;;\u{1b}\\";

fn osc8_open(url: &str) -> String {
    format!("\u{1b}]8;;{url}\u{1b}\\")
}

/// Render the whole document at `width`, attributes on, colours off.
#[must_use]
pub fn render(doc: &Document, width: u16) -> String {
    render_with(
        doc,
        width,
        std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty()),
    )
}

/// The same, with the `NO_COLOR` decision made by the caller — tests inject
/// rather than touch the process environment.
#[must_use]
pub fn render_with(doc: &Document, width: u16, styled: bool) -> String {
    let width = width.max(20);
    // Aligned tables, like plain output: cards exist only on a screen.
    let layout = Layout::with_images(doc, width, HashMap::new(), true);
    let mut out = String::new();
    let mut rows: Vec<Row> = Vec::new();
    let mut pending_blanks = 0u32;

    for i in 0..doc.block_count() {
        let b = BlockIdx(i as u32);
        let node = doc.node_for_block(b);
        let heading_bold = matches!(node.kind, NodeKind::Heading { .. });
        layout.rows_for(doc, b, &mut rows);
        let mut first_text = true;
        for row in &rows {
            match row.kind {
                RowKind::Decoration => pending_blanks += 1,
                RowKind::Text { .. } => {
                    for _ in 0..pending_blanks.min(1) {
                        out.push('\n');
                    }
                    pending_blanks = 0;
                    // Prefix first, unstyled — a bullet needs no weight.
                    if first_text && let Some(p) = &node.prefix {
                        out.push_str(&" ".repeat(row.indent.saturating_sub(p.width) as usize));
                        out.push_str(&p.text);
                    } else {
                        out.push_str(&" ".repeat(row.indent as usize));
                    }
                    let text = &doc.text[row.doc.start as usize..row.doc.end as usize];
                    for seg in segments(
                        text,
                        row.doc.start,
                        node.inlines.iter(),
                        &doc.links,
                        heading_bold,
                    ) {
                        let (bytes, attrs, url) = seg;
                        let body = &text[bytes.start..bytes.end];
                        if !styled {
                            out.push_str(body);
                            continue;
                        }
                        if let Some(url) = &url {
                            out.push_str(&osc8_open(url));
                        }
                        let seq = attrs.sgr();
                        if !seq.is_empty() {
                            out.push_str(&seq);
                        }
                        out.push_str(body);
                        if !seq.is_empty() {
                            out.push_str(RESET);
                        }
                        if url.is_some() {
                            out.push_str(OSC8_CLOSE);
                        }
                    }
                    out.push('\n');
                    first_text = false;
                }
            }
        }
    }
    out
}

/// One styled piece of a row: local byte range, attributes, link URL.
type Seg<'a> = (std::ops::Range<usize>, Attrs, Option<&'a str>);

/// Split one row's text against the block's inline runs.
///
/// Runs are sorted and non-overlapping over the BLOCK's range, so each is
/// clipped to the row and the gaps between them stay plain. A link's URL
/// rides along for the OSC 8 wrapper.
fn segments<'a>(
    text: &'a str,
    row_start: u32,
    inlines: impl Iterator<Item = &'a Inline>,
    links: &'a [Box<str>],
    heading_bold: bool,
) -> Vec<Seg<'a>> {
    let row_end = row_start + text.len() as u32;
    // Headings carry their weight over every segment, styled or not.
    let plain = Attrs {
        bold: heading_bold,
        ..Attrs::default()
    };
    let mut out: Vec<Seg<'a>> = Vec::new();
    let mut cursor = 0usize;
    for inl in inlines {
        if inl.doc.end <= row_start || inl.doc.start >= row_end {
            continue;
        }
        let s = (inl.doc.start.max(row_start) - row_start) as usize;
        let e = (inl.doc.end.min(row_end) - row_start) as usize;
        if s >= e {
            continue;
        }
        if s > cursor {
            out.push((cursor..s, plain, None));
        }
        let mut attrs = Attrs::from_style(inl.style);
        attrs.bold |= heading_bold;
        let url = inl
            .link
            .and_then(|id| links.get(id.0 as usize))
            .map(|b| &**b);
        out.push((s..e, attrs, url));
        cursor = e;
    }
    if cursor < text.len() {
        out.push((cursor..text.len(), plain, None));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = concat!(
        "# Head\n\n",
        "plain **bold** *italic* `code` [text](https://example.com/x) plain\n"
    );

    #[test]
    fn attributes_and_links_are_emitted_colours_never() {
        let doc = Document::parse(SRC);
        let out = render_with(&doc, 80, true);
        assert!(out.contains("\u{1b}[1mHead"), "{out:?}");
        assert!(out.contains("\u{1b}[1mbold\u{1b}[0m"), "strong: {out:?}");
        assert!(out.contains("\u{1b}[3mitalic\u{1b}[0m"), "emph: {out:?}");
        assert!(out.contains("\u{1b}[2mcode\u{1b}[0m"), "code dims: {out:?}");
        assert!(
            out.contains("\u{1b}]8;;https://example.com/x\u{1b}\\text\u{1b}]8;;\u{1b}\\"),
            "osc8 wraps the label: {out:?}"
        );
        // Attributes only: no SGR colour ever appears (30-37 / 90-97 / 38;5).
        for line in out.lines() {
            let codes: Vec<&str> = line
                .split("\u{1b}[")
                .skip(1)
                .filter_map(|c| c.split('m').next())
                .collect();
            for c in codes {
                assert!(
                    c.chars().all(|ch| ch.is_ascii_digit() || ch == ';'),
                    "attribute-only SGR, got {c:?}"
                );
            }
        }
    }

    #[test]
    fn no_color_reduces_this_to_plain_output() {
        let doc = Document::parse(SRC);
        let ansi = render_with(&doc, 80, false);
        let plain = crate::plain::render(&doc, 80);
        // The prefix path differs by construction order only; text identical.
        let strip = |s: &str| s.replace('\u{1b}', "").replace("[1m", "").replace('[', "");
        assert!(!ansi.contains('\u{1b}'));
        assert_eq!(strip(&ansi), strip(&plain));
    }
}
