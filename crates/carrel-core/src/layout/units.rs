//! Text → break units. **All Unicode in the reflow layer lives here.**
//!
//! See the reflow-layer design doc (notes repo) §5.
//!
//! The point of this module is that [`super::pack`] can then know nothing about
//! text: it packs measured units, and its invariants are checkable against units
//! built by hand with no string behind them at all.

use std::ops::Range;

use unicode_linebreak::BreakOpportunity;
use unicode_segmentation::UnicodeSegmentation;

use super::WidthFn;
use crate::hyphen::Hyphenation;

/// One UAX #14 break opportunity's worth of text, measured.
///
/// `width` includes trailing whitespace; [`Unit::content_width`] is what the
/// fit test uses, because whitespace at a break is elided rather than painted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Unit {
    /// Byte range within the chunk text, relative to its start.
    pub range: Range<u32>,
    /// Display cells of the whole unit, trailing whitespace included.
    pub width: u16,
    /// Cells of `width` that are trailing whitespace.
    pub trailing_ws: u16,
    /// Bytes of `range` that are trailing whitespace.
    pub trailing_ws_bytes: u32,
    /// Ends at a mandatory break — a hard newline, or the end of the text.
    pub mandatory: bool,
    /// Its content ends in a soft hyphen (U+00AD): the author's own division
    /// point. Invisible and zero cells mid-row; a row that ends here shows a
    /// hyphen, so the unit fits only where that hyphen would too.
    pub soft_hyphen: bool,
}

impl Unit {
    /// Width excluding trailing whitespace. **This is what fits, or doesn't.**
    pub(super) fn content_width(&self) -> u16 {
        self.width.saturating_sub(self.trailing_ws)
    }

    /// Cells this unit needs to stand at the end of a row: its content, and
    /// the hyphen its soft hyphen becomes there.
    pub(super) fn end_width(&self) -> u16 {
        self.content_width()
            .saturating_add(u16::from(self.soft_hyphen && !self.mandatory))
    }

    /// End of the painted content, excluding trailing whitespace.
    ///
    /// A row ending here is what produces the documented gap between
    /// consecutive rows' doc ranges (`architecture.md` §3.3).
    pub(super) fn content_end(&self) -> u32 {
        self.range.end.saturating_sub(self.trailing_ws_bytes)
    }
}

/// Split `text` at UAX #14 break opportunities, measuring each unit.
///
/// **Width-independent.** Nothing here consults a viewport, which is what would
/// let a future implementation cache the result across widths. The only
/// width-dependent step is [`split`], which the packer asks for only when a
/// unit is wider than a whole row.
///
/// Hard newlines need no special handling: `unicode-linebreak` already reports
/// them as [`BreakOpportunity::Mandatory`], and the newline itself is trailing
/// whitespace of the unit it ends.
pub(super) fn units<'a>(text: &'a str, w: &'a WidthFn) -> impl Iterator<Item = Unit> + 'a {
    let mut prev = 0usize;
    unicode_linebreak::linebreaks(text).map(move |(pos, op)| {
        let start = prev;
        prev = pos;
        let s = &text[start..pos];
        let (width, trailing_ws, trailing_ws_bytes) = measure(s, w);
        Unit {
            range: start as u32..pos as u32,
            width,
            trailing_ws,
            trailing_ws_bytes,
            mandatory: op == BreakOpportunity::Mandatory,
            // Only where the break is AT the soft hyphen: one followed by a
            // space breaks at the space, and shows nothing.
            soft_hyphen: trailing_ws_bytes == 0 && s.ends_with(SOFT_HYPHEN),
        }
    })
}

/// U+00AD, the author's own division point: UAX #14 breaks after it, and
/// it is drawn only where it does.
const SOFT_HYPHEN: char = '\u{AD}';

/// Measure a unit: total cells, trailing-whitespace cells, trailing bytes.
fn measure(s: &str, w: &WidthFn) -> (u16, u16, u32) {
    let content_len = s.trim_end_matches(char::is_whitespace).len();
    let (content, tail) = s.split_at(content_len);
    let cw = sum_width(content, w);
    let tw = sum_width(tail, w);
    (cw.saturating_add(tw), tw, tail.len() as u32)
}

fn sum_width(s: &str, w: &WidthFn) -> u16 {
    // Printable ASCII is one cluster and one cell per byte, so both the
    // grapheme walk and the width table are skippable. This relies on the
    // documented [`WidthFn`] precondition that printable ASCII measures 1 —
    // true of every terminal, and unaffected by DEC mode 2027, which changes
    // how *clusters* are counted, not how `a` is.
    if s.is_ascii() && !s.bytes().any(|b| b < 0x20 || b == 0x7f) {
        return u16::try_from(s.len()).unwrap_or(u16::MAX);
    }
    s.graphemes(true)
        .fold(0u16, |acc, g| acc.saturating_add(w(g)))
}

/// How a row that ends inside a unit is marked: what an emergency break
/// leaves at the end of the row it cuts.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(super) enum Cut {
    /// Nothing: a code line, whose next row carries the `↳` marker instead,
    /// or a row too narrow to give up a cell.
    Plain,
    /// A word divided where its language's patterns allow, or at the
    /// author's own soft hyphen: one cell for a hyphen.
    Hyphen,
    /// Anything else cut where no break is allowed — a hash, a token, a
    /// long URL segment, a word of no language here: one cell for a
    /// continuation mark.
    Marked,
}

/// Cells a row gives up to the mark a [`Cut::Hyphen`] or [`Cut::Marked`]
/// ends it with. The same cell either way.
pub(super) const MARK_COLS: u16 = 1;

/// Where words may be divided in a line, and which bytes of it must not be:
/// what [`divide`] and [`split`] need beyond the unit itself. `fixed(range)`
/// answers whether any byte of the range sits in a style run that must not
/// divide (code, math, a script); the caller owns the document.
pub(super) struct Soft<'a> {
    pub(super) language: Hyphenation,
    pub(super) fixed: &'a dyn Fn(Range<u32>) -> bool,
}

/// The emergency break: cut `u`, which is wider than a whole row of `avail`
/// cells, into a head that fills the row and the tail that follows.
///
/// Asked by the packer only for a unit that does not fit an EMPTY row, so it
/// is the case where a single word is longer than the line. Keeping the
/// question on this side of the seam means the packer still never sees a
/// string.
///
/// - A word its language's patterns can divide (`soft`) is divided at the
///   latest point whose head fits with its hyphen: a long German compound in
///   a narrow column reads as one, not as letters run off the edge.
/// - Anything else is cut at a grapheme cluster boundary. With `mark`, the
///   head leaves one cell for a continuation mark, so the reader can see the
///   row did not end where the text does — a commit hash cut after twelve
///   characters otherwise reads as a twelve-character hash. Never a hyphen:
///   a hyphen in a URL or a token is a character somebody types back in.
///
/// Splits only at grapheme cluster boundaries. `None` when the unit is a
/// single cluster wider than `avail`: it is placed whole and overhangs —
/// content is never dropped, and the frontend clips. `avail = 1` against a
/// 2-cell CJK character is exactly this case.
pub(super) fn split(
    u: &Unit,
    text: &str,
    avail: u16,
    w: &WidthFn,
    mark: bool,
    soft: Option<&Soft>,
) -> Option<(Unit, Unit, Cut)> {
    if let Some(cut) = soft.and_then(|soft| divide_long(u, text, avail, w, soft)) {
        return Some(cut);
    }
    let reserve = if mark && avail > MARK_COLS {
        MARK_COLS
    } else {
        0
    };
    let budget = avail - reserve;
    let start = u.range.start as usize;
    let content_end = u.content_end() as usize;
    let mut col = 0u16;
    let mut at = None;
    for (off, g) in text[start..content_end].grapheme_indices(true) {
        let gw = w(g);
        if col > 0 && col.saturating_add(gw) > budget {
            at = Some(start + off);
            break;
        }
        col = col.saturating_add(gw);
    }
    let at = at?;
    // A first cluster wider than the budget went on the row alone; it keeps
    // its mark only when the mark still fits beside it.
    let cut = if reserve > 0 && col + reserve <= avail {
        Cut::Marked
    } else {
        Cut::Plain
    };
    Some(halves(u, at as u32, col, cut))
}

/// `u` cut at byte `at`, the head `head_width` cells wide.
fn halves(u: &Unit, at: u32, head_width: u16, cut: Cut) -> (Unit, Unit, Cut) {
    (
        Unit {
            range: u.range.start..at,
            width: head_width,
            trailing_ws: 0,
            trailing_ws_bytes: 0,
            mandatory: false,
            soft_hyphen: false,
        },
        Unit {
            range: at..u.range.end,
            width: u.width.saturating_sub(head_width),
            trailing_ws: u.trailing_ws,
            trailing_ws_bytes: u.trailing_ws_bytes,
            mandatory: u.mandatory,
            soft_hyphen: u.soft_hyphen,
        },
        cut,
    )
}

/// [`split`]'s first choice: a word longer than the row, divided at its
/// language's latest point that fits `avail` with the hyphen.
///
/// `u` may be a word or the tail of one a previous row divided, so the word
/// is found by looking back to the whitespace before it — its patterns are
/// the whole word's, never the tail's. It must still be free-standing:
/// whitespace, or the line's edge, on both sides.
fn divide_long(
    u: &Unit,
    text: &str,
    avail: u16,
    w: &WidthFn,
    soft: &Soft,
) -> Option<(Unit, Unit, Cut)> {
    let start = u.range.start as usize;
    let end = u.content_end() as usize;
    let word_start = text[..start]
        .char_indices()
        .rev()
        .find(|&(_, c)| c.is_whitespace())
        .map_or(0, |(i, c)| i + c.len_utf8());
    if text[end..]
        .chars()
        .next()
        .is_some_and(|c| !c.is_whitespace())
    {
        return None;
    }
    let content = &text[word_start..end];
    let word = crate::hyphen::word_in(soft.language, content)?;
    let (ws, we) = (word_start + word.start, word_start + word.end);
    if (soft.fixed)(ws as u32..we as u32) {
        return None;
    }
    let max_head = avail.checked_sub(MARK_COLS)?;
    let (at, head_width) = crate::hyphen::divisions(soft.language, &text[ws..we])
        .map(|(_, b)| ws + b)
        .filter(|&at| at > start)
        .map(|at| (at, sum_width(&text[start..at], w)))
        .take_while(|&(_, cells)| cells <= max_head)
        .last()?;
    Some(halves(u, at as u32, head_width, Cut::Hyphen))
}

/// Divide a word that missed the end of its row: a head that fits in `room`
/// cells **with its hyphen**, and the tail that starts the next row.
///
/// The soft break, as opposed to [`split`]'s emergency one: this is for a
/// word dividing only to fill the hole it leaves. `None` whenever the unit
/// is not a free-standing word [`crate::hyphen`] will divide — which is most
/// units, deliberately.
///
/// *Free-standing* means whitespace on both sides. A break opportunity is not
/// a word boundary: UAX #14 also breaks after the `/` in `src/main`, after the
/// hyphen in `well-known` and around an em dash, and what sits on either side
/// of those is part of something larger. It is also what keeps the tail of a
/// word an earlier row divided from being divided again as if it were a word.
pub(super) fn divide(
    u: &Unit,
    text: &str,
    room: u16,
    w: &WidthFn,
    soft: &Soft,
) -> Option<(Unit, Unit)> {
    let start = u.range.start as usize;
    let after_space = text[..start]
        .chars()
        .next_back()
        .is_none_or(char::is_whitespace);
    if !after_space || u.trailing_ws_bytes == 0 {
        return None;
    }
    let content = &text[start..u.content_end() as usize];
    let word = crate::hyphen::word_in(soft.language, content)?;
    if (soft.fixed)((start + word.start) as u32..(start + word.end) as u32) {
        return None;
    }
    // Opening punctuation rides with the head and costs cells like any other.
    let lead = sum_width(&content[..word.start], w);
    let max_head = room.checked_sub(lead + MARK_COLS)?;
    let cut =
        crate::hyphen::division(soft.language, &content[word.clone()], usize::from(max_head))?;
    let at = (start + word.start + cut) as u32;
    let head_width = sum_width(&text[start..at as usize], w);
    let (head, tail, _) = halves(u, at, head_width, Cut::Hyphen);
    Some((head, tail))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::cluster_width;

    fn units_of(s: &str) -> Vec<Unit> {
        units(s, &cluster_width).collect()
    }

    #[test]
    fn a_trailing_space_is_measured_separately_from_content() {
        let u = &units_of("abc def")[0];
        assert_eq!(
            &"abc def"[u.range.start as usize..u.range.end as usize],
            "abc "
        );
        assert_eq!(u.width, 4);
        assert_eq!(u.trailing_ws, 1);
        assert_eq!(u.content_width(), 3, "the space must not count toward fit");
    }

    #[test]
    fn a_hard_newline_is_a_mandatory_break_needing_no_special_case() {
        let us = units_of("a\nb");
        assert!(
            us[0].mandatory,
            "the unit ending at the newline is mandatory"
        );
        assert_eq!(us[0].content_end(), 1, "the newline itself is not painted");
    }

    #[test]
    fn an_emergency_break_never_falls_inside_a_cluster() {
        // Two `e + combining acute` clusters: 3 bytes each, 1 cell each, and no
        // UAX #14 break between them. Splitting by bytes or by chars would tear
        // the accent off its base letter.
        let text = "e\u{301}e\u{301}";
        let u = units_of(text)[0].clone();
        assert_eq!(u.content_width(), 2, "one cell per cluster");
        let (head, tail, cut) = split(&u, text, 1, &cluster_width, true, None).unwrap();
        let strs = [head, tail].map(|p| &text[p.range.start as usize..p.range.end as usize]);
        assert_eq!(strs, ["e\u{301}", "e\u{301}"]);
        assert_eq!(cut, Cut::Plain, "one cell has no room for a mark");
    }

    #[test]
    fn a_cluster_wider_than_the_viewport_overhangs_rather_than_vanishing() {
        // A 2-cell character at avail 1 cannot be split. It must still be
        // emitted, overhanging by one cell, rather than dropped.
        let text = "日";
        let u = units_of(text)[0].clone();
        assert_eq!(
            split(&u, text, 1, &cluster_width, true, None),
            None,
            "placed whole: it overhangs rather than vanishing"
        );
    }
}
