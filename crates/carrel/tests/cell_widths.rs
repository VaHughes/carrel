//! `carrel_core::cluster_width` against the cells: the two cannot drift.
//!
//! The layout measures a cluster with `cluster_width`, and the screen is a
//! ratatui buffer, which decides for itself how many cells a cluster takes
//! and whether it takes any. Where the two disagree, everything after the
//! cluster on its row is hit-tested, underlined and lit a column from where
//! it is painted. The core may not depend on ratatui, so the agreement is
//! held here: every Unicode scalar on its own, and the clusters whose
//! width is not the sum of their parts.

use carrel_core::{cluster_width, display_width};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

/// The columns the buffer advances for `cluster` written on its own.
fn cells(cluster: &str) -> u16 {
    let mut buf = Buffer::empty(Rect::new(0, 0, 64, 1));
    buf.set_stringn(0, 0, cluster, 64, Style::default()).0
}

/// A tab and a line break measure one cell and the buffer refuses both:
/// the core expands a tab before layout (one survives only as a table
/// row's cell boundary, painted as a space) and a line break ends a row
/// before it is measured as part of one. `cluster_width` documents both.
fn kept_on_purpose(c: char) -> bool {
    matches!(c, '\t' | '\n')
}

#[test]
fn every_scalar_measures_the_cells_it_is_given() {
    let mut differ = Vec::new();
    for c in (0..=0x10FFFFu32).filter_map(char::from_u32) {
        if kept_on_purpose(c) {
            continue;
        }
        let s = c.to_string();
        if cluster_width(&s) != cells(&s) {
            differ.push(format!(
                "U+{:04X} measures {} and takes {}",
                c as u32,
                cluster_width(&s),
                cells(&s)
            ));
        }
    }
    assert!(
        differ.is_empty(),
        "{} of them: {:?}",
        differ.len(),
        &differ[..differ.len().min(12)]
    );
}

/// Clusters of several scalars, and the blanks that look like spaces but
/// are not: Hangul fillers and a medial or final jamo with no consonant to
/// lead it take no cell at all, whatever a terminal handed them raw would
/// draw.
#[test]
fn a_cluster_measures_the_cells_it_is_given() {
    for (cluster, want) in [
        // Hangul: a filler, a lone medial, a lone final, a run of them.
        ("\u{3164}", 0),
        ("\u{FFA0}", 0),
        ("\u{1160}", 0),
        ("\u{1161}", 0),
        ("\u{11AB}", 0),
        ("\u{1161}\u{11AB}", 0),
        ("\u{D7B0}", 0),
        ("\u{D7CB}", 0),
        ("\u{3164}\u{301}", 0),
        // A leading consonant, its filler, and whole syllables.
        ("\u{115F}", 2),
        ("\u{115F}\u{1160}", 2),
        ("\u{1100}", 2),
        ("\u{1100}\u{1161}", 2),
        ("\u{1100}\u{1161}\u{11AB}", 2),
        ("\u{AC00}", 2),
        ("\u{AC00}\u{11AB}", 2),
        ("\u{A960}\u{D7B0}", 2),
        // Halfwidth forms, with and without their voiced marks.
        ("\u{FF76}", 1),
        ("\u{FF76}\u{FF9E}", 2),
        ("\u{FF8A}\u{FF9F}", 2),
        ("\u{FF9E}", 1),
        ("\u{FFA1}", 1),
        // Emoji: a presentation selector, a joined family, a flag, a
        // keycap, a skin tone, a tag sequence.
        ("\u{2764}\u{FE0F}", 2),
        ("\u{2764}", 1),
        ("\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}", 2),
        ("\u{1F1EB}\u{1F1F7}", 2),
        ("1\u{FE0F}\u{20E3}", 2),
        ("\u{1F44D}\u{1F3FD}", 2),
        (
            "\u{1F3F4}\u{E0067}\u{E0062}\u{E0065}\u{E006E}\u{E0067}\u{E007F}",
            2,
        ),
        // Marks on a base, and with no base.
        ("e\u{301}", 1),
        ("\u{65E5}\u{3099}", 2),
        ("\u{301}", 0),
        ("\u{0E01}\u{0E33}", 2),
        ("\u{0915}\u{094D}\u{0937}", 2),
        // Nothing at all.
        ("\u{200B}", 0),
        ("\u{200D}", 0),
        ("\u{2060}", 0),
        ("\u{FEFF}", 0),
        ("\u{202E}", 0),
        ("\u{AD}", 0),
        ("\u{1b}", 0),
        ("\r\n", 1),
        // Blanks that do take cells.
        ("\u{A0}", 1),
        ("\u{3000}", 2),
        ("\u{2800}", 1),
        ("\u{2003}", 1),
    ] {
        if cluster != "\r\n" {
            assert_eq!(cluster_width(cluster), cells(cluster), "{cluster:?}");
        }
        assert_eq!(cluster_width(cluster), want, "{cluster:?}");
    }
}

/// A row is its clusters side by side: no pair of them is narrower
/// together than apart (as `unicode-width` alone reads lam and alef), and
/// nothing the cells refuse takes its neighbor's column.
#[test]
fn a_row_measures_the_cells_it_is_given() {
    for row in [
        "\u{0644}\u{0627} \u{0627}\u{0644}\u{0633}\u{0644}\u{0627}\u{0645}",
        "a\u{3164}b\u{FFA0}c\u{1160}d\u{1161}\u{11AB}e",
        "\u{3164}\u{3164}\u{3164} \u{D55C}\u{AE00} \u{1112}\u{1161}\u{11AB}\u{1100}\u{1173}\u{11AF}",
        "\u{FF76}\u{FF9E}\u{FF77}\u{FF9E} \u{FF8A}\u{FF9F} \u{FF9E}x",
        "x\u{200B}y\u{200D}z\u{FEFF}\u{202E}w\u{2069}",
        "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467} \u{1F1EB}\u{1F1F7}\u{1F1E9}\u{1F1EA} 1\u{FE0F}\u{20E3}",
        "e\u{301}\u{301} \u{301}a \u{0E01}\u{0E33}\u{0E32} \u{0915}\u{094D}\u{0937}\u{093F}",
        "\u{A0}\u{3000}\u{2003}\u{2800}|",
    ] {
        assert_eq!(display_width(row), cells(row), "{row:?}");
    }
}
