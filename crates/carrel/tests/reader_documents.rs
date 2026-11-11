//! Malformed and mixed markdown must remain readable at arbitrary terminal sizes.
use std::collections::HashMap;

use carrel::action::Action;
use carrel::app::{App, update};
use carrel::render::{Painted, draw_full};
use carrel_core::{BlockIdx, Document, search};
use proptest::prelude::*;
use ratatui::{Terminal, backend::TestBackend};

// Deliberately joined without balancing delimiters: a streamed document can
// end halfway through any of these constructs. Unicode and entities exercise
// the source/display seam, while nested containers exercise block geometry.
const PARTS: &[&str] = &[
    "# Heading\n\n",
    "## Café 界\n\n",
    "alpha **beta** _gamma_ ~gone~ ^up^\n\n",
    "e\u{301} 👩‍💻 🇯🇵\n",
    "\u{200d}\u{301}\u{ad}",
    "```rust\nfn main() {\n",
    "```\n\n",
    "~~~text\n",
    "> > quoted\n>\n",
    "- [ ] task\n  - nested\n",
    "1. ordered\n   continuation\n",
    "| one | two |\n| --- | --- |\n| 界 | e\u{301} |\n",
    "| a | b | c | d |\n",
    "<details><summary>Summary</summary>\n\n",
    "</details>\n",
    "<!-- hidden ",
    " -->\n",
    "[^note]: note text\n\n",
    "[ref](#heading)[^note] ![alt](missing.png)\n",
    "[ref]: #heading\n",
    "---\ntitle: A title\ntags: [one, two]\n---\n",
    "$\\frac{a}{b}$ and $$x^2$$\n\n",
    "&amp; &#x754c; &notAnEntity;\n",
    "[unclosed](<\n",
    "<https://example.invalid/a?q=x&b=2>\n",
    "\t    indentation\n\n",
    "\r\n\n",
];

proptest! {
    #[test]
    fn mixed_markdown_keeps_valid_bytes_and_searches_through_layout_changes(
        pieces in prop::collection::vec(prop::sample::select(PARTS), 0..45),
        width in 0u16..120,
        height in 0u16..40,
    ) {
        let source = pieces.concat();
        let mut app = App::new("mixed.md".into(), Document::parse(&source), 80, 24);
        let text = app.doc.text.clone();
        for i in 0..app.doc.block_count() {
            let range = &app.doc.node_for_block(BlockIdx(i as u32)).doc;
            prop_assert!(range.start <= range.end && range.end as usize <= text.len());
            prop_assert!(text.is_char_boundary(range.start as usize));
            prop_assert!(text.is_char_boundary(range.end as usize));
        }
        let needle = text.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_string());
        if let Some(needle) = needle {
            let matches = search(&app.doc, &needle, true);
            prop_assert!(!matches.ranges.is_empty(), "visible letter {needle:?} was not searchable");
            app.matches = Some(matches);
        }
        let ranges = app.matches.as_ref().map(|m| m.ranges.clone());
        let mut painted = Painted::default();
        for action in [Action::GoToEnd, Action::FoldAll, Action::UnfoldAll, Action::TableToggle, Action::RenderedToggle] {
            update(&mut app, action);
            for (cols, rows) in [(width, height), (1, 0), (0, 1), (1, 1), (80, 24)] {
                let anchor = app.view.anchor;
                app.on_resize(cols, rows);
                prop_assert_eq!(app.view.anchor, anchor, "resize changed the position");
                prop_assert!(text.is_char_boundary(app.view.anchor as usize));
                prop_assert!(app.view.scroll_row <= app.layout.max_scroll(app.text_h()));
                let mut terminal = Terminal::new(TestBackend::new(cols, rows)).unwrap();
                terminal.draw(|frame| draw_full(frame, &app, &mut painted, &mut HashMap::new())).unwrap();
                prop_assert_eq!(&app.doc.text, &text, "layout or paint changed display text");
                prop_assert_eq!(app.matches.as_ref().map(|m| &m.ranges), ranges.as_ref());
                for target in painted.targets.as_slice() {
                    prop_assert!(target.zone.x.saturating_add(target.zone.w) <= cols);
                    prop_assert!(target.zone.y.saturating_add(target.zone.h) <= rows);
                }
            }
        }
    }
}
