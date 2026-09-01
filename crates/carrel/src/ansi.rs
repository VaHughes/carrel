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
//! The row walk is [`crate::plain`]'s skeleton, and the two line BODIES are
//! deliberately duplicated rather than parameterised: they differ in every
//! branch, and a mode flag threaded through ten match arms would be harder to
//! read than two honest functions. What opens a row does not differ, so it is
//! not duplicated — [`crate::plain::open_row`] emits the block separator,
//! quote bars, indent and prefix for both, after the copies here had already
//! drifted out from under the `NO_COLOR` promise above. NO RATATUI — rule 6.

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

/// The OSC 8 opener for `url`, with control characters stripped out of the
/// destination.
///
/// Stripping happens HERE rather than at the call site so that no caller can
/// forget it — `render.rs`'s OSC pass has its own copy of this policy, and
/// this collection point had gone without one. It matters because
/// `CommonMark` entity-decodes link destinations: `[c](&#27;]8;;http://evil)`
/// reaches `doc.links` holding a raw ESC, which hands the document's author
/// the rest of the escape sequence, and `&#7;` terminates the OSC early on
/// xterm-family terminals. Reference definitions (`[r]: <&#27;x>`) decode the
/// same way.
fn osc8_open(url: &str) -> String {
    let url: String = url.chars().filter(|c| !c.is_control()).collect();
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
                    // Separator, quote bars, indent and prefix are [`plain`]'s
                    // to decide — see [`crate::plain::open_row`] for why the
                    // duplication this module's header defends stops there.
                    // The prefix comes back unstyled: a bullet needs no weight.
                    let mut line = crate::plain::open_row(
                        &mut out,
                        &mut pending_blanks,
                        node,
                        row,
                        first_text,
                    );
                    let text = &doc.text[row.doc.start as usize..row.doc.end as usize];
                    for seg in segments(
                        text,
                        row.doc.start,
                        node.inlines.iter(),
                        &doc.links,
                        heading_bold,
                    ) {
                        let (bytes, attrs, url) = seg;
                        // The body is document text, so it goes through the
                        // same filter plain output uses: this module owns the
                        // escapes it emits itself and none of the file's.
                        let body = &text[bytes.start..bytes.end];
                        if !styled {
                            crate::plain::push_text(&mut line, body);
                            continue;
                        }
                        if let Some(url) = &url {
                            line.push_str(&osc8_open(url));
                        }
                        let seq = attrs.sgr();
                        if !seq.is_empty() {
                            line.push_str(&seq);
                        }
                        crate::plain::push_text(&mut line, body);
                        if !seq.is_empty() {
                            line.push_str(RESET);
                        }
                        if url.is_some() {
                            line.push_str(OSC8_CLOSE);
                        }
                    }
                    // Trimmed like plain output's lines, which is half of what
                    // `NO_COLOR` byte-identity needs; a styled line ends in a
                    // reset or a link terminator, so the trim finds nothing.
                    out.push_str(line.trim_end());
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
        // A nested blockquote is in the corpus deliberately: quote bars are
        // the concrete way the two walks had already drifted apart, so
        // without one this equality asserts nothing about them.
        let src = format!("{SRC}\n> quoted words here\n>\n> > deeper quote\n\n- item one\n");
        let doc = Document::parse(&src);
        let ansi = render_with(&doc, 80, false);
        let plain = crate::plain::render(&doc, 80);
        assert!(!ansi.contains('\u{1b}'));
        // Byte-identical, with nothing stripped from either side: a `strip`
        // closure that erases `[` would hide exactly the differences this
        // test exists to catch.
        assert_eq!(ansi, plain);
        assert!(plain.contains("> > deeper quote"), "{plain:?}");
    }

    #[test]
    fn control_bytes_in_the_document_never_reach_the_output() {
        let doc = Document::parse(crate::plain::CONTROL_BYTES);
        for styled in [true, false] {
            let out = render_with(&doc, 40, styled);
            // Strip the sequences this module emits ITSELF, then nothing
            // controlling may remain — the document's own escapes are what
            // this is about, and they are indistinguishable once printed.
            let mut rest = out.replace(RESET, "").replace(OSC8_CLOSE, "");
            while let Some(i) = rest.find("\u{1b}]8;;") {
                let end = rest[i..].find("\u{1b}\\").expect("an opener closes");
                rest.replace_range(i..i + end + 2, "");
            }
            for seq in ["\u{1b}[1m", "\u{1b}[2m", "\u{1b}[3m", "\u{1b}[9m"] {
                rest = rest.replace(seq, "");
            }
            for c in rest.chars() {
                assert!(
                    !c.is_control() || c == '\n',
                    "control byte {c:?} reached --render output: {out:?}"
                );
            }
            assert!(out.contains("RED"), "the text itself survives: {out:?}");
        }
    }

    #[test]
    fn an_entity_encoded_escape_in_a_link_destination_is_stripped() {
        // CommonMark entity-decodes link destinations, so these reach
        // `doc.links` holding a raw ESC or BEL — an author-controlled escape
        // sequence in the middle of our OSC 8 opener.
        for src in [
            "[c](&#27;]8;;http://evil)\n",
            "[b](&#7;bel)\n",
            "[r]\n\n[r]: <&#27;x>\n",
        ] {
            let doc = Document::parse(src);
            assert!(
                doc.links.iter().any(|u| u.chars().any(char::is_control)),
                "the corpus must reach us holding a control byte: {:?}",
                doc.links
            );
            let out = render_with(&doc, 80, true);
            let opener = "\u{1b}]8;;";
            let i = out.find(opener).expect("an OSC 8 opener");
            let url = &out[i + opener.len()..];
            let url = &url[..url.find("\u{1b}\\").expect("its terminator")];
            assert!(
                !url.chars().any(char::is_control),
                "destination smuggled a control byte from {src:?}: {url:?}"
            );
        }
    }
}
