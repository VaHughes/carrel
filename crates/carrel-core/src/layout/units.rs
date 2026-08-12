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
}

impl Unit {
    /// Width excluding trailing whitespace. **This is what fits, or doesn't.**
    pub(super) fn content_width(&self) -> u16 {
        self.width.saturating_sub(self.trailing_ws)
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
/// width-dependent step is [`split_to_fit`], deliberately kept separate.
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
        }
    })
}

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

/// Units from `text`, each already narrow enough for the packer to place.
///
/// The split is rare — only a word longer than the line — so it allocates only
/// when it happens. The common path yields the producer's unit untouched,
/// which matters: at one allocation per word a 1 MB document would make some
/// 175,000 of them per height pass.
pub(super) struct Fitted<'a, I> {
    pub(super) inner: I,
    pub(super) text: &'a str,
    pub(super) avail: u16,
    pub(super) w: &'a WidthFn,
    queue: std::vec::IntoIter<Unit>,
}

pub(super) fn fitted<'a, I: Iterator<Item = Unit>>(
    inner: I,
    text: &'a str,
    avail: u16,
    w: &'a WidthFn,
) -> Fitted<'a, I> {
    Fitted {
        inner,
        text,
        avail,
        w,
        queue: Vec::new().into_iter(),
    }
}

impl<I: Iterator<Item = Unit>> Iterator for Fitted<'_, I> {
    type Item = Unit;

    fn next(&mut self) -> Option<Unit> {
        if let Some(u) = self.queue.next() {
            return Some(u);
        }
        let u = self.inner.next()?;
        if u.content_width() <= self.avail {
            return Some(u);
        }
        self.queue = split_to_fit(&u, self.text, self.avail, self.w).into_iter();
        self.queue.next()
    }
}

/// Break a unit too wide for the viewport into pieces that fit.
///
/// This is the **emergency break** — the case where a single word is longer
/// than the line. Keeping it here rather than inside the packer means the
/// packer never sees a unit it cannot place, so its loop has no overflow case.
///
/// Splits only at grapheme cluster boundaries. A single cluster wider than
/// `avail` becomes its own piece and overhangs: content is never dropped, and
/// the frontend clips. `avail = 1` against a 2-cell CJK character is exactly
/// this case.
pub(super) fn split_to_fit(u: &Unit, text: &str, avail: u16, w: &WidthFn) -> Vec<Unit> {
    if u.content_width() <= avail {
        return vec![u.clone()];
    }

    let start = u.range.start as usize;
    let content_end = u.content_end() as usize;
    let mut out = Vec::new();
    let mut seg_start = start;
    let mut col = 0u16;

    for (off, g) in text[start..content_end].grapheme_indices(true) {
        let abs = start + off;
        let gw = w(g);
        if col > 0 && col.saturating_add(gw) > avail {
            out.push(Unit {
                range: seg_start as u32..abs as u32,
                width: col,
                trailing_ws: 0,
                trailing_ws_bytes: 0,
                mandatory: false,
            });
            seg_start = abs;
            col = 0;
        }
        col = col.saturating_add(gw);
    }

    // The final piece inherits the original's tail and its break kind.
    out.push(Unit {
        range: seg_start as u32..u.range.end,
        width: col.saturating_add(u.trailing_ws),
        trailing_ws: u.trailing_ws,
        trailing_ws_bytes: u.trailing_ws_bytes,
        mandatory: u.mandatory,
    });
    out
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
    fn split_to_fit_leaves_a_unit_that_already_fits_alone() {
        let u = units_of("abc")[0].clone();
        assert_eq!(split_to_fit(&u, "abc", 10, &cluster_width), vec![u]);
    }

    #[test]
    fn split_to_fit_never_breaks_inside_a_cluster() {
        // Two `e + combining acute` clusters: 3 bytes each, 1 cell each, and no
        // UAX #14 break between them. Splitting by bytes or by chars would tear
        // the accent off its base letter.
        let text = "e\u{301}e\u{301}";
        let u = units_of(text)[0].clone();
        assert_eq!(u.content_width(), 2, "one cell per cluster");
        let parts = split_to_fit(&u, text, 1, &cluster_width);
        let strs: Vec<&str> = parts
            .iter()
            .map(|p| &text[p.range.start as usize..p.range.end as usize])
            .collect();
        assert_eq!(strs, vec!["e\u{301}", "e\u{301}"]);
    }

    #[test]
    fn a_cluster_wider_than_the_viewport_overhangs_rather_than_vanishing() {
        // A 2-cell character at avail 1 cannot be split. It must still be
        // emitted, overhanging by one cell, rather than dropped.
        let text = "日";
        let u = units_of(text)[0].clone();
        let parts = split_to_fit(&u, text, 1, &cluster_width);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].width, 2, "overhangs rather than vanishing");
    }
}
