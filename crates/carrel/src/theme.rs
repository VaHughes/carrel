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
//! # The desktop palette
//!
//! One palette is not in that table: [`OMARCHY`] is derived at runtime from
//! the colours the desktop publishes ([`crate::omarchy`]) and can be replaced
//! while the reader is up, so [`ACTIVE`] holding [`OMARCHY_SLOT`] means "look
//! in [`DESKTOP`] instead". Everything else about it — how it is named, how it
//! cycles, how it is persisted — is the same as for any built-in.
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
///
/// `Copy` on purpose: the desktop palette ([`install_omarchy`]) is built at
/// runtime and can be replaced while the reader is up, so [`active`] hands
/// back a value rather than a `&'static` borrowed from a table that may be
/// about to change. ~120 bytes off a hot L1 line, against a `Box::leak` per
/// theme change — measured at noise either way, and this one does not leak.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    /// A diff line that adds, and one that removes. Chosen per theme from
    /// that theme's own published palette — `string`/`kw` are not reliable
    /// green/red proxies (dracula's string is yellow, solarized's kw olive).
    ins: Color,
    /// `del_` because `del` is close enough to a keyword to read badly.
    del_: Color,
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
        ins: LAMP,
        del_: c(0xC0604E),
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
        ins: LAMP,
        del_: c(0xC0604E),
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
        ins: c(0x4A7A44),
        del_: c(0x9A3B2E),
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
        ins: c(0xA6E3A1),
        del_: c(0xF38BA8),
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
        ins: c(0x40A02B),
        del_: c(0xD20F39),
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
        ins: c(0xB8BB26),
        del_: c(0xFB4934),
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
        ins: c(0x79740E),
        del_: c(0x9D0006),
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
        ins: c(0x9ECE6A),
        del_: c(0xF7768E),
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
        ins: c(0xA3BE8C),
        del_: c(0xBF616A),
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
        ins: c(0x50FA7B),
        del_: c(0xFF5555),
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
        ins: c(0x859900),
        del_: c(0xDC322F),
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
        ins: c(0x859900),
        del_: c(0xDC322F),
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
        ins: c(0xA7C080),
        del_: c(0xE67E80),
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
        ins: c(0x9CCFD8),
        del_: c(0xEB6F92),
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
        ins: c(0x98BB6C),
        del_: c(0xC34043),
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
        ins: c(0x72F1B8),
        del_: c(0xFE4450),
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
        ins: c(0x99C794),
        del_: c(0xEC5F67),
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

/// The name the desktop palette answers to, in config and on `T`.
pub const OMARCHY: &str = "omarchy";

/// [`ACTIVE`] holding this means "the desktop palette", which lives in
/// [`DESKTOP`] rather than in [`PALETTES`]. A sentinel rather than an index
/// because the built-in table is a compile-time constant and this one is not.
const OMARCHY_SLOT: usize = usize::MAX;

/// The palette read from the desktop, once [`install_omarchy`] has been given
/// one. Written at most once a second by the event loop's poll and read once
/// per style; an uncontended `RwLock` read is two atomics, and only the
/// desktop theme pays even that.
static DESKTOP: std::sync::RwLock<Option<Palette>> = std::sync::RwLock::new(None);

fn desktop() -> Option<Palette> {
    *DESKTOP.read().ok()?
}

/// Is there a desktop palette to switch to?
#[must_use]
pub fn omarchy_available() -> bool {
    desktop().is_some()
}

fn active() -> Palette {
    let i = ACTIVE.load(Ordering::Relaxed);
    if i == OMARCHY_SLOT {
        // Selected but not installed — a config naming `omarchy` on a machine
        // where Omarchy is gone. The terminal's own colours are the right
        // answer to "match my desktop" when the desktop stops saying.
        return desktop().unwrap_or(PALETTES[0]);
    }
    PALETTES[i.min(PALETTES.len() - 1)]
}

/// Select a theme by name or alias. `false` if no such theme exists.
pub fn set_theme(name: &str) -> bool {
    if name == OMARCHY {
        // Only offered when the desktop actually published a palette, so a
        // stale `theme omarchy` gets the same honest "unknown theme" note as
        // any other name that no longer resolves.
        if !omarchy_available() {
            return false;
        }
        ACTIVE.store(OMARCHY_SLOT, Ordering::Relaxed);
        return true;
    }
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
///
/// The desktop palette rides at the end of the rotation when there is one, so
/// `T` still walks every theme and comes home to `terminal`.
pub fn cycle_theme() -> &'static str {
    let i = ACTIVE.load(Ordering::Relaxed);
    let next = if i == OMARCHY_SLOT {
        0
    } else if i + 1 >= PALETTES.len() && omarchy_available() {
        OMARCHY_SLOT
    } else {
        (i + 1) % PALETTES.len()
    };
    ACTIVE.store(next, Ordering::Relaxed);
    current_name()
}

#[must_use]
pub fn current_name() -> &'static str {
    if ACTIVE.load(Ordering::Relaxed) == OMARCHY_SLOT {
        return OMARCHY;
    }
    active().name
}

/// Install (or replace) the palette derived from the desktop's own colours.
///
/// Returns `true` when the installed palette actually moved, which is the
/// event loop's cue that a repaint is worth doing — `omarchy theme set` to
/// the theme already showing must not flicker the screen once a second.
pub fn install_omarchy(c: &crate::omarchy::Colors) -> bool {
    let next = from_desktop(c);
    let Ok(mut slot) = DESKTOP.write() else {
        return false;
    };
    if *slot == Some(next) {
        return false;
    }
    *slot = Some(next);
    true
}

/// Blend `pct` per cent of `b` into `a`, per channel.
fn mix(a: u32, b: u32, pct: u32) -> u32 {
    let ch = |shift: u32| {
        let x = ((a >> shift) & 0xff) * (100 - pct) + ((b >> shift) & 0xff) * pct;
        (x / 100) & 0xff
    };
    (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

/// Perceived brightness, 0-255. The same weights omawrite uses to decide a
/// theme's polarity, and the ones every "is this dark?" check has used since
/// CCIR 601.
fn luma(hex: u32) -> u32 {
    (299 * ((hex >> 16) & 0xff) + 587 * ((hex >> 8) & 0xff) + 114 * (hex & 0xff)) / 1000
}

/// The first candidate that will actually be legible against `bg`, else
/// `fallback`. A theme whose blue is nearly its background — and there are
/// several — must not end up with invisible links.
fn legible(candidates: &[u32], bg: u32, fallback: u32) -> u32 {
    let floor = luma(bg);
    *candidates
        .iter()
        .find(|&&c| luma(c).abs_diff(floor) >= 48)
        .unwrap_or(&fallback)
}

/// Map the desktop's terminal palette onto carrel's semantic slots.
///
/// Everything derived is derived by blending **toward the page** or **toward
/// the ink** rather than toward black or white, so a light Omarchy theme and
/// a dark one both come out right without a branch on polarity.
fn from_desktop(d: &crate::omarchy::Colors) -> Palette {
    let (bg, fg, accent) = (d.background, d.foreground, d.accent);
    // A panel raised just off the page, and text sunk just into it.
    let panel = mix(bg, fg, 10);
    let muted = mix(fg, bg, 45);
    let (red, green, yellow) = (d.ansi[1], d.ansi[2], d.ansi[3]);
    let (blue, magenta, cyan) = (d.ansi[4], d.ansi[5], d.ansi[6]);

    // Links are blue by five decades of convention; the accent is the theme's
    // signature and belongs on the headings. Both guarded, because a theme is
    // free to put either one almost on top of its background.
    let link = legible(&[blue, cyan, accent], bg, fg);
    let head = legible(&[accent, magenta, blue], bg, fg);
    // The lamp: search's current match, which must read at a glance.
    let lamp = legible(&[yellow, accent], bg, fg);

    Palette {
        name: OMARCHY,
        alias: None,
        bg: Some(c(bg)),
        fg: Some(c(fg)),
        heading_hi: c(head),
        heading_lo: c(mix(head, bg, 35)),
        code_fg: c(fg),
        code_bg: c(panel),
        link: c(link),
        dim: c(muted),
        status_fg: c(fg),
        status_bg: c(mix(bg, fg, 16)),
        // The desktop names its own selection colour, and a search hit is a
        // selection: borrowing it makes carrel's highlight look like every
        // other highlight on the screen.
        match_bg: c(legible(&[d.selection], bg, mix(lamp, bg, 60))),
        cur_fg: c(bg),
        cur_bg: c(lamp),
        sel: c(head),
        lsel_fg: c(bg),
        lsel_bg: c(link),
        wordmark: c(head),
        kw: c(legible(&[magenta, red], bg, fg)),
        string: c(legible(&[green], bg, fg)),
        comment: c(muted),
        number: c(legible(&[yellow], bg, fg)),
        func: c(legible(&[blue, cyan], bg, fg)),
        ty: c(legible(&[cyan, blue], bg, fg)),
        punct: c(muted),
        ins: c(legible(&[green], bg, fg)),
        del_: c(legible(&[red], bg, fg)),
    }
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
    // No terminal has real raised or lowered text, and the reflow layer cannot
    // give a run a different baseline. DIM is the one signal that survives
    // everywhere and still reads as "this is not the main line".
    if scope.contains(carrel_core::Style::SUPERSCRIPT)
        || scope.contains(carrel_core::Style::SUBSCRIPT)
    {
        s = s.add_modifier(Modifier::DIM);
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
        // A diff line reads as a whole line, so it takes weight as well as
        // hue — on a low-contrast theme the green and the red alone are not
        // enough to tell them apart at a glance.
        K::Inserted => code.fg(p.ins).add_modifier(Modifier::BOLD),
        K::Deleted => code.fg(p.del_),
        K::Meta => code.fg(p.comment).add_modifier(Modifier::BOLD),
        // `Plain`, and anything a future `TokenKind` adds: the block's own
        // colours. The enum is `#[non_exhaustive]`, so an upstream addition
        // must degrade rather than fail the build.
        K::Plain | _ => code,
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

/// GFM alert accents, mapped from existing palette slots so every theme —
/// the desktop's included — colours them without new fields: Note leans on the link blue, Tip on the
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

/// The thing under the pointer.
///
/// **Modifiers only, never a colour.** A palette-derived background looked
/// right and was not: **14 of the 17 palettes give code blocks and the
/// status bar the same background**, so a tint borrowed from one would have
/// been invisible on every pane row, the help sheet and the outline in most
/// themes — and obviously fine in the one the tests happen to run in.
/// Modifiers cannot be defeated by a palette, need no new entry in
/// seventeen tables, and are already right under `NO_COLOR` and in
/// monochrome without going through [`tinted`] at all.
///
/// **Both** bold and underline, because each alone collides with something
/// already on the cell: a link is underlined by [`inline`], and a selected
/// row is bolded by [`selected`]. Together, every surface a pointer can land
/// on gains at least one of the two.
///
/// (The design said "underline the link, highlight the row". One signal for
/// every clickable surface is both easier to explain and the only version
/// that survives contact with the palettes.)
#[must_use]
pub fn hover() -> Style {
    Style::default().add_modifier(Modifier::BOLD.union(Modifier::UNDERLINED))
}

/// The mouse selection. `REVERSED` rather than a palette colour: it reads as
/// a selection in every palette — including one derived from a desktop theme
/// nobody has seen — and survives `NO_COLOR` untouched. The same choice
/// terminals themselves make.
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

    // Deriving a desktop palette reads no global state, so it is safe here.
    // Only *switching* to it has to live in the integration binary.

    fn desktop(text: &str) -> crate::omarchy::Colors {
        crate::omarchy::parse(text).expect("test palette parses")
    }

    #[test]
    fn mixing_moves_toward_the_other_colour_from_either_end() {
        assert_eq!(mix(0x000000, 0xffffff, 0), 0x000000);
        assert_eq!(mix(0x000000, 0xffffff, 100), 0xffffff);
        assert_eq!(mix(0x000000, 0xffffff, 50), 0x7f7f7f);
        // The reason every tint blends toward the page or the ink rather than
        // toward black or white: one call, and a light desktop theme comes
        // out light while a dark one comes out dark.
        assert!(luma(mix(0xffffff, 0x222324, 10)) < luma(0xffffff));
        assert!(luma(mix(0x101010, 0xeeeeee, 10)) > luma(0x101010));
    }

    #[test]
    fn a_desktop_palette_never_hides_a_link_in_its_own_page() {
        // A theme whose "blue" is all but its background. Real palettes get
        // close to this, and an unguarded mapping would paint links invisible.
        let mut d = desktop(
            "background = \"#101010\"\n\
             foreground = \"#eeeeee\"\n\
             accent = \"#c08040\"\n\
             color6 = \"#7fd0d0\"\n",
        );
        d.ansi[4] = 0x131313;
        let p = from_desktop(&d);
        assert_ne!(p.link, c(0x131313), "an invisible link is not a link");
        assert_eq!(p.link, c(0x7fd0d0), "it falls through to the cyan slot");
    }

    #[test]
    fn a_desktop_palette_takes_its_accent_and_its_page() {
        let d = desktop(
            "background = \"#0e091d\"\n\
             foreground = \"#dc8f7c\"\n\
             accent = \"#6e6080\"\n\
             selection_background = \"#6e6080\"\n\
             color2 = \"#a68e5a\"\n\
             color1 = \"#c53253\"\n",
        );
        let p = from_desktop(&d);
        assert_eq!(p.bg, Some(c(0x0e091d)));
        assert_eq!(p.fg, Some(c(0xdc8f7c)));
        assert_eq!(p.heading_hi, c(0x6e6080), "headings wear the accent");
        assert_eq!(p.wordmark, c(0x6e6080));
        assert_eq!(p.ins, c(0xa68e5a), "a diff's additions take the green slot");
        assert_eq!(p.del_, c(0xc53253), "and its removals the red one");
        assert_eq!(p.name, OMARCHY);
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
