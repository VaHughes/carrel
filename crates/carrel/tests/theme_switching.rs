//! Theme *switching* tests, quarantined in their own binary.
//!
//! The active palette is a process-global atomic. The unit-test binary runs
//! its tests in parallel threads that all read it, so a unit test that
//! switched themes would tear other tests' reads (paint under one palette,
//! assert under another). An integration binary is a separate process — the
//! only place switching can be exercised safely. Keep every switching
//! assertion inside this ONE test function so additions never race each other.

use carrel::app::App;
use carrel::theme::{self, PALETTES};
use carrel_core::Document;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Color;

fn frame(cols: u16, rows: u16) -> Buffer {
    let app = App::new(
        "t.md".into(),
        Document::parse("# Title\n\nbody text\n"),
        cols,
        rows,
    );
    let mut t = Terminal::new(TestBackend::new(cols, rows)).unwrap();
    t.draw(|f| carrel::render::draw(f, &app)).unwrap();
    t.backend().buffer().clone()
}

#[test]
fn switching() {
    // Aliases resolve; unknown names are refused and change nothing.
    assert!(theme::set_theme("dark") && theme::current_name() == "carrel-dark");
    assert!(theme::set_theme("light") && theme::current_name() == "carrel-light");
    assert!(!theme::set_theme("no-such-theme"));
    assert_eq!(theme::current_name(), "carrel-light");

    // A named theme paints a real page; `terminal` inherits.
    assert!(theme::set_theme("dracula"));
    assert_eq!(theme::body().bg, Some(Color::Rgb(0x28, 0x2A, 0x36)));
    assert_eq!(theme::heading(1).fg, Some(Color::Rgb(0xBD, 0x93, 0xF9)));
    assert!(theme::set_theme("terminal"));
    assert_eq!(theme::body().bg, None, "terminal inherits the background");

    // Cycling visits every theme exactly once per lap, then wraps.
    let mut seen = vec![theme::current_name()];
    for _ in 1..PALETTES.len() {
        seen.push(theme::cycle_theme());
    }
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), PALETTES.len(), "cycle must visit all themes");
    assert_eq!(theme::cycle_theme(), "terminal", "and wrap back around");

    // And the pixels agree: the page colour lands in real cells.
    assert!(theme::set_theme("dracula"));
    let buf = frame(30, 8);
    assert_eq!(
        buf[(0, 0)].bg,
        Color::Rgb(0x28, 0x2A, 0x36),
        "the page paints its own bg out to the margins"
    );
    assert_eq!(
        buf[(carrel::app::PAD_LEFT, carrel::app::PAD_TOP)].fg,
        Color::Rgb(0xBD, 0x93, 0xF9),
        "H1 in dracula purple"
    );
    assert!(theme::set_theme("terminal"));
    let buf = frame(30, 8);
    assert_eq!(buf[(0, 3)].bg, Color::Reset, "terminal inherits the bg");

    // NO_COLOR monochrome (process-global, so it lives in this test too):
    // colours vanish, weight survives, and bg-only styles turn REVERSED so
    // matches and the status bar keep a visible signal.
    assert!(theme::set_theme("dracula"));
    theme::set_mono(true);
    assert_eq!(theme::heading(1).fg, None, "no colour under NO_COLOR");
    assert!(
        theme::heading(1)
            .add_modifier
            .contains(ratatui::style::Modifier::BOLD),
        "weight survives"
    );
    assert_eq!(theme::match_current().bg, None);
    assert!(
        theme::match_current()
            .add_modifier
            .contains(ratatui::style::Modifier::REVERSED),
        "bg-only styles reverse instead of vanishing"
    );
    theme::set_mono(false);
    assert!(theme::set_theme("terminal"));
}
