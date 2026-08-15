//! Semantic scope to terminal style. **The only file with a colour in it.**
//!
//! `carrel-core` emits semantic scopes and never a colour or an ANSI code, so
//! a second frontend maps the same scopes to CSS. Everything below is that
//! map, for the terminal — now as a table of palettes.
//!
//! # The active palette is process-global presentation state
//!
//! An atomic index into [`PALETTES`], set at startup from config and by the
//! theme-cycle key. It never enters `App`: rule 6 keeps ratatui colour types
//! out of the state layer, and the *name* in the config file is just a string.
//!
//! # Third-party palettes
//!
//! Catppuccin, Gruvbox, Tokyo Night, Nord, Dracula, Solarized, Everforest,
//! Rosé Pine, Kanagawa, Synthwave '84, and Oceanic Next are all MIT-licensed
//! palettes, encoded faithfully from each project's canonical definition,
//! with thanks.

// `0xRRGGBB` is the universal notation for a colour; `0x00E0_A044` is not.
#![allow(clippy::unreadable_literal)]

use std::sync::atomic::{AtomicUsize, Ordering};

use ratatui::style::{Color, Modifier, Style};

/// Every colour decision the frontend makes, as one flat struct.
///
/// `bg`/`fg` of `None` mean "inherit the terminal", which is what makes the
/// `terminal` theme a first-class palette rather than a special case.
#[derive(Debug)]
pub struct Palette {
    pub name: &'static str,
    /// A short alias (`dark`, `light`) that also resolves to this palette.
    alias: Option<&'static str>,
    bg: Option<Color>,
    fg: Option<Color>,
    heading_hi: Color,
    heading_lo: Color,
    code_fg: Color,
    code_bg: Color,
    link: Color,
    dim: Color,
    status_fg: Color,
    status_bg: Color,
    match_bg: Color,
    cur_fg: Color,
    cur_bg: Color,
    /// Selected row accent (home list).
    sel: Color,
    /// Selected link: inverted pill.
    lsel_fg: Color,
    lsel_bg: Color,
    wordmark: Color,
    kw: Color,
    string: Color,
    comment: Color,
    number: Color,
    func: Color,
    ty: Color,
    punct: Color,
}

const fn c(hex: u32) -> Color {
    #[allow(clippy::cast_possible_truncation)]
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

// The house colours, from the brand notes.
const LAMP: Color = c(0x7AA874);
const LAMP_DIM: Color = c(0x5C865A);
const AMBER: Color = c(0xE0A044);
const AMBER_DIM: Color = c(0x5A3E18);
const WOOD: Color = c(0x4A3626);
const WOOD_DIM: Color = c(0x7A644A);
const CREAM: Color = c(0xE2D4BA);
const INK: Color = c(0x1C1610);

/// Every palette carrel ships. `terminal` is index 0 and the default.
pub static PALETTES: &[Palette] = &[
    // The accents-on-your-own-terminal default: this IS "match user settings".
    Palette {
        name: "terminal",
        alias: None,
        bg: None,
        fg: None,
        heading_hi: LAMP,
        heading_lo: LAMP_DIM,
        code_fg: CREAM,
        code_bg: WOOD,
        link: AMBER,
        dim: WOOD_DIM,
        status_fg: CREAM,
        status_bg: WOOD,
        match_bg: AMBER_DIM,
        cur_fg: INK,
        cur_bg: AMBER,
        sel: AMBER,
        lsel_fg: INK,
        lsel_bg: LAMP,
        wordmark: LAMP,
        kw: AMBER,
        string: LAMP,
        comment: WOOD_DIM,
        number: LAMP_DIM,
        func: CREAM,
        ty: LAMP_DIM,
        punct: WOOD_DIM,
    },
    // The desk at night.
    Palette {
        name: "carrel-dark",
        alias: Some("dark"),
        bg: Some(INK),
        fg: Some(CREAM),
        heading_hi: LAMP,
        heading_lo: LAMP_DIM,
        code_fg: CREAM,
        code_bg: c(0x2A2118),
        link: AMBER,
        dim: WOOD_DIM,
        status_fg: CREAM,
        status_bg: WOOD,
        match_bg: AMBER_DIM,
        cur_fg: INK,
        cur_bg: AMBER,
        sel: AMBER,
        lsel_fg: INK,
        lsel_bg: LAMP,
        wordmark: LAMP,
        kw: AMBER,
        string: LAMP,
        comment: WOOD_DIM,
        number: LAMP_DIM,
        func: CREAM,
        ty: LAMP_DIM,
        punct: WOOD_DIM,
    },
    // Reading a page: cream paper, ink text.
    Palette {
        name: "carrel-light",
        alias: Some("light"),
        bg: Some(c(0xF5EDD8)),
        fg: Some(c(0x2A2118)),
        heading_hi: c(0x4A7A44),
        heading_lo: c(0x5C865A),
        code_fg: c(0x2A2118),
        code_bg: c(0xE8DCC0),
        link: c(0xA06818),
        dim: c(0x9A8464),
        status_fg: c(0x2A2118),
        status_bg: c(0xE2D4BA),
        match_bg: c(0xECD9A8),
        cur_fg: c(0x2A2118),
        cur_bg: AMBER,
        sel: c(0xA06818),
        lsel_fg: c(0xF5EDD8),
        lsel_bg: c(0x4A7A44),
        wordmark: c(0x4A7A44),
        kw: c(0xA06818),
        string: c(0x4A7A44),
        comment: c(0x9A8464),
        number: c(0x5C865A),
        func: c(0x2A2118),
        ty: c(0x5C865A),
        punct: c(0x9A8464),
    },
    Palette {
        name: "catppuccin-mocha",
        alias: None,
        bg: Some(c(0x1E1E2E)),
        fg: Some(c(0xCDD6F4)),
        heading_hi: c(0xCBA6F7),
        heading_lo: c(0x89B4FA),
        code_fg: c(0xCDD6F4),
        code_bg: c(0x313244),
        link: c(0x74C7EC),
        dim: c(0x6C7086),
        status_fg: c(0xCDD6F4),
        status_bg: c(0x313244),
        match_bg: c(0x45475A),
        cur_fg: c(0x1E1E2E),
        cur_bg: c(0xFAB387),
        sel: c(0xFAB387),
        lsel_fg: c(0x1E1E2E),
        lsel_bg: c(0x94E2D5),
        wordmark: c(0xCBA6F7),
        kw: c(0xCBA6F7),
        string: c(0xA6E3A1),
        comment: c(0x6C7086),
        number: c(0xFAB387),
        func: c(0x89B4FA),
        ty: c(0xF9E2AF),
        punct: c(0x7F849C),
    },
    Palette {
        name: "catppuccin-latte",
        alias: None,
        bg: Some(c(0xEFF1F5)),
        fg: Some(c(0x4C4F69)),
        heading_hi: c(0x8839EF),
        heading_lo: c(0x1E66F5),
        code_fg: c(0x4C4F69),
        code_bg: c(0xCCD0DA),
        link: c(0x209FB5),
        dim: c(0x9CA0B0),
        status_fg: c(0x4C4F69),
        status_bg: c(0xCCD0DA),
        match_bg: c(0xBCC0CC),
        cur_fg: c(0xEFF1F5),
        cur_bg: c(0xFE640B),
        sel: c(0xFE640B),
        lsel_fg: c(0xEFF1F5),
        lsel_bg: c(0x179299),
        wordmark: c(0x8839EF),
        kw: c(0x8839EF),
        string: c(0x40A02B),
        comment: c(0x9CA0B0),
        number: c(0xFE640B),
        func: c(0x1E66F5),
        ty: c(0xDF8E1D),
        punct: c(0x7C7F93),
    },
    Palette {
        name: "gruvbox-dark",
        alias: None,
        bg: Some(c(0x282828)),
        fg: Some(c(0xEBDBB2)),
        heading_hi: c(0xFE8019),
        heading_lo: c(0xFABD2F),
        code_fg: c(0xEBDBB2),
        code_bg: c(0x3C3836),
        link: c(0x83A598),
        dim: c(0x928374),
        status_fg: c(0xEBDBB2),
        status_bg: c(0x3C3836),
        match_bg: c(0x504945),
        cur_fg: c(0x282828),
        cur_bg: c(0xFE8019),
        sel: c(0xFE8019),
        lsel_fg: c(0x282828),
        lsel_bg: c(0x8EC07C),
        wordmark: c(0xFE8019),
        kw: c(0xFB4934),
        string: c(0xB8BB26),
        comment: c(0x928374),
        number: c(0xD3869B),
        func: c(0x8EC07C),
        ty: c(0xFABD2F),
        punct: c(0xA89984),
    },
    Palette {
        name: "gruvbox-light",
        alias: None,
        bg: Some(c(0xFBF1C7)),
        fg: Some(c(0x3C3836)),
        heading_hi: c(0xAF3A03),
        heading_lo: c(0xB57614),
        code_fg: c(0x3C3836),
        code_bg: c(0xEBDBB2),
        link: c(0x076678),
        dim: c(0x928374),
        status_fg: c(0x3C3836),
        status_bg: c(0xEBDBB2),
        match_bg: c(0xD5C4A1),
        cur_fg: c(0xFBF1C7),
        cur_bg: c(0xAF3A03),
        sel: c(0xAF3A03),
        lsel_fg: c(0xFBF1C7),
        lsel_bg: c(0x427B58),
        wordmark: c(0xAF3A03),
        kw: c(0x9D0006),
        string: c(0x79740E),
        comment: c(0x928374),
        number: c(0x8F3F71),
        func: c(0x427B58),
        ty: c(0xB57614),
        punct: c(0x7C6F64),
    },
    Palette {
        name: "tokyo-night",
        alias: None,
        bg: Some(c(0x1A1B26)),
        fg: Some(c(0xC0CAF5)),
        heading_hi: c(0x7AA2F7),
        heading_lo: c(0xBB9AF7),
        code_fg: c(0xC0CAF5),
        code_bg: c(0x24283B),
        link: c(0x7DCFFF),
        dim: c(0x565F89),
        status_fg: c(0xC0CAF5),
        status_bg: c(0x24283B),
        match_bg: c(0x414868),
        cur_fg: c(0x1A1B26),
        cur_bg: c(0xFF9E64),
        sel: c(0xFF9E64),
        lsel_fg: c(0x1A1B26),
        lsel_bg: c(0x73DACA),
        wordmark: c(0x7AA2F7),
        kw: c(0xBB9AF7),
        string: c(0x9ECE6A),
        comment: c(0x565F89),
        number: c(0xFF9E64),
        func: c(0x7AA2F7),
        ty: c(0x2AC3DE),
        punct: c(0xA9B1D6),
    },
    Palette {
        name: "nord",
        alias: None,
        bg: Some(c(0x2E3440)),
        fg: Some(c(0xD8DEE9)),
        heading_hi: c(0x88C0D0),
        heading_lo: c(0x81A1C1),
        code_fg: c(0xD8DEE9),
        code_bg: c(0x3B4252),
        link: c(0x88C0D0),
        dim: c(0x616E88),
        status_fg: c(0xD8DEE9),
        status_bg: c(0x3B4252),
        match_bg: c(0x434C5E),
        cur_fg: c(0x2E3440),
        cur_bg: c(0xEBCB8B),
        sel: c(0xEBCB8B),
        lsel_fg: c(0x2E3440),
        lsel_bg: c(0x88C0D0),
        wordmark: c(0x88C0D0),
        kw: c(0x81A1C1),
        string: c(0xA3BE8C),
        comment: c(0x616E88),
        number: c(0xB48EAD),
        func: c(0x88C0D0),
        ty: c(0x8FBCBB),
        punct: c(0xD8DEE9),
    },
    Palette {
        name: "dracula",
        alias: None,
        bg: Some(c(0x282A36)),
        fg: Some(c(0xF8F8F2)),
        heading_hi: c(0xBD93F9),
        heading_lo: c(0xFF79C6),
        code_fg: c(0xF8F8F2),
        code_bg: c(0x44475A),
        link: c(0x8BE9FD),
        dim: c(0x6272A4),
        status_fg: c(0xF8F8F2),
        status_bg: c(0x44475A),
        match_bg: c(0x44475A),
        cur_fg: c(0x282A36),
        cur_bg: c(0xFFB86C),
        sel: c(0xFFB86C),
        lsel_fg: c(0x282A36),
        lsel_bg: c(0x50FA7B),
        wordmark: c(0xBD93F9),
        kw: c(0xFF79C6),
        string: c(0xF1FA8C),
        comment: c(0x6272A4),
        number: c(0xBD93F9),
        func: c(0x50FA7B),
        ty: c(0x8BE9FD),
        punct: c(0xF8F8F2),
    },
    Palette {
        name: "solarized-dark",
        alias: None,
        bg: Some(c(0x002B36)),
        fg: Some(c(0x839496)),
        heading_hi: c(0x268BD2),
        heading_lo: c(0x6C71C4),
        code_fg: c(0x93A1A1),
        code_bg: c(0x073642),
        link: c(0x268BD2),
        dim: c(0x586E75),
        status_fg: c(0x93A1A1),
        status_bg: c(0x073642),
        match_bg: c(0x073642),
        cur_fg: c(0x002B36),
        cur_bg: c(0xB58900),
        sel: c(0xB58900),
        lsel_fg: c(0x002B36),
        lsel_bg: c(0x2AA198),
        wordmark: c(0x268BD2),
        kw: c(0x859900),
        string: c(0x2AA198),
        comment: c(0x586E75),
        number: c(0xD33682),
        func: c(0x268BD2),
        ty: c(0xB58900),
        punct: c(0x839496),
    },
    Palette {
        name: "solarized-light",
        alias: None,
        bg: Some(c(0xFDF6E3)),
        fg: Some(c(0x657B83)),
        heading_hi: c(0x268BD2),
        heading_lo: c(0x6C71C4),
        code_fg: c(0x586E75),
        code_bg: c(0xEEE8D5),
        link: c(0x268BD2),
        dim: c(0x93A1A1),
        status_fg: c(0x586E75),
        status_bg: c(0xEEE8D5),
        match_bg: c(0xEEE8D5),
        cur_fg: c(0xFDF6E3),
        cur_bg: c(0xB58900),
        sel: c(0xB58900),
        lsel_fg: c(0xFDF6E3),
        lsel_bg: c(0x2AA198),
        wordmark: c(0x268BD2),
        kw: c(0x859900),
        string: c(0x2AA198),
        comment: c(0x93A1A1),
        number: c(0xD33682),
        func: c(0x268BD2),
        ty: c(0xB58900),
        punct: c(0x657B83),
    },
    Palette {
        name: "everforest",
        alias: None,
        bg: Some(c(0x2D353B)),
        fg: Some(c(0xD3C6AA)),
        heading_hi: c(0xA7C080),
        heading_lo: c(0x83C092),
        code_fg: c(0xD3C6AA),
        code_bg: c(0x343F44),
        link: c(0x7FBBB3),
        dim: c(0x859289),
        status_fg: c(0xD3C6AA),
        status_bg: c(0x343F44),
        match_bg: c(0x3D484D),
        cur_fg: c(0x2D353B),
        cur_bg: c(0xDBBC7F),
        sel: c(0xDBBC7F),
        lsel_fg: c(0x2D353B),
        lsel_bg: c(0xA7C080),
        wordmark: c(0xA7C080),
        kw: c(0xE67E80),
        string: c(0xA7C080),
        comment: c(0x859289),
        number: c(0xD699B6),
        func: c(0x83C092),
        ty: c(0xDBBC7F),
        punct: c(0x9DA9A0),
    },
    Palette {
        name: "rose-pine",
        alias: None,
        bg: Some(c(0x191724)),
        fg: Some(c(0xE0DEF4)),
        heading_hi: c(0xC4A7E7),
        heading_lo: c(0x9CCFD8),
        code_fg: c(0xE0DEF4),
        code_bg: c(0x26233A),
        link: c(0x9CCFD8),
        dim: c(0x6E6A86),
        status_fg: c(0xE0DEF4),
        status_bg: c(0x26233A),
        match_bg: c(0x403D52),
        cur_fg: c(0x191724),
        cur_bg: c(0xF6C177),
        sel: c(0xF6C177),
        lsel_fg: c(0x191724),
        lsel_bg: c(0x9CCFD8),
        wordmark: c(0xC4A7E7),
        kw: c(0x31748F),
        string: c(0xF6C177),
        comment: c(0x6E6A86),
        number: c(0xC4A7E7),
        func: c(0xEBBCBA),
        ty: c(0x9CCFD8),
        punct: c(0x908CAA),
    },
    Palette {
        name: "kanagawa",
        alias: None,
        bg: Some(c(0x1F1F28)),
        fg: Some(c(0xDCD7BA)),
        heading_hi: c(0x7E9CD8),
        heading_lo: c(0x957FB8),
        code_fg: c(0xDCD7BA),
        code_bg: c(0x2A2A37),
        link: c(0x7E9CD8),
        dim: c(0x727169),
        status_fg: c(0xDCD7BA),
        status_bg: c(0x2A2A37),
        match_bg: c(0x2D4F67),
        cur_fg: c(0x1F1F28),
        cur_bg: c(0xE6C384),
        sel: c(0xE6C384),
        lsel_fg: c(0x1F1F28),
        lsel_bg: c(0x7AA89F),
        wordmark: c(0x7E9CD8),
        kw: c(0x957FB8),
        string: c(0x98BB6C),
        comment: c(0x727169),
        number: c(0xD27E99),
        func: c(0x7E9CD8),
        ty: c(0x7AA89F),
        punct: c(0x9CABCA),
    },
    // The weird cyber neon one. Glow sold separately.
    Palette {
        name: "synthwave",
        alias: None,
        bg: Some(c(0x262335)),
        fg: Some(c(0xF0EFF1)),
        heading_hi: c(0xFF7EDB),
        heading_lo: c(0x36F9F6),
        code_fg: c(0xF0EFF1),
        code_bg: c(0x2A2139),
        link: c(0x36F9F6),
        dim: c(0x848BBD),
        status_fg: c(0xF0EFF1),
        status_bg: c(0x241B2F),
        match_bg: c(0x34294F),
        cur_fg: c(0x262335),
        cur_bg: c(0xFF7EDB),
        sel: c(0xFF7EDB),
        lsel_fg: c(0x262335),
        lsel_bg: c(0x36F9F6),
        wordmark: c(0xFF7EDB),
        kw: c(0xFEDE5D),
        string: c(0xFF8B39),
        comment: c(0x848BBD),
        number: c(0xF97E72),
        func: c(0x36F9F6),
        ty: c(0xFF7EDB),
        punct: c(0xB6B1B1),
    },
    // The ocean one: Oceanic Next's deep-sea blues.
    Palette {
        name: "oceanic",
        alias: None,
        bg: Some(c(0x1B2B34)),
        fg: Some(c(0xD8DEE9)),
        heading_hi: c(0x6699CC),
        heading_lo: c(0x5FB3B3),
        code_fg: c(0xC0C5CE),
        code_bg: c(0x343D46),
        link: c(0x5FB3B3),
        dim: c(0x65737E),
        status_fg: c(0xC0C5CE),
        status_bg: c(0x343D46),
        match_bg: c(0x4F5B66),
        cur_fg: c(0x1B2B34),
        cur_bg: c(0xFAC863),
        sel: c(0xFAC863),
        lsel_fg: c(0x1B2B34),
        lsel_bg: c(0x5FB3B3),
        wordmark: c(0x6699CC),
        kw: c(0xC594C5),
        string: c(0x99C794),
        comment: c(0x65737E),
        number: c(0xF99157),
        func: c(0x6699CC),
        ty: c(0xFAC863),
        punct: c(0xC0C5CE),
    },
];

static ACTIVE: AtomicUsize = AtomicUsize::new(0);

/// `NO_COLOR` (no-color.org): colours off, weight and emphasis kept. Set
/// once at startup by the binary; process-global like the active palette.
static MONO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_mono(on: bool) {
    MONO.store(on, Ordering::Relaxed);
}

/// The monochrome filter every style passes through. A style whose only
/// signal was its background (matches, selection, the status bar) keeps a
/// visible signal by turning REVERSED instead of vanishing.
fn tinted(s: Style) -> Style {
    if !MONO.load(Ordering::Relaxed) {
        return s;
    }
    let had_bg = s.bg.is_some();
    let mut m = Style {
        fg: None,
        bg: None,
        ..s
    };
    if had_bg {
        m = m.add_modifier(Modifier::REVERSED);
    }
    m
}

fn active() -> &'static Palette {
    &PALETTES[ACTIVE.load(Ordering::Relaxed).min(PALETTES.len() - 1)]
}

/// Select a theme by name or alias. `false` if no such theme exists.
pub fn set_theme(name: &str) -> bool {
    let Some(i) = PALETTES
        .iter()
        .position(|p| p.name == name || p.alias == Some(name))
    else {
        return false;
    };
    ACTIVE.store(i, Ordering::Relaxed);
    true
}

/// Advance to the next theme, returning its name.
pub fn cycle_theme() -> &'static str {
    let next = (ACTIVE.load(Ordering::Relaxed) + 1) % PALETTES.len();
    ACTIVE.store(next, Ordering::Relaxed);
    PALETTES[next].name
}

#[must_use]
pub fn current_name() -> &'static str {
    active().name
}

/// The page itself: `None`s inherit the terminal (`terminal` theme).
#[must_use]
pub fn body() -> Style {
    let p = active();
    let mut s = Style::default();
    if let Some(fg) = p.fg {
        s = s.fg(fg);
    }
    if let Some(bg) = p.bg {
        s = s.bg(bg);
    }
    tinted(s)
}

/// Inline runs: emphasis, strong, code, strikethrough, links.
#[must_use]
pub fn inline(scope: carrel_core::Style) -> Style {
    let p = active();
    let mut s = Style::default();
    if scope.contains(carrel_core::Style::STRONG) {
        s = s.add_modifier(Modifier::BOLD);
    }
    if scope.contains(carrel_core::Style::EMPHASIS) {
        s = s.add_modifier(Modifier::ITALIC);
    }
    if scope.contains(carrel_core::Style::STRIKETHROUGH) {
        s = s.add_modifier(Modifier::CROSSED_OUT);
    }
    if scope.contains(carrel_core::Style::CODE) {
        s = s.fg(p.code_fg).bg(p.code_bg);
    }
    if scope.contains(carrel_core::Style::LINK) {
        s = s.fg(p.link).add_modifier(Modifier::UNDERLINED);
    }
    tinted(s)
}

#[must_use]
pub fn heading(level: u8) -> Style {
    let p = active();
    let c = if level <= 2 {
        p.heading_hi
    } else {
        p.heading_lo
    };
    tinted(Style::default().fg(c).add_modifier(Modifier::BOLD))
}

/// List bullets and ordered numbers.
#[must_use]
pub fn marker() -> Style {
    tinted(Style::default().fg(active().dim))
}

#[must_use]
pub fn quote_bar() -> Style {
    tinted(Style::default().fg(active().dim))
}

#[must_use]
pub fn status() -> Style {
    let p = active();
    tinted(Style::default().fg(p.status_fg).bg(p.status_bg))
}

#[must_use]
pub fn match_normal() -> Style {
    tinted(Style::default().bg(active().match_bg))
}

/// The lamp. the brand notes: search "stays on when everything else moves".
#[must_use]
pub fn match_current() -> Style {
    let p = active();
    tinted(Style::default().fg(p.cur_fg).bg(p.cur_bg))
}

/// Semantic syntax tokens, muted. `Plain` keeps the code block's own colours.
#[must_use]
pub fn token(kind: carrel_core::TokenKind) -> Style {
    use carrel_core::TokenKind as K;
    let p = active();
    let code = Style::default().fg(p.code_fg).bg(p.code_bg);
    tinted(match kind {
        K::Keyword => code.fg(p.kw),
        K::String => code.fg(p.string),
        K::Comment => code.fg(p.comment).add_modifier(Modifier::ITALIC),
        K::Number => code.fg(p.number),
        K::Function => code.fg(p.func).add_modifier(Modifier::BOLD),
        K::Type => code.fg(p.ty),
        K::Punctuation => code.fg(p.punct),
        K::Plain => code,
    })
}

/// The home-screen wordmark.
#[must_use]
pub fn wordmark() -> Style {
    tinted(
        Style::default()
            .fg(active().wordmark)
            .add_modifier(Modifier::BOLD),
    )
}

/// GFM alert accents, mapped from existing palette slots so all 17 themes
/// colour them without new fields: Note leans on the link blue, Tip on the
/// string green, Important on the bright heading, Warning on the amber
/// accent, Caution on the keyword (red in the palettes that have one).
#[must_use]
pub fn alert(kind: carrel_core::AlertKind) -> Style {
    use carrel_core::AlertKind as K;
    let p = active();
    let c = match kind {
        K::Note => p.link,
        K::Tip => p.string,
        K::Important => p.heading_hi,
        K::Warning => p.sel,
        K::Caution => p.kw,
    };
    tinted(Style::default().fg(c))
}

/// The lamp itself — the splash's bulb and light pool. The amber accent
/// without `selected()`'s bold, so ░ reads as glow rather than paint.
#[must_use]
pub fn lamp() -> Style {
    tinted(Style::default().fg(active().sel))
}

/// The tagline and other quiet secondary text.
#[must_use]
pub fn dim() -> Style {
    tinted(Style::default().fg(active().dim))
}

/// A frontmatter key. Bold against the value, but still dim against the
/// body — metadata is context, not content.
#[must_use]
pub fn meta_key() -> Style {
    tinted(
        Style::default()
            .fg(active().dim)
            .add_modifier(Modifier::BOLD),
    )
}

/// A frontmatter value.
#[must_use]
pub fn meta_value() -> Style {
    tinted(Style::default().fg(active().dim))
}

/// The selected row in a list.
#[must_use]
pub fn selected() -> Style {
    tinted(
        Style::default()
            .fg(active().sel)
            .add_modifier(Modifier::BOLD),
    )
}

/// The link `Tab` has selected — the lamp pointed at it.
#[must_use]
pub fn link_selected() -> Style {
    let p = active();
    tinted(Style::default().fg(p.lsel_fg).bg(p.lsel_bg))
}

/// The mouse selection. `REVERSED` rather than a palette colour: it reads as
/// a selection in all 17 palettes and survives `NO_COLOR` untouched — the
/// same choice terminals themselves make.
#[must_use]
pub fn selection() -> Style {
    Style::default().add_modifier(Modifier::REVERSED)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Theme *switching* is deliberately not tested here: the active palette
    // is process-global, and the unit-test binary runs its tests in parallel
    // threads that all read it. Switching lives in `tests/theme_switching.rs`,
    // a separate integration binary — its own process. Tests here (and in
    // render.rs) only ever read the default.

    #[test]
    fn every_theme_name_is_unique() {
        let mut names: Vec<&str> = PALETTES.iter().map(|p| p.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), PALETTES.len(), "duplicate theme name");
    }

    #[test]
    fn strong_is_bold_and_emphasis_is_italic() {
        assert!(
            inline(carrel_core::Style::STRONG)
                .add_modifier
                .contains(Modifier::BOLD)
        );
        assert!(
            inline(carrel_core::Style::EMPHASIS)
                .add_modifier
                .contains(Modifier::ITALIC)
        );
    }

    #[test]
    fn combined_scopes_combine_styles() {
        let s = inline(carrel_core::Style(
            carrel_core::Style::STRONG.0 | carrel_core::Style::EMPHASIS.0,
        ));
        assert!(s.add_modifier.contains(Modifier::BOLD));
        assert!(s.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn body_text_imposes_no_colour_so_the_terminal_theme_wins() {
        let s = inline(carrel_core::Style::NONE);
        assert_eq!(s.fg, None);
        assert_eq!(s.bg, None);
    }

    #[test]
    fn the_current_match_is_visually_distinct_from_an_ordinary_one() {
        assert_ne!(match_current(), match_normal());
    }
}
