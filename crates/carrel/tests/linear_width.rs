//! `--plain` and `--render` against the width they were given.
//!
//! The contract, in the words of `fmt`: **words are never divided and no
//! character is added.** Every line fits the width, except a line holding one
//! unbreakable run that by itself — with the row's indent, quote bars and
//! list prefix, which count — is wider than the width. That run sits on a
//! line of its own: what comes before it ends the line above, and what comes
//! after it wraps onto the lines below, each of which fits.
//!
//! An *unbreakable run* is a stretch of text with no UAX #14 break
//! opportunity inside it, the author's soft hyphen (U+00AD) not counting as
//! one. Soft hyphens are not break points in linear output at all: a word
//! that fits a line moves to the next one whole.
//!
//! `--render` with styling on keeps that line structure, except that a run
//! wider than the whole row is divided the way the screen divides it — at a
//! soft hyphen or its language's point with a hyphen, otherwise cut with a
//! `↩` — and the added mark is FAINT (SGR 2). Under `NO_COLOR` it is
//! `--plain`, byte for byte.

use carrel::ansi::render_with;
use carrel::plain::render;
use carrel_core::{Document, display_width};
use proptest::prelude::*;

/// 48 characters with no break opportunity in them: a hash.
const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef";

fn plain(src: &str, width: u16) -> String {
    render(&Document::parse(src), width)
}

fn styled(src: &str, width: u16) -> String {
    render_with(&Document::parse(src), width, true)
}

fn width_of(line: &str) -> usize {
    usize::from(display_width(line))
}

/// What a styled line shows: SGR and OSC 8 sequences removed.
fn visible(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(i) = rest.find('\u{1b}') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let end = if tail.starts_with("\u{1b}[") {
            tail.find('m').map_or(tail.len(), |m| m + 1)
        } else if tail.starts_with("\u{1b}]") {
            tail.find("\u{1b}\\").map_or(tail.len(), |t| t + 2)
        } else {
            1
        };
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// Every line fits `width`, except the lines named, which must be exactly
/// `structure + run`.
fn assert_fits_except(out: &str, width: u16, overhangs: &[&str]) {
    for line in out.lines() {
        if width_of(line) > usize::from(width) {
            assert!(
                overhangs.contains(&line),
                "line {line:?} is {} cells, past {width}, and is not a lone run:\n{out}",
                width_of(line)
            );
        }
    }
    for want in overhangs {
        assert!(
            out.lines().any(|l| l == *want),
            "expected the line {want:?} on its own:\n{out}"
        );
    }
}

#[test]
fn an_overlong_token_mid_paragraph_stands_alone_and_the_text_after_it_wraps() {
    let src = format!("A hash {HASH} and https://example.com/some/long/path more words follow.\n");
    let out = plain(&src, 30);
    assert_fits_except(&out, 30, &[HASH]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "A hash", "{out}");
    assert_eq!(lines[1], HASH, "{out}");
    assert!(lines[2].starts_with("and "), "{out}");
}

#[test]
fn an_overlong_token_opening_or_closing_a_paragraph_stands_alone() {
    let out = plain(&format!("{HASH} then words that wrap along\n"), 30);
    assert_fits_except(&out, 30, &[HASH]);
    assert!(out.starts_with(&format!("{HASH}\nthen ")), "{out}");

    let out = plain(&format!("Words that come first and then {HASH}\n"), 30);
    assert_fits_except(&out, 30, &[HASH]);
    assert!(out.ends_with(&format!("\n{HASH}\n")), "{out}");

    let out = plain(&format!("{HASH}\n"), 30);
    assert_eq!(out, format!("{HASH}\n"));
}

#[test]
fn a_soft_hyphenated_word_that_fits_moves_to_the_next_line_whole() {
    // At 20 cells `Twelve chars ` leaves a hole of seven: room for `super`
    // and its hyphen, which is exactly where the screen divides it — and
    // exactly what linear output must not: the word was rejoined there and
    // ran the line to 33 cells.
    let src = "Twelve chars super&shy;cali&shy;fragi&shy;listic end.\n";
    let out = plain(src, 20);
    assert!(!out.contains('\u{AD}'), "{out:?}");
    assert_fits_except(&out, 20, &[]);
    assert!(
        out.lines().any(|l| l.contains("supercalifragilistic")),
        "the word is whole on one line:\n{out}"
    );
    assert!(out.starts_with("Twelve chars\n"), "{out}");
}

#[test]
fn a_soft_hyphenated_word_wider_than_the_row_stands_alone_whole() {
    let src = "One super&shy;cali&shy;fragi&shy;listic&shy;expi&shy;ali&shy;docious two\n";
    let out = plain(src, 20);
    assert_fits_except(&out, 20, &["supercalifragilisticexpialidocious"]);
    assert_eq!(
        out, "One\nsupercalifragilisticexpialidocious\ntwo\n",
        "{out}"
    );
}

#[test]
fn a_list_items_prefix_and_indent_count_toward_the_width() {
    let out = plain(&format!("- item with {HASH} tail words\n"), 30);
    let hung = format!("  {HASH}");
    assert_fits_except(&out, 30, &[&hung]);
    assert_eq!(out, format!("- item with\n{hung}\n  tail words\n"));

    // A run that fits the width but not beside the indent is wider than the
    // ROW: it stands alone and overhangs by the indent's worth.
    let run = &HASH[..29];
    let out = plain(&format!("- {run} after\n"), 30);
    let first = format!("- {run}");
    assert_fits_except(&out, 30, &[&first]);
    assert_eq!(out, format!("{first}\n  after\n"));

    // And one that fits beside the indent is not an overhang at all.
    let run = &HASH[..28];
    let out = plain(&format!("- {run} after\n"), 30);
    assert_fits_except(&out, 30, &[]);
}

#[test]
fn quote_bars_count_toward_the_width() {
    let out = plain(&format!("> quoted {HASH} tail of the quote\n"), 30);
    let barred = format!("> {HASH}");
    assert_fits_except(&out, 30, &[&barred]);
    assert_eq!(out, format!("> quoted\n{barred}\n> tail of the quote\n"));

    let out = plain(&format!("> > nested {HASH} tail\n"), 30);
    let barred = format!("> > {HASH}");
    assert_fits_except(&out, 30, &[&barred]);
    assert!(out.ends_with("> > tail\n"), "{out}");
}

#[test]
fn wide_clusters_are_measured_in_cells() {
    // `©️` is two cells and `AL` to UAX #14, so fifteen of them are one
    // thirty-cell run. CJK and emoji beside it break between characters.
    let run = "©️".repeat(15);
    let src = format!("An emoji {run} run and 日本語のテキストです。😀 text\n");
    let out = plain(&src, 21);
    assert_fits_except(&out, 21, &[&run]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "An emoji", "{out}");
    assert_eq!(lines[1], run, "{out}");

    // Ten of them are twenty cells: a run that fits exactly is not an
    // overhang, and a measure in chars would call it ten.
    let run = "©️".repeat(10);
    let out = plain(&format!("ab {run} cd\n"), 21);
    assert_fits_except(&out, 21, &[]);
    assert_eq!(out, format!("ab\n{run}\ncd\n"));
}

#[test]
fn a_heading_divides_no_word_either() {
    let src = format!("# Title with {HASH} tail words\n");
    let out = plain(&src, 30);
    assert_fits_except(&out, 30, &[HASH]);
    assert_eq!(out, format!("Title with\n{HASH}\ntail words\n"));
}

#[test]
fn a_code_token_is_never_cut_in_linear_output() {
    let lit = format!("\"{HASH}\";");
    let src = format!("```\nlet x = {lit} foo bar\n```\n");
    let out = plain(&src, 30);
    assert!(
        out.lines().any(|l| l.trim_start() == lit),
        "the literal is whole and alone on its line:\n{out}"
    );
    for l in out.lines() {
        if width_of(l) > 30 {
            assert_eq!(l.trim_start(), lit, "only the literal overhangs:\n{out}");
        }
    }
    // Styled output keeps the same code lines: the screen marks a cut code
    // line at the start of its next row, in cells linear output gives to
    // the code's own indentation.
    assert_eq!(
        visible(&styled(&src, 30)),
        out,
        "styled code lines are plain code lines"
    );
}

/// The fixtures every test above uses, for the identity and styled checks.
fn fixtures() -> Vec<String> {
    vec![
        format!("A hash {HASH} and https://example.com/some/long/path more words.\n"),
        format!("{HASH} then words\n\nWords and then {HASH}\n"),
        "Twelve chars super&shy;cali&shy;fragi&shy;listic end.\n".into(),
        "One super&shy;cali&shy;fragi&shy;listic&shy;expi&shy;ali&shy;docious two\n".into(),
        format!("- item with {HASH} tail words\n\n> quoted {HASH} tail\n"),
        format!(
            "An emoji {} run and 日本語のテキストです。😀 text\n",
            "©️".repeat(15)
        ),
        format!("# Title with {HASH} tail words\n"),
        format!("```\nlet x = \"{HASH}\"; foo bar\n```\n"),
        format!("**bold {HASH} words** and [a link {HASH}](https://e.x) after\n"),
    ]
}

#[test]
fn no_color_render_is_plain_byte_for_byte() {
    for src in fixtures() {
        let doc = Document::parse(&src);
        for w in [20u16, 30, 41, 80] {
            assert_eq!(
                render_with(&doc, w, false),
                render(&doc, w),
                "width {w}, {src:?}"
            );
        }
    }
}

/// The faint marks styled output may add, and nothing else.
const FAINT_HYPHEN: &str = "\u{1b}[2m-\u{1b}[0m";
const FAINT_CUT: &str = "\u{1b}[2m↩\u{1b}[0m";

#[test]
fn styled_output_cuts_an_overlong_run_with_a_faint_mark() {
    let src = format!("A hash {HASH} and more words.\n");
    let out = styled(&src, 30);
    for line in out.lines() {
        assert!(width_of(&visible(line)) <= 30, "{line:?} in:\n{out}");
    }
    let marked: Vec<&str> = out.lines().filter(|l| l.ends_with(FAINT_CUT)).collect();
    assert!(!marked.is_empty(), "the cut is marked:\n{out:?}");
    // The mark is the only thing added: take it out and the hash is whole.
    let joined: String = out
        .lines()
        .map(|l| {
            l.strip_suffix(FAINT_CUT)
                .map_or(l.to_string(), |s| s.to_string() + "\u{0}")
        })
        .collect::<Vec<_>>()
        .join("\n")
        .replace("\u{0}\n", "");
    assert!(visible(&joined).contains(HASH), "{joined:?}");
    // Text before the run ends its line, as in plain output.
    assert_eq!(out.lines().next(), Some("A hash"), "{out:?}");
}

#[test]
fn styled_output_divides_an_overlong_word_at_its_soft_hyphens() {
    let src = "One super&shy;cali&shy;fragi&shy;listic&shy;expi&shy;ali&shy;docious two\n";
    let out = styled(src, 20);
    assert!(!out.contains('\u{AD}'), "{out:?}");
    assert!(
        !out.contains('↩'),
        "a soft hyphen fits, so nothing is cut:\n{out:?}"
    );
    let lines: Vec<String> = out.lines().map(visible).collect();
    assert_eq!(lines[0], "One", "{out:?}");
    for l in &lines {
        assert!(width_of(l) <= 20, "{l:?} in {out:?}");
    }
    // `supercalifragilistic` is twenty cells and its hyphen a twenty-first,
    // so the latest soft hyphen that fits is the one before `listic` —
    // where the screen, breaking at each, ends the row too.
    assert_eq!(
        out,
        format!("One\nsupercalifragi{FAINT_HYPHEN}\nlisticexpialidocious\ntwo\n"),
    );
    // And a soft-hyphenated word that FITS is not divided at all.
    let out = styled(
        "Twelve chars super&shy;cali&shy;fragi&shy;listic end.\n",
        20,
    );
    assert!(!out.contains(FAINT_HYPHEN), "{out:?}");
    assert!(out.starts_with("Twelve chars\n"), "{out:?}");
}

#[test]
fn styled_headings_and_quotes_mark_their_cuts_too() {
    let out = styled(&format!("# Title with {HASH} tail\n"), 30);
    assert!(out.contains(FAINT_CUT), "{out:?}");
    for line in out.lines() {
        assert!(width_of(&visible(line)) <= 30, "{line:?} in {out:?}");
    }
    let out = styled(&format!("> quoted {HASH} tail\n"), 30);
    assert!(out.contains(FAINT_CUT), "{out:?}");
    for line in out.lines() {
        let v = visible(line);
        assert!(v.starts_with("> "), "every line keeps its bar: {out:?}");
        assert!(width_of(&v) <= 30, "{v:?} in {out:?}");
    }
}

/// One generated block: words joined by spaces, in a paragraph, a list item
/// or a block quote, and the structure each output line opens with.
#[derive(Debug, Clone)]
struct Case {
    words: Vec<String>,
    kind: u8,
}

impl Case {
    fn source(&self) -> String {
        let body = self.words.join(" ");
        match self.kind {
            0 => format!("{body}\n"),
            1 => format!("- {body}\n"),
            _ => format!("> {body}\n"),
        }
    }

    /// Strip the structure an output line opens with: `(structure, rest)`.
    fn split<'a>(&self, line: &'a str) -> (&'a str, &'a str) {
        // A list item's first line opens `- `, the rest hang under it;
        // a quote's every line opens `> `. Two cells either way.
        let n = if self.kind == 0 { 0 } else { 2 };
        (&line[..n.min(line.len())], &line[n.min(line.len())..])
    }
}

/// A word: ASCII letters and digits — UAX #14 never breaks between those —
/// with soft hyphens inside, which linear output does not break at either.
/// So a word is an unbreakable run, and its width is its letter count.
fn word() -> impl Strategy<Value = String> {
    prop_oneof![
        4 => prop::collection::vec("[a-z0-9]{1,9}", 1..6).prop_map(|p| p.join("\u{AD}")),
        1 => "[a-z0-9]{20,60}",
        // Wide clusters, which break on both sides of every one.
        1 => prop::collection::vec(prop::sample::select(vec!["日", "本", "語", "😀", "👨‍👩‍👧"]), 1..12)
            .prop_map(|v| v.concat()),
    ]
}

fn case() -> impl Strategy<Value = Case> {
    (prop::collection::vec(word(), 1..30), 0u8..3).prop_map(|(words, kind)| Case { words, kind })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    /// Every plain line fits, unless it is structure plus exactly one whole
    /// ASCII word (its soft hyphens dropped) that does not fit beside that
    /// structure; nothing is added, nothing is lost, and no word is divided.
    #[test]
    fn plain_lines_fit_or_hold_one_unbreakable_run(c in case(), width in 20u16..61) {
        let src = c.source();
        let doc = Document::parse(&src);
        let out = render(&doc, width);
        prop_assert_eq!(&render_with(&doc, width, false), &out);
        let words: Vec<String> = c.words.iter().map(|w| w.replace('\u{AD}', "")).collect();
        let mut seen = String::new();
        for line in out.lines() {
            let (structure, rest) = c.split(line);
            if width_of(line) > usize::from(width) {
                prop_assert!(
                    rest.is_ascii() && words.iter().any(|w| w == rest),
                    "{:?} overhangs {} and is not one whole word:\n{}", line, width, out
                );
                prop_assert!(width_of(structure) + rest.len() > usize::from(width));
            }
            seen.push_str(rest);
            seen.push(' ');
        }
        // Each ASCII word is whole somewhere, as a word of its own.
        let tokens: Vec<&str> = seen.split_whitespace().collect();
        for w in words.iter().filter(|w| w.is_ascii()) {
            prop_assert!(tokens.contains(&w.as_str()), "{:?} was divided:\n{}", w, out);
        }
        // Nothing added, nothing lost.
        let squash = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        prop_assert_eq!(squash(&seen), squash(&words.concat()));
    }

    /// Styled lines always fit, and the only thing they add is a faint mark
    /// at the end of a line that divides a run.
    #[test]
    fn styled_lines_always_fit_and_add_only_faint_marks(c in case(), width in 20u16..61) {
        let src = c.source();
        let out = render_with(&Document::parse(&src), width, true);
        let words: Vec<String> = c.words.iter().map(|w| w.replace('\u{AD}', "")).collect();
        let mut seen = String::new();
        for line in out.lines() {
            let shown = visible(line);
            prop_assert!(width_of(&shown) <= usize::from(width), "{:?} in:\n{}", shown, out);
            let body = line
                .strip_suffix(FAINT_HYPHEN)
                .or_else(|| line.strip_suffix(FAINT_CUT));
            let divided = body.is_some();
            let shown = visible(body.unwrap_or(line));
            let (_, rest) = c.split(&shown);
            seen.push_str(rest);
            if !divided {
                seen.push(' ');
            }
        }
        let tokens: Vec<&str> = seen.split_whitespace().collect();
        for w in words.iter().filter(|w| w.is_ascii()) {
            prop_assert!(tokens.contains(&w.as_str()), "{:?} lost or mangled:\n{:?}", w, out);
        }
        let squash = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        prop_assert_eq!(squash(&seen), squash(&words.concat()));
    }
}
