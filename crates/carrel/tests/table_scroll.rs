//! Pan is presentation state: text, match ranges, and pointer coordinates agree.
use carrel::{
    action::{Action, Where},
    app::{App, update},
    render,
};
use carrel_core::{BlockIdx, Document};
use ratatui::{Terminal, backend::TestBackend};

const TABLE: &str = "| first | middle | last |\n|---|---|---|\n| α界👩‍💻 words in the first column | another long value | [destination](https://example.com) |\n";

fn app() -> App {
    App::new("table.md".into(), Document::parse(TABLE), 32, 14)
}
fn pan(app: &mut App, delta: i32) {
    update(
        app,
        Action::TableScroll {
            block: Some(BlockIdx(0)),
            delta,
        },
    );
}

#[test]
fn column_rows_stay_whole_and_pan_reaches_the_last_column() {
    let mut app = app();
    let text = app.doc.text.clone();
    pan(&mut app, i32::MAX);
    assert!(app.wrap_tables);
    assert_eq!(app.layout.content_height(&app.doc, BlockIdx(0)), 2);
    assert_eq!(
        app.table_offset(BlockIdx(0)),
        app.table_max_offset(BlockIdx(0))
    );
    let mut terminal = Terminal::new(TestBackend::new(32, 14)).unwrap();
    terminal.draw(|f| render::draw(f, &app)).unwrap();
    let buffer = terminal.backend().buffer();
    let screen: String = buffer
        .content
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect();
    assert!(screen.contains("destination"), "{screen}");
    assert!(screen.contains("[←]"), "{screen}");
    assert_eq!(text, app.doc.text);
    pan(&mut app, i32::MIN);
    assert_eq!(app.table_offset(BlockIdx(0)), 0);
}

#[test]
fn painted_graphemes_and_pointer_bytes_agree_at_every_pan_position() {
    let mut app = app();
    update(&mut app, Action::TableToggle);
    for offset in 0..=app.table_max_offset(BlockIdx(0)) {
        app.table_offsets.insert(BlockIdx(0), offset);
        let mut terminal = Terminal::new(TestBackend::new(32, 14)).unwrap();
        terminal.draw(|f| render::draw(f, &app)).unwrap();
        let buffer = terminal.backend().buffer();
        for y in app.text_y()..app.text_y() + 2 {
            for x in app.block_span_x(BlockIdx(0)).0..30 {
                let symbol = buffer[(x, y)].symbol();
                if symbol.trim().is_empty() || symbol == "│" {
                    continue;
                }
                let (start, end) = app.doc_span_at(x, y).expect("painted content is hittable");
                assert_eq!(
                    &app.doc.text[start as usize..end as usize],
                    symbol,
                    "offset {offset}, ({x}, {y})"
                );
            }
        }
    }
}

#[test]
fn search_reveals_hidden_columns_and_survives_resize_and_mode_changes() {
    let mut app = app();
    update(&mut app, Action::TableToggle);
    let mut matches = carrel_core::search(&app.doc, "destination", false);
    matches.current = Some(0);
    let ranges = matches.ranges.clone();
    let byte = ranges[0].start;
    app.matches = Some(matches);
    app.reveal_byte(byte, app.text_h(), Where::Top);
    assert!(app.table_offset(BlockIdx(0)) > 0);
    let mut rows = Vec::new();
    app.layout.rows_for(&app.doc, BlockIdx(0), &mut rows);
    assert!(rows.iter().any(|r| app.visible_row(r).doc.contains(&byte)));
    app.on_resize(16, 14);
    assert!(rows.iter().any(|r| app.visible_row(r).doc.contains(&byte)));
    app.on_resize(120, 14);
    assert_eq!(app.table_offset(BlockIdx(0)), 0);
    app.table_offsets.clear(); // It fit without any pan at the wider size.
    app.on_resize(32, 14);
    assert!(rows.iter().any(|r| app.visible_row(r).doc.contains(&byte)));
    update(&mut app, Action::TableToggle);
    assert_eq!(app.table_offset(BlockIdx(0)), 0);
    assert_eq!(app.matches.as_ref().unwrap().ranges, ranges);
}

#[test]
fn reload_discards_table_offsets() {
    let mut app = app();
    pan(&mut app, 2);
    app.reload_from("plain text");
    assert!(app.table_offsets.is_empty());
}

#[test]
fn clipped_links_register_only_the_visible_text_and_buttons_act_on_their_table() {
    let mut app = app();
    pan(&mut app, i32::MAX);
    let mut terminal = Terminal::new(TestBackend::new(32, 14)).unwrap();
    let mut painted = render::Painted::default();
    terminal
        .draw(|f| render::draw_full(f, &app, &mut painted, &mut std::collections::HashMap::new()))
        .unwrap();
    let link = painted
        .links
        .iter()
        .find(|l| l.url == "https://example.com")
        .unwrap();
    assert_eq!(link.text, "destination");
    assert!(
        link.x + carrel_core::display_width(&link.text)
            <= app.block_span_x(BlockIdx(0)).0 + app.block_span_x(BlockIdx(0)).1
    );
    let buttons: Vec<_> = painted
        .targets
        .as_slice()
        .iter()
        .filter(|t| matches!(t.action, Action::TableScroll { .. }))
        .collect();
    assert_eq!(buttons.len(), 1, "only the enabled left button, once");
    let old = app.table_offset(BlockIdx(0));
    update(&mut app, buttons[0].action);
    assert!(app.table_offset(BlockIdx(0)) < old);
    // The link straddles the right edge after one leftward step.
    terminal
        .draw(|f| render::draw_full(f, &app, &mut painted, &mut std::collections::HashMap::new()))
        .unwrap();
    let link = painted
        .links
        .iter()
        .find(|l| l.url == "https://example.com")
        .unwrap();
    assert!("destination".starts_with(&link.text));
    assert!(link.text.len() < "destination".len());
}

#[test]
fn context_menu_pans_the_clicked_table_and_leaves_its_neighbor_alone() {
    let src = format!("{TABLE}\n{TABLE}");
    let mut app = App::new("tables.md".into(), Document::parse(&src), 32, 20);
    update(&mut app, Action::TableToggle);
    let second = BlockIdx(1);
    let byte = app.doc.node_for_block(second).doc.start;
    let menu = carrel::menu::context(&app, byte);
    let action = menu
        .iter()
        .find(|item| item.label == "Scroll table right")
        .unwrap()
        .action
        .unwrap();
    update(&mut app, action);
    assert_eq!(app.table_offset(BlockIdx(0)), 0);
    assert!(app.table_offset(second) > 0);
}

#[test]
fn tables_wider_than_u16_are_still_reachable() {
    let cell = "x".repeat(33000);
    let source = format!("| a | b | final |\n|---|---|---|\n| {cell} | {cell} | reached |\n");
    let mut app = App::new("wide.md".into(), Document::parse(&source), 32, 14);
    pan(&mut app, i32::MAX);
    assert!(app.table_offset(BlockIdx(0)) > u32::from(u16::MAX));
    let mut rows = Vec::new();
    app.layout.rows_for(&app.doc, BlockIdx(0), &mut rows);
    let visible = app.visible_row(&rows[1]);
    assert!(app.doc.text[visible.doc.start as usize..visible.doc.end as usize].contains("reached"));
}

#[test]
fn shift_arrows_pan_with_counts_and_plain_arrows_keep_navigation() {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut keys = carrel::keys::Keys::new();
    assert_eq!(
        keys.map(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE), false),
        None
    );
    assert_eq!(
        keys.map(KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT), false),
        Some(Action::TableScroll {
            block: None,
            delta: 3
        })
    );
    assert_eq!(
        keys.map(KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT), false),
        Some(Action::TableScroll {
            block: None,
            delta: -1
        })
    );
    assert_eq!(
        keys.map(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE), false),
        Some(Action::Back)
    );
    assert_eq!(
        keys.map(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE), false),
        Some(Action::LinkFollow)
    );
}
