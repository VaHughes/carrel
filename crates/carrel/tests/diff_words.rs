use carrel::{action::Where, app::App, render, theme};
use carrel_core::{BlockIdx, Document, TokenKind, cols_for_doc_range};
use ratatui::{Terminal, backend::TestBackend};

#[test]
fn changed_word_styles_survive_wrapping_and_search_overrides_them() {
    for cols in [80, 24] {
        let doc =
            Document::parse("```diff\n-The quiet reader works.\n+The quick reader works.\n```\n");
        let mut app = App::new("diff.md".into(), doc, cols, 14);
        let mut terminal = Terminal::new(TestBackend::new(cols, 14)).unwrap();
        terminal.draw(|f| render::draw(f, &app)).unwrap();
        let mut rows = Vec::new();
        app.layout.rows_for(&app.doc, BlockIdx(0), &mut rows);
        let mut words_seen = 0;
        for token in app
            .doc
            .tokens(BlockIdx(0))
            .iter()
            .filter(|t| matches!(t.kind, TokenKind::InsertedWord | TokenKind::DeletedWord))
        {
            for (i, row) in rows.iter().enumerate() {
                let range = token.doc.start.max(row.doc.start)..token.doc.end.min(row.doc.end);
                if range.is_empty() {
                    continue;
                }
                let text = &app.doc.text[row.doc.start as usize..row.doc.end as usize];
                let (left, right) = cols_for_doc_range(text, row.doc.start, row.indent, &range);
                for x in left..right {
                    let cell = &terminal.backend().buffer()[(
                        app.block_span_x(BlockIdx(0)).0 + x,
                        app.text_y() + u16::try_from(i).unwrap(),
                    )];
                    assert_eq!(cell.bg, theme::token(token.kind).bg.unwrap());
                    assert!(cell.modifier.contains(ratatui::style::Modifier::UNDERLINED));
                }
                words_seen += 1;
            }
        }
        assert!(words_seen >= 2);
        let mut matches = carrel_core::search(&app.doc, "quick", false);
        matches.current = Some(0);
        let byte = matches.ranges[0].start;
        app.matches = Some(matches);
        app.reveal_byte(byte, app.text_h(), Where::Top);
        terminal.draw(|f| render::draw(f, &app)).unwrap();
        let row = rows.iter().find(|r| r.doc.contains(&byte)).unwrap();
        let text = &app.doc.text[row.doc.start as usize..row.doc.end as usize];
        let (x, _) = cols_for_doc_range(text, row.doc.start, row.indent, &(byte..byte + 1));
        let cell = &terminal.backend().buffer()[(
            app.block_span_x(BlockIdx(0)).0 + x,
            app.text_y()
                + u16::try_from(
                    rows.iter().position(|r| r.doc.contains(&byte)).unwrap() as u32
                        - app.view.scroll_row,
                )
                .unwrap(),
        )];
        assert_eq!(cell.bg, theme::match_current().bg.unwrap());
    }
}
