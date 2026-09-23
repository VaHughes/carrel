//! Full-screen images preserve document coordinates and own input while open.
use carrel::action::{Action, Span};
use carrel::app::{App, Outcome, update};
use carrel::keys::Keys;
use carrel_core::{BlockIdx, Document};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn app() -> App {
    App::new(
        "test.md".into(),
        Document::parse("![first](one.png)\n\ntext\n\n![second](https://example.com/two.png)\n"),
        100,
        20,
    )
}

#[test]
fn viewer_navigates_in_document_order_and_restores_reading_position() {
    let mut app = app();
    let anchor = app.view.anchor;
    update(&mut app, Action::LinkFollow);
    assert_eq!(app.lightbox, Some(BlockIdx(0)));
    update(&mut app, Action::ImageStep(1));
    assert_eq!(app.lightbox, Some(BlockIdx(2)));
    update(&mut app, Action::ImageStep(1));
    assert_eq!(app.lightbox, Some(BlockIdx(0)));
    update(&mut app, Action::ImageStep(-1));
    assert_eq!(app.lightbox, Some(BlockIdx(2)));
    assert_eq!(
        update(&mut app, Action::Scroll(Span::Page, 1)),
        Outcome::Idle
    );
    assert_eq!(update(&mut app, Action::GoHome), Outcome::Idle);
    update(&mut app, Action::Dismiss);
    assert_eq!(app.lightbox, None);
    assert_eq!(app.view.anchor, anchor);
}

#[test]
fn viewer_reports_failure_and_never_exposes_document_links() {
    let mut app = app();
    let block = BlockIdx(0);
    app.image_errors.insert(block, "missing file".into());
    update(&mut app, Action::ImageOpen(Some(block)));
    let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
    let mut painted = carrel::render::Painted::default();
    terminal
        .draw(|f| {
            carrel::render::draw_full(f, &app, &mut painted, &mut std::collections::HashMap::new());
        })
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect();
    assert!(text.contains("Cannot display image: missing file"));
    assert!(text.contains("[close]"));
    assert!(painted.links.is_empty());
    assert_eq!(painted.targets.hit(1, 19).unwrap().action, Action::Dismiss);
    assert_eq!(painted.targets.hit(80, 10).unwrap().action, Action::Absorb);
    update(&mut app, Action::ImageStep(1));
    terminal.draw(|f| carrel::render::draw(f, &app)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect();
    assert!(text.contains("remote images are never fetched"));
}

#[test]
fn viewer_owns_keys_and_survives_a_tiny_resize() {
    assert_eq!(
        Keys::map_lightbox(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE)),
        Some(Action::ImageStep(1))
    );
    assert_eq!(
        Keys::map_lightbox(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)),
        None
    );
    let mut app = app();
    update(&mut app, Action::ImageOpen(None));
    app.on_resize(1, 1);
    let mut terminal = Terminal::new(TestBackend::new(1, 1)).unwrap();
    terminal.draw(|f| carrel::render::draw(f, &app)).unwrap();
    assert_eq!(app.lightbox, Some(BlockIdx(0)));
    update(&mut app, Action::CloseFile);
    assert_eq!(app.lightbox, None);
}

#[test]
fn decoded_image_fits_above_controls_and_resizes() {
    let mut app = app();
    update(&mut app, Action::ImageOpen(Some(BlockIdx(0))));
    let mut picker = ratatui_image::picker::Picker::halfblocks();
    picker.set_protocol_type(ratatui_image::picker::ProtocolType::Halfblocks);
    let pixels = image::DynamicImage::new_rgb8(120, 300);
    let mut images =
        std::collections::HashMap::from([(BlockIdx(0), picker.new_resize_protocol(pixels))]);
    for (width, height) in [(100, 20), (30, 10)] {
        app.on_resize(width, height);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|f| {
                carrel::render::draw_full(
                    f,
                    &app,
                    &mut carrel::render::Painted::default(),
                    &mut images,
                );
            })
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();
        assert!(text.contains("Image 1/2"));
        assert!(text.contains("[close]"));
        assert!(!text.contains("Loading image"));
    }
}
