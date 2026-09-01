//! Line breaking and column measurement.
//!
//! See architecture.md (private notes repo) §3.4 and §5, and the design at
//! the reflow-layer design doc (notes repo).
//!
//! # Shape
//!
//! Two stages with a seam between them:
//!
//! - [`units`] turns text into measured break units. **All Unicode is here.**
//! - [`pack`] turns units into rows. **It never sees a string**, so its
//!   invariants are testable against units built by hand.
//!
//! Keeping the base unit producer width-free is deliberate: the only
//! width-dependent step is `split_to_fit`, layered on top, which leaves room to
//! cache break units across widths later without touching the fitting logic.
//!
//! # Scope
//!
//! Every block wraps through [`wrap`], which is why there is no per-kind
//! layout to keep in step. What differs is the fit, not the algorithm:
//!
//! - **Code blocks** carry `Slice::code`, which reserves the continuation
//!   marker and hangs wrapped rows under the line's own indent. A no-wrap
//!   policy with horizontal scroll would need viewport state this crate does
//!   not hold, and remains unbuilt.
//! - **Tables** are aligned at PARSE — column widths are max-content display
//!   widths, so cells are padded in space 2 and each visual row is already one
//!   contiguous range. Nothing table-shaped happens here. [`wrap_range`] lays
//!   out a single cell for the frontend's card view (Q15, answered and
//!   shipped 2026-08-11).
//! - **Images, mermaid art and math** wrap their alt/source text like anything
//!   else; their real heights are a frontend override, because they depend on
//!   cell size and decoded dimensions this crate never sees.
//!
//! # The width rule
//!
//! Segment into grapheme clusters, then measure **each cluster string**. Never
//! sum per-`char` widths: `unicode-width` 0.2.x returns 2 for the ZWJ family
//! emoji as a cluster and 6 if you add up its codepoints.

#[cfg(test)]
mod proptests;

mod pack;
mod units;

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::document::Document;
use crate::position::BlockIdx;

/// Bytes after which a block is split into another layout chunk.
///
/// In markdown a paragraph is one logical line and can be hundreds of KB.
/// Wrapping is greedy, so painting a row near the end of such a block would
/// otherwise rescan the whole thing every time. Chunking bounds that — but only
/// because [`wrap_chunk`] lets a caller lay out one chunk; chunking internally
/// and then always wrapping every chunk would bound nothing.
pub const CHUNK_BYTES: u32 = 64 * 1024;

/// Measures a grapheme cluster in terminal cells.
///
/// Injected so this crate compiles and unit-tests with no terminal, and so a
/// future DEC mode 2027 negotiation can swap the implementation. That is the
/// only reason the indirection exists.
///
/// # Precondition
///
/// **Must return 1 for every printable ASCII cluster.** The reflow layer skips
/// the grapheme walk entirely for printable-ASCII runs, which is most of the
/// throughput on real documents. This costs nothing in practice: DEC mode 2027
/// changes how *clusters* are counted, not how `a` is, and no terminal renders
/// printable ASCII in anything but one cell.
pub type WidthFn = dyn Fn(&str) -> u16 + Send + Sync;

/// The default measurement: UAX #29 clusters, UAX #11 widths, minimum one cell.
///
/// `.max(1)` matches Helix's `grapheme_width` and keeps an ill-formed cluster
/// occupying a cell rather than vanishing.
#[must_use]
pub fn cluster_width(g: &str) -> u16 {
    u16::try_from(UnicodeWidthStr::width(g)).unwrap_or(1).max(1)
}

/// Display width of a string, measured cluster by cluster.
#[must_use]
pub fn display_width(s: &str) -> u16 {
    s.graphemes(true)
        .fold(0u16, |acc, g| acc.saturating_add(cluster_width(g)))
}

/// Cells the core reserves on a code block's continuation rows for a
/// frontend-drawn marker.
///
/// A **semantic reservation**, like `indent` — not a glyph. The terminal draws
/// `↳ `; a GTK frontend may draw a hanging-indent rule in the same space.
/// Without the reservation the marker would overwrite the first two characters
/// of the continued text.
pub const CONTINUATION_COLS: u16 = 2;

/// What a row is made of.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RowKind {
    Text {
        first_in_block: bool,
        /// This row continues a logical line that did not fit on one row.
        /// The core states the fact; the frontend decides whether to mark it.
        continued: bool,
    },
    /// A rule, a table border, an image frame — has no content bytes.
    /// Nothing constructs this yet; see the scope note in the module docs.
    Decoration,
}

/// One visual row. **Derived from `(document, width)`; never persisted.**
#[derive(Clone, Debug)]
pub struct Row {
    pub block: BlockIdx,
    /// The slice of `Document::text` painted on this row.
    ///
    /// Within a block, rows are ordered and non-overlapping **with gaps** exactly
    /// where wrapping elided whitespace:
    /// `rows[i].doc.end <= rows[i+1].doc.start`. The gap is correct, not a
    /// defect — see architecture.md (private notes repo) §3.3.
    pub doc: Range<u32>,
    /// Leading display cells before content.
    ///
    /// This is the whole inset, including the width reserved for a first-row
    /// [`Prefix`](crate::document::Prefix). There is no separate continuation
    /// indent: painting the prefix *into* the reserved region gives hanging
    /// indent for free.
    pub indent: u16,
    pub kind: RowKind,
}

/// Byte offsets at which each layout chunk of `text` starts.
///
/// A boundary is the first break opportunity at or after each [`CHUNK_BYTES`]
/// multiple, so boundaries are a pure function of the text and are therefore
/// identical in the height pass and the row pass.
fn chunk_starts(text: &str) -> Vec<u32> {
    let mut starts = vec![0u32];
    if text.len() as u64 <= u64::from(CHUNK_BYTES) {
        return starts;
    }
    let mut target = CHUNK_BYTES;
    for (pos, _) in unicode_linebreak::linebreaks(text) {
        let pos = pos as u32;
        if pos >= target && (pos as usize) < text.len() {
            starts.push(pos);
            target = pos.saturating_add(CHUNK_BYTES);
        }
    }
    starts
}

/// The byte range of one chunk of `text`.
fn chunk_range(text: &str, chunk: u32) -> Range<usize> {
    let starts = chunk_starts(text);
    let i = chunk as usize;
    let start = starts.get(i).copied().unwrap_or(text.len() as u32) as usize;
    let end = starts
        .get(i + 1)
        .copied()
        .map_or(text.len(), |e| e as usize);
    start..end
}

/// How many layout chunks a block has. Always at least 1.
///
/// **Width-independent** — chunk boundaries are a function of the text alone,
/// which is why this is allowed in an API that forbids width-dependent layout
/// quantities. It is the same category as `Node::indent`.
#[must_use]
pub fn chunk_count(doc: &Document, block: BlockIdx) -> u32 {
    chunk_starts(doc.block_text(block)).len() as u32
}

/// Lay out one chunk of a block at `width`. Returns the row count.
///
/// Lets a caller bound work on a pathologically large paragraph. `first` says
/// whether the next row emitted is the block's first, and is updated — a chunk
/// boundary is not a block boundary.
pub fn wrap_chunk<F: FnMut(Row)>(
    doc: &Document,
    block: BlockIdx,
    chunk: u32,
    width: u16,
    w: &WidthFn,
    first: &mut bool,
    mut sink: F,
) -> u32 {
    let node = doc.node_for_block(block);
    let text = doc.block_text(block);
    let r = chunk_range(text, chunk);
    let at = Slice {
        doc_base: node.doc.start + r.start as u32,
        block,
        width,
        indent: node.indent,
        code: matches!(node.kind, crate::document::NodeKind::CodeBlock { .. }),
    };
    wrap_slice(&text[r], &at, w, first, &mut sink)
}

/// Lay out one doc subrange at `width`, inset by `indent`. Returns the row count.
///
/// The card view's workhorse: a table cell is a subrange of its block, laid
/// out under the label gutter. `range` must lie within `block` and contain no
/// mandatory break. `first` behaves as in [`wrap_chunk`].
#[allow(clippy::too_many_arguments)]
pub fn wrap_range<F: FnMut(Row)>(
    doc: &Document,
    block: BlockIdx,
    range: Range<u32>,
    width: u16,
    indent: u16,
    w: &WidthFn,
    first: &mut bool,
    mut sink: F,
) -> u32 {
    let text = &doc.text[range.start as usize..range.end as usize];

    // Empty or whitespace-only ranges emit nothing.
    if text.trim().is_empty() {
        return 0;
    }

    let at = Slice {
        doc_base: range.start,
        block,
        width,
        indent,
        code: false,
    };
    wrap_slice(text, &at, w, first, &mut sink)
}

/// Lay out one block at `width`. Returns the row count.
///
/// `sink` is called once per produced row — pass a counting closure for the
/// height-only pass and a `Vec::push` for the row pass, so the two can never
/// disagree.
pub fn wrap<F: FnMut(Row)>(
    doc: &Document,
    block: BlockIdx,
    width: u16,
    w: &WidthFn,
    mut sink: F,
) -> u32 {
    let node = doc.node_for_block(block);
    let text = doc.block_text(block);
    let mut first = true;
    let mut rows = 0u32;
    // `chunk_starts` runs a full UAX #14 pass over the block, so it is
    // computed ONCE and windowed over. Calling `chunk_range` per chunk
    // recomputed it every time, which made the chunking that exists to BOUND
    // work on a huge paragraph itself quadratic: a block of k chunks paid
    // k+1 full scans. Measured on one paragraph, `--plain`: 1 MB 93 ms,
    // 2 MB 337 ms, 4 MB 1240 ms, 8 MB 6361 ms — and this is the resize path,
    // walked twice per relayout (the height pass and the row pass).
    let starts = chunk_starts(text);
    for (i, start) in starts.iter().enumerate() {
        let end = starts.get(i + 1).map_or(text.len(), |&e| e as usize);
        let at = Slice {
            doc_base: node.doc.start + start,
            block,
            width,
            indent: node.indent,
            code: matches!(node.kind, crate::document::NodeKind::CodeBlock { .. }),
        };
        rows = rows.saturating_add(wrap_slice(
            &text[*start as usize..end],
            &at,
            w,
            &mut first,
            &mut sink,
        ));
    }
    rows
}

/// Where a slice of text sits, and how wide it may be. Everything `wrap_slice`
/// needs that is not the text, the width function, or the sink.
struct Slice {
    doc_base: u32,
    block: BlockIdx,
    width: u16,
    indent: u16,
    /// Code blocks reserve [`CONTINUATION_COLS`] on continuation rows for a
    /// marker; prose reserves nothing.
    code: bool,
}

/// Widths and indents for one logical line.
///
/// `avail` is not constant across a block: a code line indented four spaces has
/// continuation rows four cells narrower, plus the marker reservation.
pub(super) struct LineFit {
    /// Column at which the first row's content starts.
    pub(super) first_indent: u16,
    /// Column at which continuation content starts. Includes the reservation.
    pub(super) cont_indent: u16,
    /// Cells available on the line's first row.
    pub(super) first_avail: u16,
    /// Cells available on continuation rows.
    pub(super) cont_avail: u16,
}

impl LineFit {
    /// `lead` is the display width of the line's leading whitespace.
    fn new(width: u16, block_indent: u16, lead: u16, reserve: u16) -> Self {
        let first_indent = block_indent;
        let first_avail = width.saturating_sub(first_indent).max(1);

        // Degenerate cases, in order: drop the reservation, then drop the
        // hanging indent. A 20-space-indented line in a 24-column terminal must
        // still show something. This is a clamp, not an error.
        let mut cont_indent = block_indent.saturating_add(lead).saturating_add(reserve);
        if width.saturating_sub(cont_indent) == 0 {
            cont_indent = block_indent.saturating_add(lead);
        }
        if width.saturating_sub(cont_indent) == 0 {
            cont_indent = block_indent;
        }
        let cont_avail = width.saturating_sub(cont_indent).max(1);

        Self {
            first_indent,
            cont_indent,
            first_avail,
            cont_avail,
        }
    }
}

/// Every character UAX #14 classes as a mandatory break.
///
/// `wrap_slice` must split logical lines on ALL of these, not just '\n' —
/// `unicode-linebreak` reports each as `Mandatory`, and a mandatory break
/// arriving mid-call used to leave the packer's continuation state stale:
/// the next line started with the previous line's continuation indent and a
/// bogus `continued` flag.
const MANDATORY_BREAKS: [char; 6] = [
    '\n', '\u{000B}', '\u{000C}', '\u{0085}', '\u{2028}', '\u{2029}',
];

/// Display width of `line`'s leading whitespace.
///
/// Chars, not clusters, is safe here: whitespace never combines into a wider
/// cluster, and tabs were expanded at parse.
fn leading_width(line: &str, w: &WidthFn) -> u16 {
    line.chars()
        .take_while(|c| c.is_whitespace() && !MANDATORY_BREAKS.contains(c))
        .fold(0u16, |acc, c| {
            let mut b = [0u8; 4];
            acc.saturating_add(w(c.encode_utf8(&mut b)))
        })
}

/// Wrap a raw slice. The whole pipeline, minus document lookup and chunking.
///
/// Iterates **logical lines**, because leading whitespace — and therefore the
/// continuation indent — differs per line. A chunk boundary can fall mid-line,
/// so the first line of a later chunk is treated as a new logical line and
/// loses its continuation indent; that affects blocks over [`CHUNK_BYTES`]
/// only, by one row.
fn wrap_slice<F: FnMut(Row)>(
    text: &str,
    at: &Slice,
    w: &WidthFn,
    first: &mut bool,
    sink: &mut F,
) -> u32 {
    let reserve = if at.code { CONTINUATION_COLS } else { 0 };
    let mut rows = 0u32;
    let mut offset = 0usize;

    // `split_inclusive` keeps the terminator, so offsets stay exact and the
    // final line without one is still yielded. The pattern is the full UAX #14
    // mandatory set, not just '\n' — see `MANDATORY_BREAKS`.
    for line in text.split_inclusive(MANDATORY_BREAKS) {
        let lead = leading_width(line, w);
        let fit = LineFit::new(at.width, at.indent, lead, reserve);
        // Split against the NARROWER budget so a unit can never overflow the
        // row it lands on. Only tokens wider than `cont_avail` are affected,
        // and those break a few cells earlier on the first row than strictly
        // necessary — deterministic, and pathological tokens only.
        let fitted = units::fitted(units::units(line, w), line, fit.cont_avail, w);
        rows = rows.saturating_add(pack::pack(
            fitted,
            at.doc_base + offset as u32,
            at.block,
            &fit,
            first,
            sink,
        ));
        offset += line.len();
    }

    // An empty slice yields no lines at all, but a block is at least one row.
    if rows == 0 {
        let fit = LineFit::new(at.width, at.indent, 0, reserve);
        rows = pack::pack(std::iter::empty(), at.doc_base, at.block, &fit, first, sink);
    }
    rows
}

/// Wrap a raw slice as a standalone block. Test entry point: lets the packer be
/// exercised without constructing a `Document`.
#[cfg(test)]
pub(crate) fn wrap_text<F: FnMut(Row)>(
    text: &str,
    doc_base: u32,
    block: BlockIdx,
    width: u16,
    indent: u16,
    w: &WidthFn,
    mut sink: F,
) -> u32 {
    let mut first = true;
    let mut rows = 0u32;
    let starts = chunk_starts(text);
    for (i, start) in starts.iter().enumerate() {
        let end = starts.get(i + 1).map_or(text.len(), |&e| e as usize);
        let at = Slice {
            doc_base: doc_base + start,
            block,
            width,
            indent,
            code: false,
        };
        rows = rows.saturating_add(wrap_slice(
            &text[*start as usize..end],
            &at,
            w,
            &mut first,
            &mut sink,
        ));
    }
    rows
}

/// Convert a doc-byte sub-range of a row into a `[start_col, end_col)` cell range.
///
/// Boundaries are snapped **outward** to grapheme cluster boundaries. This is not
/// cosmetic: ratatui writes a cluster's symbol into its first cell and `reset()`s
/// the trailing cells, so styling only the trailing half of a wide cluster paints
/// a visible gap rather than half a character. A regex match lands on a `char`
/// boundary, which is not necessarily a cluster boundary.
///
/// `r` is expected to intersect the row; a range that does not returns an
/// **empty** `c0 == c1`, which every caller already treats as "paint nothing".
/// That is a value rather than an `Option` on purpose — the miss is not an
/// error, and the callers that would have to unwrap one are three
/// paint loops that each guard `c1 > c0` anyway. It has to be *said*, though,
/// because the walk's own answer for a range past the end of the row was the
/// WHOLE row: `lo` outran every offset so the start stayed at the indent, and
/// `hi` outran every offset so the end reached the last cluster.
#[must_use]
pub fn cols_for_doc_range(
    row_text: &str,
    row_doc_start: u32,
    indent: u16,
    r: &Range<u32>,
) -> (u16, u16) {
    let row_end = row_doc_start.saturating_add(u32::try_from(row_text.len()).unwrap_or(u32::MAX));
    if r.end <= row_doc_start || r.start >= row_end {
        return (indent, indent);
    }
    let lo = r.start.saturating_sub(row_doc_start) as usize;
    let hi = r.end.saturating_sub(row_doc_start) as usize;

    let mut col = indent;
    let (mut c0, mut c1) = (indent, indent);
    let mut seen_start = false;

    for (off, g) in row_text.grapheme_indices(true) {
        let w = cluster_width(g);
        if !seen_start && off + g.len() > lo {
            c0 = col;
            seen_start = true;
        }
        if off < hi {
            c1 = col + w;
        } else {
            break;
        }
        col += w;
    }
    (c0, c1.max(c0))
}

/// The doc-byte `(start, end)` of the grapheme cluster rendered at display
/// column `col` of a row — the inverse of [`cols_for_doc_range`], for pointer
/// hits. A column before `indent` resolves to the first cluster; one past the
/// row's last cluster resolves to the zero-width `(end, end)`.
///
/// This is one of the bounded, transient col↔byte walks `position.rs`
/// licenses: nothing produced here is stored beyond the selection byte range
/// it helps construct.
#[must_use]
pub fn cluster_at_col(row_text: &str, row_doc_start: u32, indent: u16, col: u16) -> (u32, u32) {
    let mut at = indent;
    for (off, g) in row_text.grapheme_indices(true) {
        let w = cluster_width(g);
        let start = row_doc_start + u32::try_from(off).unwrap_or(u32::MAX);
        let end = start + u32::try_from(g.len()).unwrap_or(u32::MAX);
        if col < at + w {
            return (start, end);
        }
        at += w;
    }
    let end = row_doc_start + u32::try_from(row_text.len()).unwrap_or(u32::MAX);
    (end, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows_of(text: &str, width: u16) -> Vec<String> {
        let mut out = Vec::new();
        wrap_text(text, 0, BlockIdx(0), width, 0, &cluster_width, |r| {
            out.push(text[r.doc.start as usize..r.doc.end as usize].to_string());
        });
        out
    }

    #[test]
    fn zwj_emoji_is_measured_as_a_cluster_not_a_sum_of_codepoints() {
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
        assert_eq!(display_width(family), 2, "cluster width");
        let naive: u16 = family.chars().map(|c| cluster_width(&c.to_string())).sum();
        assert!(naive > 2, "the naive per-char sum should be wrong: {naive}");
    }

    #[test]
    fn cjk_is_two_cells() {
        assert_eq!(display_width("日本語"), 6);
    }

    /// Wrap raw text as if it were a block of the given kind.
    fn wrap_text_kind<F: FnMut(Row)>(text: &str, width: u16, code: bool, mut sink: F) {
        let at = Slice {
            doc_base: 0,
            block: BlockIdx(0),
            width,
            indent: 0,
            code,
        };
        let mut first = true;
        wrap_slice(text, &at, &cluster_width, &mut first, &mut sink);
    }

    #[test]
    fn a_code_line_continues_under_its_own_indentation() {
        // Four spaces of leading whitespace, plus 2 reserved for the marker,
        // means continuation content starts at column 6.
        let text = "    let result = compute(alpha, beta);";
        let mut rows = Vec::new();
        wrap_text_kind(text, 30, true, |r| rows.push((r.indent, r.kind)));
        assert!(rows.len() > 1, "must wrap: {rows:?}");
        assert_eq!(rows[0].0, 0, "the first row keeps the block indent");
        assert!(
            rows[1..].iter().all(|(i, _)| *i == 4 + CONTINUATION_COLS),
            "continuations hang under the code's own indent: {rows:?}",
        );
        assert!(
            rows[1..].iter().all(|(_, k)| matches!(
                k,
                RowKind::Text {
                    continued: true,
                    ..
                }
            )),
            "{rows:?}",
        );
    }

    #[test]
    fn prose_continuation_gets_no_reservation() {
        let mut rows = Vec::new();
        wrap_text_kind("alpha beta gamma delta epsilon", 12, false, |r| {
            rows.push((r.indent, r.kind));
        });
        assert!(rows.len() > 1);
        assert!(
            rows.iter().all(|(i, _)| *i == 0),
            "a paragraph has no interior indentation to hang under: {rows:?}",
        );
    }

    #[test]
    fn an_indent_wider_than_the_viewport_still_shows_content() {
        let text = "                    deeply indented and long enough to wrap";
        let mut rows = Vec::new();
        wrap_text_kind(text, 24, true, |r| rows.push((r.indent, r.kind)));
        assert!(!rows.is_empty());
        assert!(
            rows.iter().all(|(i, _)| *i < 24),
            "content must always get at least one cell: {rows:?}",
        );
    }

    #[test]
    fn a_line_separator_starts_a_fresh_logical_line_with_fresh_state() {
        // U+2028 is a UAX #14 mandatory break that is not '\n'. Text after it
        // must start a new logical line: block indent, not the continuation
        // indent, and continued = false.
        let text = "    alpha beta gamma delta\u{2028}omega";
        let mut rows = Vec::new();
        wrap_text_kind(text, 16, true, |r| rows.push((r.indent, r.kind)));
        let last = rows.last().unwrap();
        assert_eq!(
            last.0, 0,
            "post-separator text keeps the block indent: {rows:?}"
        );
        assert!(
            matches!(
                last.1,
                RowKind::Text {
                    continued: false,
                    ..
                }
            ),
            "post-separator text is not a continuation: {rows:?}",
        );
    }

    #[test]
    fn a_row_that_continues_a_long_line_is_marked_as_such() {
        let mut kinds = Vec::new();
        wrap_text(
            "alpha beta gamma delta",
            0,
            BlockIdx(0),
            11,
            0,
            &cluster_width,
            |r| kinds.push(r.kind),
        );
        assert!(kinds.len() > 1, "must wrap: {kinds:?}");
        assert_eq!(
            kinds[0],
            RowKind::Text {
                first_in_block: true,
                continued: false
            },
        );
        assert!(
            kinds[1..].iter().all(|k| matches!(
                k,
                RowKind::Text {
                    continued: true,
                    ..
                }
            )),
            "every row after the first continues the same logical line: {kinds:?}",
        );
    }

    #[test]
    fn a_hard_newline_starts_a_new_logical_line_not_a_continuation() {
        let mut kinds = Vec::new();
        wrap_text("a\nb\nc", 0, BlockIdx(0), 80, 0, &cluster_width, |r| {
            kinds.push(r.kind);
        });
        assert_eq!(kinds.len(), 3);
        assert!(
            kinds.iter().all(|k| matches!(
                k,
                RowKind::Text {
                    continued: false,
                    ..
                }
            )),
            "none of these continue anything: {kinds:?}",
        );
    }

    #[test]
    fn wraps_at_word_boundaries() {
        assert_eq!(
            rows_of("the quick brown fox", 10),
            ["the quick", "brown fox"]
        );
    }

    #[test]
    fn rows_are_ordered_and_non_overlapping_with_gaps_at_elided_spaces() {
        let text = "alpha beta gamma delta";
        let mut rows = Vec::new();
        wrap_text(text, 0, BlockIdx(0), 12, 0, &cluster_width, |r| {
            rows.push(r);
        });
        for w in rows.windows(2) {
            assert!(
                w[0].doc.end <= w[1].doc.start,
                "overlap: {:?} then {:?}",
                w[0].doc,
                w[1].doc
            );
        }
    }

    #[test]
    fn hard_newlines_start_new_rows() {
        assert_eq!(rows_of("a\nb\nc", 80), ["a", "b", "c"]);
    }

    #[test]
    fn a_word_longer_than_the_viewport_is_broken_to_fit() {
        // The old test asserted only that SOME row came out, which passes while
        // the row overflows the viewport. Assert the actual requirement.
        let rows = rows_of("supercalifragilistic", 5);
        assert!(
            rows.iter().all(|r| display_width(r) <= 5),
            "row wider than the viewport: {rows:?}"
        );
        assert_eq!(rows.concat(), "supercalifragilistic", "no bytes lost");
    }

    #[test]
    fn trailing_whitespace_may_overhang_rather_than_forcing_an_early_wrap() {
        // "abc def" is exactly 7 cells and fits at width 7. The space after
        // "def" is elided at the row end, so counting it toward the fit test
        // wraps a column early.
        assert_eq!(rows_of("abc def ", 7), ["abc def"]);
    }

    #[test]
    fn a_wide_cluster_never_splits_and_overhangs_at_width_one() {
        let rows = rows_of("日本語", 1);
        assert_eq!(rows, ["日", "本", "語"]);
    }

    #[test]
    fn an_empty_block_is_one_empty_row() {
        assert_eq!(rows_of("", 80), [""]);
    }

    #[test]
    fn the_height_pass_and_the_row_pass_agree() {
        let text = "the quick brown fox jumps over the lazy dog 日本語 supercalifragilistic";
        for width in 1..=40u16 {
            let mut counted = 0u32;
            let n = wrap_text(text, 0, BlockIdx(0), width, 0, &cluster_width, |_| {
                counted += 1;
            });
            assert_eq!(n, counted, "at width {width}");
        }
    }

    #[test]
    fn cols_snap_outward_to_cluster_boundaries() {
        // "日本語" — a range landing inside the first cluster must still start
        // the highlight at column 0 and cover the whole 2-cell character.
        let (c0, c1) = cols_for_doc_range("日本語", 0, 0, &(1..4));
        assert_eq!(c0, 0, "start snapped down to the cluster containing byte 1");
        assert_eq!(c1, 4, "end snapped up past the cluster containing byte 3");
    }

    #[test]
    fn cols_respect_indent() {
        let (c0, c1) = cols_for_doc_range("hello", 0, 4, &(0..2));
        assert_eq!((c0, c1), (4, 6));
    }

    /// A range that misses the row entirely must paint nothing. The
    /// after-the-row case used to report the WHOLE row: `lo` outran every
    /// offset so the start stayed at the indent, and `hi` outran every offset
    /// so the end walked to the last cluster.
    #[test]
    fn cols_for_a_range_that_misses_the_row_are_empty() {
        let (c0, c1) = cols_for_doc_range("hello", 0, 0, &(10..12));
        assert_eq!(c0, c1, "range after the row: {c0}..{c1}");
        let (c0, c1) = cols_for_doc_range("hello", 10, 0, &(2..4));
        assert_eq!(c0, c1, "range before the row: {c0}..{c1}");
        // Touching a boundary is still a miss: a range is half-open, so
        // `..10` ends where the row begins and `15..` starts where it ends.
        let (c0, c1) = cols_for_doc_range("hello", 10, 3, &(5..10));
        assert_eq!(c0, c1, "ends exactly at the row start: {c0}..{c1}");
        let (c0, c1) = cols_for_doc_range("hello", 10, 3, &(15..17));
        assert_eq!(c0, c1, "starts exactly at the row end: {c0}..{c1}");
    }

    #[test]
    fn cluster_at_col_maps_ascii_columns_to_bytes() {
        // "hello" at indent 4: column 6 is 'l', bytes 102..103 → 2..3 + base.
        assert_eq!(cluster_at_col("hello", 100, 4, 6), (102, 103));
        assert_eq!(cluster_at_col("hello", 100, 4, 4), (100, 101), "first col");
    }

    #[test]
    fn cluster_at_col_gives_the_same_cluster_for_both_cells_of_a_wide_char() {
        // "日本語": each cluster is 3 bytes, 2 columns.
        assert_eq!(cluster_at_col("日本語", 0, 0, 0), (0, 3));
        assert_eq!(cluster_at_col("日本語", 0, 0, 1), (0, 3), "second cell");
        assert_eq!(cluster_at_col("日本語", 0, 0, 2), (3, 6));
    }

    #[test]
    fn cluster_at_col_clamps_before_indent_and_past_the_end() {
        assert_eq!(cluster_at_col("ab", 10, 4, 0), (10, 11), "before indent");
        let end = 10 + 2;
        assert_eq!(cluster_at_col("ab", 10, 4, 40), (end, end), "past the end");
        assert_eq!(cluster_at_col("", 7, 0, 0), (7, 7), "empty row");
    }

    #[test]
    fn cluster_at_col_treats_a_zwj_family_as_one_cluster() {
        let fam = "👨\u{200d}👩\u{200d}👧"; // one cluster, width 2
        let len = u32::try_from(fam.len()).unwrap();
        assert_eq!(cluster_at_col(fam, 0, 0, 0), (0, len));
        assert_eq!(cluster_at_col(fam, 0, 0, 1), (0, len));
    }

    #[test]
    fn cluster_at_col_round_trips_with_cols_for_doc_range() {
        let text = "a日b👨\u{200d}👩\u{200d}👧c";
        for (off, g) in text.grapheme_indices(true) {
            let start = u32::try_from(off).unwrap();
            let end = start + u32::try_from(g.len()).unwrap();
            let (c0, c1) = cols_for_doc_range(text, 0, 3, &(start..end));
            for col in c0..c1 {
                assert_eq!(
                    cluster_at_col(text, 0, 3, col),
                    (start, end),
                    "column {col} of cluster {g:?}"
                );
            }
        }
    }

    #[test]
    fn wrap_range_lays_out_a_subrange_at_its_own_indent() {
        let doc = Document::parse("alpha beta gamma delta\n");
        let node = doc.node_for_block(BlockIdx(0));
        // The subrange "beta gamma delta", wrapped narrow with a 4-cell inset.
        let start = node.doc.start + 6;
        let mut rows = Vec::new();
        let mut first = false;
        let n = wrap_range(
            &doc,
            BlockIdx(0),
            start..node.doc.end,
            14,
            4,
            &cluster_width,
            &mut first,
            |r| rows.push(r),
        );
        assert!(n >= 2, "must wrap: {rows:?}");
        assert_eq!(rows[0].doc.start, start);
        assert_eq!(rows[0].indent, 4);
        assert!(matches!(
            rows[0].kind,
            RowKind::Text {
                first_in_block: false,
                continued: false
            }
        ));
        assert!(matches!(
            rows[1].kind,
            RowKind::Text {
                continued: true,
                ..
            }
        ));
        // Rows cover the subrange in order, gaps only at elided whitespace.
        assert_eq!(rows.last().unwrap().doc.end, node.doc.end);
    }

    #[test]
    fn wrap_range_of_an_empty_or_whitespace_range_emits_nothing() {
        let doc = Document::parse("a          b\n");
        let node = doc.node_for_block(BlockIdx(0));
        let mut first = false;
        let n = wrap_range(
            &doc,
            BlockIdx(0),
            node.doc.start + 2..node.doc.start + 8, // all spaces
            80,
            0,
            &cluster_width,
            &mut first,
            |_| {},
        );
        assert_eq!(n, 0);
    }
}
