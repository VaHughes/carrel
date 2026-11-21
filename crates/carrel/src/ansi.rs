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
//! not duplicated — `crate::plain::open_row` emits the block separator,
//! quote bars, indent and prefix for both, after the copies here had already
//! drifted out from under the `NO_COLOR` promise above. NO RATATUI — rule 6.

use carrel_core::{
    BlockIdx, Division, Document, Hyphenation, Inline, NodeKind, Row, RowKind, hyphenation_for,
};

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

/// A link destination in the only form an OSC 8 sequence may carry: bytes
/// 33–126, everything else percent-encoded. Both hyperlink writers — this
/// module's and the reader's post-draw pass — come through here, so neither
/// can hold a policy the other lacks.
///
/// Two steps, in this order:
///
/// 1. [`carrel_state::clip::line`], the rule for what `copy link` puts on the
///    clipboard, so the destination a terminal opens is the one a copy
///    carries. It drops every control character — `CommonMark` entity-decodes
///    destinations, so `[c](&#27;]8;;http://evil)` reaches `doc.links`
///    holding a raw ESC, and `&#7;` ends the sequence early on xterm-family
///    terminals — and the bidi overrides and isolates, which belong in no
///    address and exist in one only to mislead whoever reads it.
/// 2. Percent-encoding of what is left outside printable ASCII, as UTF-8
///    bytes. The specification allows nothing else in the field, and it is
///    what a browser would send for the same address, so `https://例え.jp/ü`
///    still opens the page it names. A space is encoded too: it is legal in
///    the field and ends the address in more than one terminal.
///
/// `;` and `:` pass: the parameter field is closed by the second `;` this
/// module writes itself, before the destination starts, so nothing in the
/// destination can add a parameter.
#[must_use]
pub fn osc8_target(url: &str) -> String {
    use std::fmt::Write as _;
    let clean = carrel_state::clip::line(url);
    let mut out = String::with_capacity(clean.len());
    for b in clean.bytes() {
        if b.is_ascii_graphic() {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// A local path as a `file://` URI for an OSC 8 sequence.
///
/// A path is not an address: `#`, `?` and `%` in a file name are the name,
/// and written bare they would be read as a fragment, a query and an escape.
/// Everything outside the unreserved set and `/` is encoded, byte for byte,
/// so a name that is not UTF-8 survives too.
///
/// Always the ABSOLUTE path, made so against the working directory
/// ([`std::path::absolute`]: no symlink is followed and no `..` folded, so
/// it is the path the reader itself would open). What follows `file://` is
/// a host: a document opened as `carrel notes.md` resolves its wikilinks to
/// `plain.md`, and `file://plain.md` names a machine called `plain.md` and
/// no file at all.
#[must_use]
pub fn osc8_file_uri(path: &std::path::Path) -> String {
    use std::fmt::Write as _;
    // Fails only for an empty path, which then encodes to an empty one.
    let absolute = std::path::absolute(path);
    let path = absolute.as_deref().unwrap_or(path);
    let mut out = String::from("file://");
    for &b in path.as_os_str().as_encoded_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// The `file://` URI of `file`, the file a relative link `dest` was found
/// to name, with the destination's `#fragment` kept.
#[must_use]
pub fn osc8_local(file: &std::path::Path, dest: &str) -> String {
    let mut uri = osc8_file_uri(file);
    if let Some((_, fragment)) = dest.split_once('#')
        && !fragment.is_empty()
    {
        uri.push('#');
        uri.push_str(&osc8_target(fragment));
    }
    uri
}

/// The OSC 8 opener for a destination [`link_targets`] prepared.
fn osc8_open(target: &str) -> String {
    format!("\u{1b}]8;;{target}\u{1b}\\")
}

/// What each of the document's links is to a terminal, by link id: the
/// destination in the form the sequence may carry, or `None` for a link
/// that gets no hyperlink at all. Made HERE, once, for every link, so that
/// no path to the output can forget the cleaning.
///
/// The screen's rule (`render::paint_row`), for a page with no library:
///
/// - a destination with a scheme is itself, through [`osc8_target`];
/// - a relative path is the `file://` URI of the file it names beside
///   `file`, fragment kept ([`osc8_local`]) — when there is one, and it
///   does not leave the document's folder, which is as far as a printed
///   page looks (`app::local_link`);
/// - a `[[wikilink]]` holds a note's name, which is not an address: it is
///   the URI of the note when the note stands beside `file`
///   ([`crate::wiki`], the reader's own rule, without the library index);
/// - anything else gets nothing: `#heading`, a file that is not there, any
///   local link at all when there is no `file` to stand beside. A terminal
///   handed `gone.md` can do nothing with it.
fn link_targets(doc: &Document, file: Option<&std::path::Path>) -> Vec<Option<String>> {
    let dir = file.map(|f| match f.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => std::path::Path::new("."),
    });
    // Absolute, as the folder rule compares paths.
    let here = dir.map(|dir| {
        dir.canonicalize()
            .or_else(|_| std::path::absolute(dir))
            .unwrap_or_else(|_| dir.to_path_buf())
    });
    let mut notes = dir.map(|dir| crate::wiki::Resolver::new(dir, &[]));
    doc.links
        .iter()
        .enumerate()
        .map(|(i, dest)| {
            let id = carrel_core::LinkId(u32::try_from(i).unwrap_or(u32::MAX));
            let target = if doc.is_wikilink(id) {
                let note = dest.split('#').next().filter(|n| !n.is_empty())?;
                osc8_local(&notes.as_mut()?.resolve(note)?, dest)
            } else if crate::app::has_scheme(dest) {
                osc8_target(dest)
            } else {
                let here = here.as_deref()?;
                let bare = dest.split('#').next().filter(|b| !b.is_empty())?;
                match crate::app::local_link(here, Some(here), bare) {
                    crate::app::LocalLink::File(found) => osc8_local(&found, dest),
                    _ => return None,
                }
            };
            (!target.is_empty()).then_some(target)
        })
        .collect()
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

/// [`render`], for the document at `file` (`None` for a pipe): see
/// [`render_for`] for what the file is needed for.
#[must_use]
pub fn render_file(doc: &Document, width: u16, file: Option<&std::path::Path>) -> String {
    render_for(
        doc,
        width,
        std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty()),
        file,
    )
}

/// The same, with the `NO_COLOR` decision made by the caller — tests inject
/// rather than touch the process environment.
///
/// Lines break where [`crate::plain::render`]'s do: no word is divided to
/// fill a line, and the author's soft hyphens are not break points. Styled,
/// one thing differs — a run wider than its whole row is divided the way
/// the screen divides it, by the document's own language rules or at a
/// soft hyphen with a hyphen, otherwise cut and ended with `↩`, and that
/// mark is written FAINT ([`Division::Overwide`]). Faint is weight, not
/// colour, so it is the screen's dim mark in a form this contract allows;
/// and like the screen's, it is not in the text. Unstyled, there is no way
/// to say a character was added, so nothing is: the run stands whole on a
/// line of its own, exactly as in plain output ([`Division::Never`]).
#[must_use]
pub fn render_with(doc: &Document, width: u16, styled: bool) -> String {
    render_for(doc, width, styled, None)
}

/// The same, for the document at `file`: a `[[wikilink]]` whose note is
/// there beside it becomes a hyperlink to that file. With no file — a pipe
/// — no wikilink is a hyperlink, and neither is one whose note is missing:
/// a note's NAME is not an address, and a terminal handed one can do
/// nothing with it.
#[must_use]
pub fn render_for(
    doc: &Document,
    width: u16,
    styled: bool,
    file: Option<&std::path::Path>,
) -> String {
    let width = width.max(20);
    // Aligned tables, like plain output: cards exist only on a screen.
    let layout = if styled {
        Layout::linear(doc, width, hyphenation_for(doc), Division::Overwide)
    } else {
        Layout::linear(doc, width, Hyphenation::Off, Division::Never)
    };
    let targets = if styled {
        link_targets(doc, file)
    } else {
        Vec::new()
    };
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
                RowKind::Text { hyphen, split, .. } => {
                    // Separator, quote bars, indent and prefix are [`plain`]'s
                    // to decide — see `crate::plain::open_row` for why the
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
                        &targets,
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
                    // The mark of a divided run, in the cell the layout kept
                    // for it — after the link closes, because it is not part
                    // of the link's text. Only the styled layout, `Overwide`, marks
                    // a row at all; the test of `styled` says so where it matters.
                    if styled && let Some(mark) = cut_mark(hyphen, split) {
                        line.push_str(&mark);
                    }
                    // Trimmed like plain output's lines, which is half of what
                    // `NO_COLOR` byte-identity needs; a styled line ends in a
                    // reset or a link terminator, so the trim finds nothing.
                    first_text = false;
                    out.push_str(line.trim_end());
                    out.push('\n');
                }
            }
        }
    }
    out
}

/// The faint mark a row divided inside a run ends with: the hyphen of a
/// word divided by its language or at its author's soft hyphen, or `↩` for
/// anything else cut — the screen's two marks (`render::paint_cut_mark`),
/// dim there and faint here.
fn cut_mark(hyphen: bool, split: bool) -> Option<String> {
    let mark = if hyphen {
        "-"
    } else if split {
        "↩"
    } else {
        return None;
    };
    let faint = Attrs {
        dim: true,
        ..Attrs::default()
    };
    Some(format!("{}{mark}{RESET}", faint.sgr()))
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
    links: &'a [Option<String>],
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
        let url = inl.link.and_then(|id| links.get(id.0 as usize)?.as_deref());
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

    /// Whatever a destination holds, the sequence carries printable ASCII
    /// and nothing else — and it is `copy link`'s cleaning that decides what
    /// is dropped, so the two cannot come to disagree about an address.
    #[test]
    fn a_hyperlink_destination_is_printable_ascii_whatever_the_document_wrote() {
        let every_odd: String = (0u32..=0x9f)
            .chain([0xAD, 0x61C, 0x200B, 0x200E, 0x200F, 0x2028, 0x2060, 0xFEFF])
            .chain(0x202A..=0x202E)
            .chain(0x2066..=0x2069)
            .chain([0xE0001, 0xE0041, 0x65E5])
            .filter_map(char::from_u32)
            .collect();
        for url in [
            every_odd.as_str(),
            "https://例え.jp/ü?q=a b#frag",
            "http://ex.com/\u{202E}gro.elgoog//:sptth",
            "x\u{1b}]8;id=evil;http://evil\u{7}",
        ] {
            let got = osc8_target(url);
            assert!(
                got.bytes().all(|b| b.is_ascii_graphic()),
                "{url:?}: {got:?}"
            );
            // Decoded, it is the address the clipboard would get.
            let mut bytes = Vec::new();
            let mut rest = got.as_bytes();
            while let Some((&b, tail)) = rest.split_first() {
                // A literal `%` in the address is followed by whatever the
                // author wrote; only this function's own escapes are of
                // non-graphic bytes, so decode exactly those.
                let hex = tail.get(..2).and_then(|h| std::str::from_utf8(h).ok());
                match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(v) if b == b'%' && !v.is_ascii_graphic() => {
                        bytes.push(v);
                        rest = &tail[2..];
                    }
                    _ => {
                        bytes.push(b);
                        rest = tail;
                    }
                }
            }
            assert_eq!(
                String::from_utf8(bytes).unwrap(),
                carrel_state::clip::line(url),
                "{url:?}"
            );
        }
        assert_eq!(
            osc8_target("https://例え.jp/ü?q=a b;c:d#f"),
            "https://%E4%BE%8B%E3%81%88.jp/%C3%BC?q=a%20b;c:d#f"
        );
    }

    /// A file name is a name: `#`, `?`, `%` and a space are part of it.
    #[test]
    fn a_file_uri_encodes_what_an_address_would_read_as_syntax() {
        use std::path::Path;
        assert_eq!(
            osc8_file_uri(Path::new("/w/my notes/50% off #1?.md")),
            "file:///w/my%20notes/50%25%20off%20%231%3F.md"
        );
        assert_eq!(
            osc8_file_uri(Path::new("/w/日\u{202E}\u{1b}.md")),
            "file:///w/%E6%97%A5%E2%80%AE%1B.md"
        );
    }

    /// In `file://plain.md` the name is a HOST. A path given relative —
    /// which is how a document opened as `carrel notes.md` hands over its
    /// wikilinks — must go out as the absolute path it names.
    #[test]
    fn a_file_uri_is_absolute_whatever_the_path_was() {
        use std::path::Path;
        let here = std::env::current_dir().unwrap();
        let here = here.to_str().unwrap();
        assert!(here.starts_with('/') && !here.contains(' '), "{here}");
        assert_eq!(
            osc8_file_uri(Path::new("plain.md")),
            format!("file://{here}/plain.md")
        );
        assert_eq!(
            osc8_file_uri(Path::new("sub/what? now.md")),
            format!("file://{here}/sub/what%3F%20now.md")
        );
        // Not canonicalised: the path is the one the reader would open.
        assert_eq!(
            osc8_file_uri(Path::new("../x.md")),
            format!("file://{here}/../x.md")
        );
        assert_eq!(
            osc8_file_uri(Path::new("/abs/a b.md")),
            "file:///abs/a%20b.md"
        );
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt as _;
            let raw = std::ffi::OsStr::from_bytes(b"caf\xe9.md");
            assert_eq!(
                osc8_file_uri(Path::new(raw)),
                format!("file://{here}/caf%E9.md")
            );
        }
    }

    /// `--render` wraps a wikilink in a hyperlink only when the note is
    /// there, and then as the file it is. It wrapped every one in the bare
    /// note name (`ESC ] 8 ; ; Known`), which is not an address.
    #[test]
    fn a_rendered_wikilink_is_a_file_uri_when_its_note_exists_and_no_link_when_not() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Known note.md"), "x\n").unwrap();
        let file = dir.path().join("index.md");
        let doc = Document::parse(
            "see [[Known note]], [[Unknown]], [[#Here]] and [w](https://e.com/x)\n",
        );
        let targets = |out: &str| -> Vec<String> {
            out.split("\u{1b}]8;;")
                .skip(1)
                .filter_map(|rest| rest.split("\u{1b}\\").next())
                .filter(|t| !t.is_empty())
                .map(str::to_string)
                .collect()
        };
        let out = render_for(&doc, 80, true, Some(&file));
        let known = osc8_file_uri(&dir.path().join("Known note.md"));
        assert!(known.starts_with("file:///") && known.ends_with("/Known%20note.md"));
        assert_eq!(targets(&out), [known, "https://e.com/x".to_string()]);
        // The words are all still there.
        let shown = render_for(&doc, 80, false, Some(&file));
        assert!(
            shown.contains("Known note") && shown.contains("Unknown"),
            "{shown}"
        );

        // With no file to stand beside — a pipe — no wikilink is an address.
        assert_eq!(targets(&render_with(&doc, 80, true)), ["https://e.com/x"]);
    }

    /// `--render` follows the screen's rule for every link, not only for
    /// wikilinks: a file beside the document is its absolute address, and a
    /// link a terminal could do nothing with gets no hyperlink.
    #[test]
    fn a_rendered_local_link_is_a_file_uri_and_a_dead_or_in_page_one_is_none() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("d2.md"), "x\n").unwrap();
        std::fs::write(dir.path().join("sub/x y.md"), "x\n").unwrap();
        let file = dir.path().join("doc.md");
        let doc = Document::parse(
            "# Here\n\n[a](d2.md#sec) [c](sub/x%20y.md) [gone](gone.md) [frag](#here) \
             [up](../outside.md) [web](https://e.com/p#f)\n",
        );
        let targets = |out: &str| -> Vec<String> {
            out.split("\u{1b}]8;;")
                .skip(1)
                .filter_map(|rest| rest.split("\u{1b}\\").next())
                .filter(|t| !t.is_empty())
                .map(str::to_string)
                .collect()
        };
        let root = osc8_file_uri(&dir.path().canonicalize().unwrap());
        assert_eq!(
            targets(&render_for(&doc, 100, true, Some(&file))),
            [
                format!("{root}/d2.md#sec"),
                format!("{root}/sub/x%20y.md"),
                "https://e.com/p#f".to_string()
            ]
        );
        // From a pipe there is no folder to look in.
        assert_eq!(
            targets(&render_with(&doc, 100, true)),
            ["https://e.com/p#f"]
        );
        // Every word is still printed.
        let shown = render_for(&doc, 100, false, Some(&file));
        assert!(shown.contains("gone") && shown.contains("frag"), "{shown}");
    }

    #[test]
    fn an_entity_encoded_escape_in_a_link_destination_is_stripped() {
        // CommonMark entity-decodes link destinations, so these reach
        // `doc.links` holding a raw ESC or BEL — an author-controlled escape
        // sequence in the middle of our OSC 8 opener.
        for src in [
            "[c](http://e.com/&#27;]8;;http://evil)\n",
            "[b](http://e.com/&#7;bel)\n",
            "[r]\n\n[r]: <http://e.com/&#27;x>\n",
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
        // With no scheme such a destination is a file name, and a file
        // name that is not there is no hyperlink at all.
        for src in ["[c](&#27;]8;;http://evil)\n", "[b](&#7;bel)\n"] {
            let out = render_with(&Document::parse(src), 80, true);
            assert!(!out.contains("\u{1b}]8;"), "{src:?}: {out:?}");
        }
    }
}
