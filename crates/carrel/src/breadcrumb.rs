//! The breadcrumb band's state: which section path, truncated to fit.
//!
//! Pure selector over [`App`], like [`crate::footer`] — no ratatui, no
//! terminal, testable frame-free, reusable by a GTK frontend verbatim.
//! What a section *is* comes from the core's section index
//! (`Document::section_path`), so this file only decides presentation:
//! the pop rule and the truncation order.

use crate::app::App;
use carrel_core::{NodeId, display_width};

/// What separates segments when painted. Public so paint and the width
/// arithmetic here can never disagree about its cost.
pub const SEP: &str = " ▸ ";
/// What replaces dropped outer segments.
pub const ELLIPSIS: &str = "… ▸ ";

/// The crumb for the current scroll position, or `None` when there is no
/// band at all (feature off, or a document with no headings). An empty
/// `segments` means the band exists but has nothing to say — the reader is
/// above the first heading — and paints blank, so the height never
/// flickers while scrolling.
#[derive(Debug, PartialEq, Eq)]
pub struct Crumb {
    /// `(heading, text)` outermost first, already truncated to the width.
    pub segments: Vec<(NodeId, String)>,
    /// Dropped outer segments are represented by a leading [`ELLIPSIS`].
    pub elided: bool,
}

/// Derive the crumb for the top visible row, fitted to `width` cells.
#[must_use]
pub fn of(app: &App, width: u16) -> Option<Crumb> {
    if !app.band() {
        return None;
    }
    let top_block = app.layout.block_at_row(app.view.scroll_row);
    if top_block.get() >= app.doc.block_count() {
        return Some(Crumb {
            segments: Vec::new(),
            elided: false,
        });
    }
    let node = app.doc.node_for_block(top_block);
    let mut path = app.doc.section_path(node.doc.start);
    // The pop rule: when the top visible block IS the innermost heading,
    // showing it in the band would double it directly above itself.
    if path.last() == Some(&node.id) {
        path.pop();
    }
    let mut segments: Vec<(NodeId, String)> = path
        .into_iter()
        .map(|id| {
            let n = &app.doc.nodes[id.0 as usize];
            (
                id,
                app.doc.text[n.doc.start as usize..n.doc.end as usize].to_string(),
            )
        })
        .collect();

    // Fit: drop outermost first behind the ellipsis — the nearest section
    // is the one a lost reader needs. Widths come from the core's cluster
    // measurement; never per-char sums (the ZWJ rule).
    let mut elided = false;
    loop {
        let seps = u16::try_from(segments.len().saturating_sub(1)).unwrap_or(u16::MAX);
        let sep_cost = display_width(SEP).saturating_mul(seps);
        let lead = if elided { display_width(ELLIPSIS) } else { 0 };
        let total: u16 = segments
            .iter()
            .map(|(_, t)| display_width(t))
            .fold(0u16, u16::saturating_add)
            .saturating_add(sep_cost)
            .saturating_add(lead);
        if total <= width || segments.len() <= 1 {
            break;
        }
        segments.remove(0);
        elided = true;
    }
    Some(Crumb { segments, elided })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{Action, Span};
    use crate::app::update;
    use carrel_core::Document;

    const SRC: &str = "\
# Top\n\nintro paragraph\n\n## Middle\n\nmid one\n\nmid two\n\n### Inner\n\ndeep one\n\ndeep two\n";

    fn app_scrolled_to(needle: &str) -> App {
        let mut a = App::new("t.md".into(), Document::parse(SRC), 40, 8);
        let at = u32::try_from(a.doc.text.find(needle).expect("needle")).unwrap();
        while {
            let b = a.layout.block_at_row(a.view.scroll_row);
            a.doc.node_for_block(b).doc.end <= at
        } {
            update(&mut a, Action::Scroll(Span::Line, 1));
        }
        a
    }

    fn words(c: &Crumb) -> Vec<&str> {
        c.segments.iter().map(|(_, t)| t.as_str()).collect()
    }

    #[test]
    fn deep_in_a_section_the_path_reads_outermost_first() {
        let a = app_scrolled_to("deep two");
        let c = of(&a, 40).expect("band on");
        assert_eq!(words(&c), ["Top", "Middle", "Inner"]);
        assert!(!c.elided);
    }

    #[test]
    fn the_top_heading_itself_is_never_doubled() {
        // Put the "Inner" heading block exactly at the top visible row.
        let mut a = App::new("t.md".into(), Document::parse(SRC), 40, 8);
        let inner = (0..a.doc.block_count())
            .map(|i| carrel_core::BlockIdx(u32::try_from(i).unwrap()))
            .find(|b| {
                let n = a.doc.node_for_block(*b);
                &a.doc.text[n.doc.start as usize..n.doc.end as usize] == "Inner"
            })
            .expect("the Inner heading is a block");
        let row = a.layout.row_start(inner);
        while a.view.scroll_row < row {
            update(&mut a, Action::Scroll(Span::Line, 1));
        }
        assert_eq!(a.layout.block_at_row(a.view.scroll_row), inner);
        let c = of(&a, 40).expect("band on");
        assert_eq!(words(&c), ["Top", "Middle"], "the visible heading pops");
    }

    #[test]
    fn above_the_first_heading_the_band_is_blank_not_absent() {
        let a = App::new("t.md".into(), Document::parse(SRC), 40, 8);
        let c = of(&a, 40).expect("band exists");
        // The top visible block is the H1 itself: path = [Top], popped = [].
        assert_eq!(words(&c), Vec::<&str>::new());
    }

    #[test]
    fn no_headings_means_no_band_at_all() {
        let a = App::new("t.md".into(), Document::parse("just prose\n"), 40, 8);
        assert_eq!(of(&a, 40), None);
    }

    #[test]
    fn toggled_off_means_none_even_with_headings() {
        let mut a = App::new("t.md".into(), Document::parse(SRC), 40, 8);
        a.breadcrumb = false;
        assert_eq!(of(&a, 40), None);
    }

    #[test]
    fn too_narrow_drops_the_outermost_segments_first() {
        let a = app_scrolled_to("deep two");
        // "Middle ▸ Inner" is 14 cells; "Top ▸ Middle ▸ Inner" is 20.
        let c = of(&a, 19).expect("band on");
        assert_eq!(words(&c), ["Middle", "Inner"], "Top drops first");
        assert!(c.elided, "and the ellipsis marks the drop");
        let tight = of(&a, 5).expect("band on");
        assert_eq!(
            words(&tight),
            ["Inner"],
            "at worst the innermost survives alone"
        );
    }
}
