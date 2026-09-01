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
    let mut app = App::new(
        "t.md".into(),
        Document::parse("# Title\n\nbody text\n"),
        cols,
        rows,
    );
    // Classic geometry: this test reads fixed cells; the breadcrumb band
    // has its own tests.
    app.breadcrumb = false;
    app.on_resize(cols, rows);
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

    // --- The desktop palette ------------------------------------------------
    // Last, because installing it lengthens the cycle lap the assertions
    // above measure.

    // Not on offer until the desktop has published one: a config carried over
    // from an Omarchy machine must fail the same way any stale name does.
    assert!(!theme::set_theme(theme::OMARCHY), "nothing to follow yet");

    let desktop = carrel::omarchy::parse(AETHER).expect("aether parses");
    assert!(
        theme::install_omarchy(&desktop),
        "installing one is a change"
    );
    assert!(
        !theme::install_omarchy(&desktop),
        "installing the same one again is not — this is what stops the \
         once-a-second poll repainting the screen forever"
    );

    assert!(theme::set_theme(theme::OMARCHY));
    assert_eq!(theme::current_name(), "omarchy");
    assert_eq!(theme::body().bg, Some(Color::Rgb(0x0e, 0x09, 0x1d)));
    assert_eq!(theme::body().fg, Some(Color::Rgb(0xdc, 0x8f, 0x7c)));
    // The point of the whole exercise: headings wear the desktop's accent,
    // not carrel's house green.
    assert_eq!(theme::heading(1).fg, Some(Color::Rgb(0x6e, 0x60, 0x80)));

    let buf = frame(30, 8);
    assert_eq!(
        buf[(0, 0)].bg,
        Color::Rgb(0x0e, 0x09, 0x1d),
        "the desktop's page colour reaches real cells"
    );

    // And it takes its place at the end of the rotation, so `T` still walks
    // every theme and still comes home to `terminal`.
    assert!(theme::set_theme("terminal"));
    let lap: Vec<&str> = (0..PALETTES.len()).map(|_| theme::cycle_theme()).collect();
    assert_eq!(
        lap.last().copied(),
        Some(theme::OMARCHY),
        "the desktop palette closes the lap"
    );
    assert_eq!(theme::cycle_theme(), "terminal", "and wraps home");
}

/// The palette Omarchy's `aether` theme publishes, verbatim.
const AETHER: &str = r##"
accent = "#6e6080"
foreground = "#dc8f7c"
background = "#0e091d"
selection_background = "#6e6080"
color0 = "#0e091d"
color1 = "#c53253"
color2 = "#a68e5a"
color3 = "#ff6565"
color4 = "#6e6080"
color5 = "#a45782"
color6 = "#8c9785"
"##;
