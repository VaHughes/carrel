//! Nesting deeper than the column is drawn in from the left: the text keeps
//! a readable width, a dim `«` says levels are hidden, and what is painted
//! is still what a click lands on.
use carrel::app::App;
use carrel::render::{Painted, draw_full};
use carrel_core::{BlockIdx, Document, RowKind};
use ratatui::{Terminal, backend::TestBackend};
use std::fmt::Write as _;

fn screen(app: &App) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(app.cols, app.rows)).unwrap();
    let mut painted = Painted::default();
    terminal
        .draw(|f| draw_full(f, app, &mut painted, &mut std::collections::HashMap::new()))
        .unwrap();
    let buf = terminal.backend().buffer();
    (0..app.rows)
        .map(|y| (0..app.cols).map(|x| buf[(x, y)].symbol()).collect())
        .collect()
}

fn deep_list() -> String {
    let mut s = String::from("# Deep\n\n");
    for i in 0..300 {
        let _ = writeln!(
            s,
            "{}- item level {i} with some words in it",
            "  ".repeat(i)
        );
    }
    s
}

#[test]
fn every_row_of_a_deep_block_keeps_a_readable_width() {
    for src in [
        deep_list(),
        format!("{} deep quoted text here\n", ">".repeat(5000)),
    ] {
        for cols in [50u16, 80, 24] {
            let app = App::new("d.md".into(), Document::parse(&src), cols, 24);
            let mut rows = Vec::new();
            for b in 0..app.doc.block_count() {
                let node = app.doc.node_for_block(BlockIdx(b as u32));
                let bw = app.layout.block_width(&node.kind);
                app.layout.rows_for(&app.doc, BlockIdx(b as u32), &mut rows);
                for r in rows
                    .iter()
                    .filter(|r| matches!(r.kind, RowKind::Text { .. }))
                {
                    assert!(
                        r.indent + carrel::layout::min_text(bw) <= bw,
                        "{cols} cols: {r:?} leaves under {} cells",
                        carrel::layout::min_text(bw)
                    );
                }
            }
        }
    }
}

#[test]
fn the_deepest_item_is_readable_and_says_it_was_drawn_in() {
    let mut app = App::new("d.md".into(), Document::parse(&deep_list()), 50, 24);
    app.breadcrumb = false;
    app.on_resize(50, 24);
    carrel::app::update(&mut app, carrel::action::Action::GoToEnd);
    let lines = screen(&app);
    let all = lines.join("\n");
    assert!(all.contains("- item level 299 with"), "{all}");
    let row = lines.iter().find(|l| l.contains("level 299")).unwrap();
    assert!(row.trim_start().starts_with('«'), "{row:?}");
    // What is painted is what a click lands on.
    let y = u16::try_from(lines.iter().position(|l| l.contains("level 299")).unwrap()).unwrap();
    let x = u16::try_from(row.chars().take_while(|&c| c != 'l').count()).unwrap();
    let (start, end) = app.doc_span_at(x, y).expect("the text is clickable");
    assert_eq!(&app.doc.text[start as usize..end as usize], "l");
}
