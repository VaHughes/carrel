//! The desktop's palette, as plain numbers.
//!
//! Omarchy publishes the theme the whole desktop is wearing as a **terminal**
//! palette at `~/.local/state/omarchy/current/theme/colors.toml` — the same
//! file its alacritty, ghostty, btop and helix themes are generated from.
//! Reading it is what lets carrel match the desktop instead of imposing its
//! own accents on top of it.
//!
//! # Why this module holds no colours
//!
//! [`crate::theme`] is "the only file with a colour in it" and stays that way:
//! this module parses hex **numbers** and knows nothing about ratatui, styles,
//! or what any of them are for. `theme` turns them into a [`crate::theme::Palette`].
//! That keeps rule 6 intact — a GTK frontend reuses this parser verbatim and
//! maps the same numbers to CSS.
//!
//! # Why not a TOML parser
//!
//! Same reason `config` hand-rolls its own: `toml` + `serde` is ~300 KB for a
//! file of `key = "#rrggbb"` lines. The subset below is the whole grammar
//! Omarchy actually emits.

// Same reason `theme` allows it: `0xRRGGBB` is the universal notation for a
// colour; `0x000e_091d` is not.
#![allow(clippy::unreadable_literal)]

use std::path::PathBuf;

/// The desktop palette, as `0xRRGGBB`.
///
/// `ansi` is the conventional 16: 0 black, 1 red, 2 green, 3 yellow, 4 blue,
/// 5 magenta, 6 cyan, 7 white, 8-15 the bright half. Theme authors follow
/// that convention even when their hues are unusual, which is what makes it
/// safe to map `ansi[2]` to "the green one" without knowing the theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Colors {
    pub background: u32,
    pub foreground: u32,
    pub accent: u32,
    pub selection: u32,
    pub ansi: [u32; 16],
}

/// `$XDG_STATE_HOME/omarchy`, else `~/.local/state/omarchy` — the same
/// resolution order [`crate::state`] uses for carrel's own state.
#[must_use]
pub fn state_dir() -> Option<PathBuf> {
    if let Some(x) = std::env::var_os("XDG_STATE_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(x).join("omarchy"));
    }
    std::env::var_os("HOME").filter(|v| !v.is_empty()).map(|h| {
        PathBuf::from(h)
            .join(".local")
            .join("state")
            .join("omarchy")
    })
}

/// The active theme's palette file. `current` is a symlink Omarchy swaps, so
/// this path stays valid across `omarchy theme set` and simply points at
/// different bytes afterwards.
#[must_use]
pub fn path() -> Option<PathBuf> {
    Some(
        state_dir()?
            .join("current")
            .join("theme")
            .join("colors.toml"),
    )
}

/// Read and parse the active palette. `None` when Omarchy is not installed,
/// the file is unreadable, or it does not carry the two colours that cannot
/// be synthesised from anything else.
#[must_use]
pub fn load() -> Option<Colors> {
    parse(&std::fs::read_to_string(path()?).ok()?)
}

/// `#rrggbb`, `rrggbb`, or either in quotes, to `0xRRGGBB`.
fn hex(value: &str) -> Option<u32> {
    let v = value.trim();
    let v = v
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .or_else(|| v.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
        .unwrap_or(v)
        .trim();
    let v = v.strip_prefix('#').unwrap_or(v);
    (v.len() == 6).then(|| u32::from_str_radix(v, 16).ok())?
}

/// Parse the `key = value` lines Omarchy writes. Comments, blank lines,
/// `[table]` headers and keys we do not use are skipped rather than refused:
/// a future Omarchy that adds a key must not stop carrel from reading the
/// ones it already understands.
#[must_use]
pub fn parse(text: &str) -> Option<Colors> {
    let mut background = None;
    let mut foreground = None;
    let mut accent = None;
    let mut selection = None;
    let mut ansi: [Option<u32>; 16] = [None; 16];

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let Some(c) = hex(value) else { continue };
        match key.trim() {
            "background" => background = Some(c),
            "foreground" => foreground = Some(c),
            "accent" => accent = Some(c),
            "selection_background" => selection = Some(c),
            k => {
                if let Some(n) = k.strip_prefix("color")
                    && let Ok(i) = n.parse::<usize>()
                    && i < 16
                {
                    ansi[i] = Some(c);
                }
            }
        }
    }

    // Background and foreground are the two the page cannot be drawn without.
    // Everything else has a defensible fallback, so a sparse file degrades to
    // a duller theme rather than to nothing at all.
    let background = background?;
    let foreground = foreground?;
    let accent = accent.or(ansi[4]).unwrap_or(foreground);
    Some(Colors {
        background,
        foreground,
        accent,
        selection: selection.unwrap_or(accent),
        ansi: std::array::from_fn(|i| {
            ansi[i].unwrap_or(if i == 0 { background } else { foreground })
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // `r##` because the palette is full of `"#` — the sequence that would end
    // an `r#` string.
    const AETHER: &str = r##"
accent = "#6e6080"
cursor = "#dc8f7c"
foreground = "#dc8f7c"
background = "#0e091d"
selection_foreground = "#dc8f7c"
selection_background = "#6e6080"

color0 = "#0e091d"
color1 = "#c53253"
color2 = "#a68e5a"
color4 = "#6e6080"
color15 = "#dc8f7c"
"##;

    #[test]
    fn reads_a_real_omarchy_palette() {
        let c = parse(AETHER).expect("aether parses");
        assert_eq!(c.background, 0x0e091d);
        assert_eq!(c.foreground, 0xdc8f7c);
        assert_eq!(c.accent, 0x6e6080);
        assert_eq!(c.selection, 0x6e6080);
        assert_eq!(c.ansi[1], 0xc53253);
        assert_eq!(c.ansi[4], 0x6e6080);
        assert_eq!(c.ansi[15], 0xdc8f7c);
    }

    #[test]
    fn a_missing_ansi_slot_falls_back_rather_than_failing() {
        let c = parse(AETHER).unwrap();
        // color3 was absent: it becomes the foreground, which is dull but
        // always legible — never an invisible colour.
        assert_eq!(c.ansi[3], c.foreground);
    }

    #[test]
    fn accent_falls_back_to_the_blue_slot_then_the_foreground() {
        let c = parse("background = \"#000000\"\nforeground = \"#ffffff\"\ncolor4 = \"#0000ff\"")
            .unwrap();
        assert_eq!(c.accent, 0x0000ff, "the blue slot stands in for accent");
        let c = parse("background = \"#000000\"\nforeground = \"#ffffff\"").unwrap();
        assert_eq!(c.accent, 0xffffff, "and the foreground stands in for both");
    }

    #[test]
    fn a_file_without_a_page_colour_is_not_a_theme() {
        assert!(parse("accent = \"#6e6080\"").is_none());
        assert!(parse("background = \"#0e091d\"").is_none(), "no foreground");
        assert!(parse("").is_none());
    }

    #[test]
    fn tolerates_the_shapes_a_hand_edited_file_takes() {
        let c = parse(
            "# a comment\n\
             [colors]\n\
             background = '#0e091d'\n\
             foreground = #dc8f7c\n\
             nonsense\n\
             opacity = 0.9\n\
             color99 = \"#ffffff\"\n",
        )
        .expect("the two colours that matter are there");
        assert_eq!(c.background, 0x0e091d);
        assert_eq!(c.foreground, 0xdc8f7c);
    }

    #[test]
    fn rejects_values_that_are_not_six_hex_digits() {
        assert_eq!(
            hex("\"#fff\""),
            None,
            "shorthand is not what omarchy writes"
        );
        assert_eq!(hex("\"#gggggg\""), None);
        assert_eq!(hex("\"#0e091d\""), Some(0x0e091d));
        assert_eq!(hex("0e091d"), Some(0x0e091d));
    }
}
