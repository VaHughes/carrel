//! Hyphenation in narrow windows: a long word that misses the end of a row is
//! divided to fill it, and the hyphen is paint, never text.
//!
//! The core decides where a word may be divided and proves the division adds
//! no byte and drops none (`carrel-core`'s layout tests). What is tested here
//! is the part only a frontend can get wrong: that the hyphen lands in the
//! cell reserved for it, that the pointer still resolves to the character it
//! is over, that a selection and a resize behave on a row with no gap after
//! it, and that the preference and the document's language are both honored.

use carrel::action::{Action, Span};
use carrel::app::{App, Setting, settings_rows, update};
use carrel_core::{BlockIdx, Document, Row, RowKind, display_width};
use proptest::prelude::*;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Modifier;

/// English prose with words long enough to miss the end of a narrow row.
const PROSE: &str = "The reader keeps documentation comfortable when the terminal becomes \
uncomfortably narrow, and that is the situation where an ordinary paragraph with several \
considerable words would otherwise leave a particularly ragged edge that you have to read \
around. This paragraph exists so that the hyphenation has something substantial to divide.";

/// The same shape of text in a language the English patterns do not know.
const GERMAN: &str = "Das Programm zeigt die Dokumentation in einem schmalen Fenster, und die \
Geschwindigkeit der Verarbeitung ist dabei nicht wichtig. Wir sehen hier einen gewöhnlichen \
Absatz mit mehreren beträchtlichen Wörtern, der sonst einen besonders unruhigen Rand \
hinterlassen würde.";

fn app_at(cols: u16, rows: u16, src: &str) -> App {
    let mut app = App::new("t.md".into(), Document::parse(src), cols, rows);
    // The band and the hint row are not what is under test; without them the
    // text starts on a known row at every size used here.
    app.breadcrumb = false;
    app.on_resize(cols, rows);
    app
}

fn frame(app: &App, cols: u16, rows: u16) -> Buffer {
    let mut t = Terminal::new(TestBackend::new(cols, rows)).unwrap();
    t.draw(|f| carrel::render::draw(f, app)).unwrap();
    t.backend().buffer().clone()
}

/// Every layout row of block 0, with whether it ends in a division.
fn rows_of(app: &App) -> Vec<(Row, bool)> {
    let mut rows = Vec::new();
    app.layout.rows_for(&app.doc, BlockIdx(0), &mut rows);
    rows.into_iter()
        .filter(|r| matches!(r.kind, RowKind::Text { .. }))
        .map(|r| {
            let hyphen = matches!(r.kind, RowKind::Text { hyphen: true, .. });
            (r, hyphen)
        })
        .collect()
}

fn divisions(app: &App) -> usize {
    rows_of(app).iter().filter(|(_, h)| *h).count()
}

#[test]
fn a_narrow_window_divides_words_and_paints_each_hyphen_in_its_own_cell() {
    let (cols, lines) = (36, 40);
    let app = app_at(cols, lines, PROSE);
    let buf = frame(&app, cols, lines);
    let x = App::text_x(cols, app.max_width, 0);
    let top = app.text_y();

    let rows = rows_of(&app);
    assert!(
        rows.iter().filter(|(_, h)| *h).count() >= 2,
        "the fixture must actually divide something"
    );
    for (i, (row, hyphen)) in rows.iter().enumerate() {
        let text = &app.doc.text[row.doc.start as usize..row.doc.end as usize];
        let end = x + row.indent + display_width(text);
        let y = top + u16::try_from(i).unwrap();
        let after = &buf[(end, y)];
        if *hyphen {
            assert_eq!(after.symbol(), "-", "row {i} {text:?}: no hyphen after it");
            assert!(
                end < x + app.text_w(),
                "row {i}: the hyphen is painted outside the text column"
            );
            // A division is mid-word on both sides, and the next row picks
            // up at the very next byte.
            assert_eq!(row.doc.end, rows[i + 1].0.doc.start, "row {i}: a gap");
            assert!(text.ends_with(|c: char| c.is_ascii_lowercase()));
        } else {
            assert_eq!(
                after.symbol(),
                " ",
                "row {i} {text:?}: something painted after an undivided row"
            );
        }
    }
}

#[test]
fn the_hyphen_does_not_pass_for_one_the_author_wrote() {
    // It is decoration, and painted as decoration: dimmer than the letters
    // beside it, so a divided `re-` cannot be read as a written `re-`.
    let (cols, lines) = (36, 40);
    let app = app_at(cols, lines, PROSE);
    let buf = frame(&app, cols, lines);
    let x = App::text_x(cols, app.max_width, 0);
    let (i, (row, _)) = rows_of(&app)
        .into_iter()
        .enumerate()
        .find(|(_, (_, h))| *h)
        .unwrap();
    let text = &app.doc.text[row.doc.start as usize..row.doc.end as usize];
    let end = x + row.indent + display_width(text);
    let y = app.text_y() + u16::try_from(i).unwrap();
    assert_ne!(
        buf[(end, y)].style(),
        buf[(end - 1, y)].style(),
        "the hyphen is styled exactly like the letter before it"
    );
}

#[test]
fn a_comfortable_window_is_never_hyphenated() {
    for cols in [80u16, 100, 200] {
        let app = app_at(cols, 40, PROSE);
        assert_eq!(divisions(&app), 0, "divided at {cols} columns");
    }
}

#[test]
fn the_preference_turns_it_off_and_the_rows_are_the_plain_ones() {
    let mut app = app_at(36, 40, PROSE);
    assert!(divisions(&app) > 0, "on by default");
    let on: Vec<_> = rows_of(&app).into_iter().map(|(r, _)| r.doc).collect();

    app.hyphenate = false;
    app.relayout();
    assert_eq!(divisions(&app), 0);
    let off: Vec<_> = rows_of(&app).into_iter().map(|(r, _)| r.doc).collect();
    assert_ne!(on, off, "the setting must change where rows break");
    // Off is the layout the core's plain `wrap` produces, byte for byte.
    let mut plain = Vec::new();
    carrel_core::wrap(
        &app.doc,
        BlockIdx(0),
        app.text_w(),
        &carrel_core::cluster_width,
        |r| plain.push(r.doc),
    );
    assert_eq!(off, plain);
}

#[test]
fn a_document_that_does_not_read_as_english_is_left_alone() {
    let app = app_at(36, 40, GERMAN);
    assert_eq!(
        divisions(&app),
        0,
        "English patterns must not divide German words"
    );
}

#[test]
fn a_declared_language_decides_in_both_directions() {
    // Too short, and too bare of function words, to be judged English on
    // its own — so without the declaration nothing is divided...
    let bare = "Documentation considerations accumulate substantially whenever \
        international organizations communicate complicated requirements.";
    let hyphens = |app: &App, block: u32| {
        let mut rows = Vec::new();
        app.layout.rows_for(&app.doc, BlockIdx(block), &mut rows);
        rows.iter()
            .filter(|r| matches!(r.kind, RowKind::Text { hyphen: true, .. }))
            .count()
    };
    assert_eq!(hyphens(&app_at(36, 60, bare), 0), 0, "the control");
    // ...and with it, the reader has said what the language is.
    let app = app_at(36, 60, &format!("---\nlang: en\n---\n\n{bare}"));
    assert!(hyphens(&app, 1) > 0);
    // ...and English prose that says it is German is left alone.
    assert!(hyphens(&app_at(36, 60, PROSE), 0) > 0, "the control");
    let app = app_at(36, 60, &format!("---\nlang: de\n---\n\n{PROSE}"));
    assert_eq!(hyphens(&app, 1), 0);
}

/// The round trip `tests/measure.rs` guards, on rows that end in a hyphen.
///
/// A hyphen cell is a painted cell with no byte behind it. If hit-testing
/// counted it as content, every click right of it would resolve one
/// character late — and no frame test would notice.
#[test]
fn the_pointer_lands_on_the_character_painted_under_it() {
    for cols in [24u16, 30, 36, 44, 60] {
        let lines = 60;
        let app = app_at(cols, lines, PROSE);
        let buf = frame(&app, cols, lines);
        let x = App::text_x(cols, app.max_width, 0);
        let top = app.text_y();
        let rows = rows_of(&app);
        assert!(rows.iter().any(|(_, h)| *h), "cols={cols}: nothing divided");
        let mut checked = 0;
        for (i, (row, hyphen)) in rows.iter().enumerate() {
            let y = top + u16::try_from(i).unwrap();
            let text = &app.doc.text[row.doc.start as usize..row.doc.end as usize];
            let end = x + row.indent + display_width(text);
            for col in x..x + app.text_w() {
                let Some((start, stop)) = app.doc_span_at(col, y) else {
                    continue;
                };
                let byte = &app.doc.text[start as usize..stop as usize];
                let painted = buf[(col, y)].symbol();
                if col >= end {
                    // Past the content: the hyphen cell and the blank cells
                    // after it resolve to nothing, never to a letter.
                    assert_eq!(byte, "", "cols={cols} row {i} col {col}");
                    assert!(painted == " " || (*hyphen && col == end));
                    continue;
                }
                assert_eq!(byte, painted, "cols={cols} row {i} col {col}");
                checked += 1;
            }
        }
        assert!(checked > 40, "cols={cols}: checked almost nothing");
    }
}

#[test]
fn selecting_a_divided_word_selects_its_letters_and_not_its_hyphen() {
    let (cols, lines) = (36, 40);
    let mut app = app_at(cols, lines, PROSE);
    let rows = rows_of(&app);
    let i = rows.iter().position(|(_, h)| *h).unwrap();
    let (head, tail) = (&rows[i].0, &rows[i + 1].0);
    // The whole divided word: from its first letter on the upper row to the
    // space after it on the lower one.
    let text = &app.doc.text;
    let word_start = text[..head.doc.end as usize]
        .rfind(' ')
        .map_or(0, |p| p + 1);
    let word_end = head.doc.end as usize + text[head.doc.end as usize..].find(' ').unwrap();
    let word = text[word_start..word_end].to_string();
    assert!(
        !word.contains('-'),
        "{word:?}: the text holds the word whole"
    );
    app.selection = Some(word_start as u32..word_end as u32);

    let buf = frame(&app, cols, lines);
    let x = App::text_x(cols, app.max_width, 0);
    let top = app.text_y();
    let reversed = |col: u16, row: usize| {
        buf[(col, top + u16::try_from(row).unwrap())]
            .style()
            .add_modifier
            .contains(Modifier::REVERSED)
    };
    let head_text = &text[head.doc.start as usize..head.doc.end as usize];
    let head_end = x + head.indent + display_width(head_text);
    assert!(
        reversed(head_end - 1, i),
        "the head's last letter is selected"
    );
    assert!(
        !reversed(head_end, i),
        "the hyphen is not part of the selection"
    );
    assert!(
        reversed(x + tail.indent, i + 1),
        "the tail's first letter is"
    );
    let tail_cells = u16::try_from(word_end).unwrap() - u16::try_from(tail.doc.start).unwrap();
    assert!(
        !reversed(x + tail.indent + tail_cells, i + 1),
        "and it stops"
    );
}

#[test]
fn a_search_finds_a_word_whether_or_not_it_is_divided() {
    // The match is a byte range in text that holds no hyphen, so the same
    // needle finds the same word at a width that divides it and one that
    // does not.
    let narrow = app_at(36, 40, PROSE);
    let rows = rows_of(&narrow);
    let i = rows.iter().position(|(_, h)| *h).unwrap();
    let text = &narrow.doc.text;
    let at = rows[i].0.doc.end as usize;
    let start = text[..at].rfind(' ').map_or(0, |p| p + 1);
    let end = at + text[at..].find(|c: char| !c.is_ascii_lowercase()).unwrap();
    let word = &text[start..end];

    let hit = carrel_core::search(&narrow.doc, word, true);
    assert_eq!(hit.ranges.first(), Some(&(start as u32..end as u32)));
    let wide = app_at(120, 40, PROSE);
    assert_eq!(
        carrel_core::search(&wide.doc, word, true).ranges,
        hit.ranges
    );
}

#[test]
fn the_settings_pane_changes_it_and_writes_it_down() {
    let cfg = tempfile::tempdir().unwrap();
    let mut app = app_at(36, 40, PROSE);
    app.config_dir = Some(cfg.path().to_path_buf());
    assert!(divisions(&app) > 0);

    update(&mut app, Action::SettingsToggle);
    let at = settings_rows(&app)
        .iter()
        .position(|r| r.key == Setting::Hyphenate)
        .expect("the preference has a row");
    assert_eq!(settings_rows(&app)[at].value, "on");
    update(&mut app, Action::SettingsPickAt(u32::try_from(at).unwrap()));

    assert!(!app.hyphenate);
    assert_eq!(settings_rows(&app)[at].value, "off");
    assert_eq!(divisions(&app), 0, "the page re-wrapped at once");
    assert_eq!(
        carrel::config::load_hyphenate_in(cfg.path()),
        Some(false),
        "and the choice is on disk"
    );
}

#[test]
fn plain_and_rendered_output_never_carry_a_hyphen_that_is_not_in_the_text() {
    // Output that leaves the screen is text someone else will read, grep or
    // paste. A division written into it would be a character to take back out.
    let doc = Document::parse(PROSE);
    for out in [
        carrel::plain::render(&doc, 36),
        carrel::ansi::render(&doc, 36),
    ] {
        assert!(!PROSE.contains('-'), "the fixture has no hyphen of its own");
        assert!(!out.contains('-'), "a hyphen reached piped output:\n{out}");
        // And no word was divided silently, either: each one is still whole
        // on some line.
        for word in PROSE.split_whitespace() {
            assert!(out.contains(word), "{word:?} was broken in:\n{out}");
        }
    }
}

proptest! {
    /// `tests/resize.rs`'s invariant, over prose that divides. A divided row
    /// is followed by one with no gap, so an anchor can sit on the first
    /// letter of a tail — mid-word — and must still come back to it.
    #[test]
    fn the_reader_does_not_move_on_resize_when_words_are_divided(
        scroll in 0i32..30,
        w1 in 12u16..=120,
        w2 in 12u16..=120,
        h in 6u16..=40,
    ) {
        let top_row = |app: &App| {
            let b = app.layout.block_at_row(app.view.scroll_row);
            let mut rows = Vec::new();
            app.layout.rows_for(&app.doc, b, &mut rows);
            let sub = (app.view.scroll_row - app.layout.row_start(b)) as usize;
            rows.get(sub).map_or(0..0, |r| r.doc.clone())
        };
        let src = format!("{PROSE}\n\n{PROSE}\n\n- {PROSE}\n");
        let mut app = App::new("t.md".into(), Document::parse(&src), w1, h);
        update(&mut app, Action::Scroll(Span::Line, scroll));
        let before = app.view.anchor;
        prop_assert_eq!(top_row(&app).start, before, "precondition");

        app.on_resize(w2, h);
        prop_assert_eq!(app.view.anchor, before);
        let top = top_row(&app);
        prop_assert!(top.start <= before, "top {:?} is past the anchor {}", top, before);

        app.on_resize(w1, h);
        prop_assert_eq!(top_row(&app).start, before, "did not come back to the same row");
    }
}

/// Every way a document arrives must wrap it the same way. `App::new` runs
/// the full relayout; the openers build a provisional layout first, and a
/// provisional layout that were the one painted would show plain rows until
/// the first resize.
#[test]
fn a_document_opened_later_is_hyphenated_like_one_opened_first() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("prose.md");
    std::fs::write(&path, PROSE).unwrap();

    let mut opened = App::new("t.md".into(), Document::parse("x"), 36, 40);
    opened.breadcrumb = false;
    opened.on_resize(36, 40);
    opened.open_path(&path).unwrap();
    assert!(divisions(&opened) > 0, "open_path");

    let first = app_at(36, 40, PROSE);
    let rows = |app: &App| -> Vec<_> { rows_of(app).into_iter().map(|(r, _)| r.doc).collect() };
    assert_eq!(rows(&opened), rows(&first));

    // A reload and a growing pipe come through `reload_from`.
    let mut piped = App::new("(stdin)".into(), Document::parse("x"), 36, 40);
    piped.breadcrumb = false;
    piped.on_resize(36, 40);
    piped.reload_from(PROSE);
    assert_eq!(rows(&piped), rows(&first), "reload_from");

    let mut welcome = App::new("t.md".into(), Document::parse("x"), 36, 40);
    welcome.open_welcome();
    let mut any = false;
    for b in 0..welcome.doc.block_count() {
        let mut r = Vec::new();
        welcome
            .layout
            .rows_for(&welcome.doc, BlockIdx(b as u32), &mut r);
        any |= r
            .iter()
            .any(|r| matches!(r.kind, RowKind::Text { hyphen: true, .. }));
    }
    assert!(
        any,
        "the built-in document is English prose in a narrow window"
    );
}
