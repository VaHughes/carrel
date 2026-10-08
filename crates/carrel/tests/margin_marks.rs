//! The left margin's marks never cost each other.
//!
//! A block can carry four marks beside its text: the change bar (the last
//! reload touched it), the bookmark dot, the code-block cursor's bar, and a
//! heading's fold marker. They share at most two cells per row — the one
//! next to the text (the state column) and the one beyond it (the fold
//! column) — and fewer in a narrow window. These used to be painted in turn,
//! each over the last, so a bookmarked one-row block showed no change bar, a
//! fold marker in a one-cell margin erased it, and in a window with no
//! margin at all the fold marker was painted over the heading's first
//! letter.
//!
//! The rule these pin:
//! - nothing is ever painted over text;
//! - with any margin at all, a change is visible on every content row of the
//!   block: as the bar, or — where a point mark needs the bar's cell — as
//!   that mark cut into the bar (reversed);
//! - a bookmark always keeps a cell on the block's first row;
//! - a fold marker keeps the fold column when there is one, and the state
//!   column in a one-cell margin unless a bookmark needs it (the fold state
//!   then still shows as the trailing `…`, and the heading row still folds);
//! - every fold target sits on a painted fold marker, and every painted
//!   fold marker is a target.

use carrel::action::{Action, Targets};
use carrel::app::{App, update};
use carrel::render::Painted;
use carrel_core::{BlockIdx, Document};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Modifier;

const SRC: &str = "# Plan\n\nShort line.\n\nA paragraph long enough to wrap onto several \
rows at every width this test uses, narrow ones included, so it goes on for a while longer \
still and then a little more.\n\n```\ncode one\ncode two\n```\n\n## Tail\n";

const ROWS: u16 = 70;

fn app_at(cols: u16) -> App {
    let mut app = App::new("t.md".into(), Document::parse(SRC), cols, ROWS);
    app.breadcrumb = false;
    app.on_resize(cols, ROWS);
    app
}

fn frame(app: &App, cols: u16) -> (Buffer, Targets) {
    let mut t = Terminal::new(TestBackend::new(cols, ROWS)).unwrap();
    let mut painted = Painted::default();
    t.draw(|f| {
        carrel::render::draw_full(f, app, &mut painted, &mut std::collections::HashMap::new());
    })
    .unwrap();
    (t.backend().buffer().clone(), painted.targets)
}

fn block(app: &App, starts: &str) -> BlockIdx {
    (0..app.doc.block_count())
        .map(|i| BlockIdx(u32::try_from(i).unwrap()))
        .find(|b| app.doc.block_text(*b).starts_with(starts))
        .unwrap_or_else(|| panic!("no block starting {starts:?}"))
}

/// The screen row of a block's first row: scrolled to the top, rows are
/// layout rows offset by the reader's top edge.
fn first_y(app: &App, b: BlockIdx) -> u16 {
    u16::try_from(app.layout.row_start(b)).unwrap() + app.text_y()
}

#[derive(Clone, Copy, Debug)]
#[allow(clippy::struct_excessive_bools)] // four independent marks, one flag each
struct Marks {
    changed: bool,
    bookmarked: bool,
    folded: bool,
    focused: bool,
}

fn reversed(buf: &Buffer, x: u16, y: u16) -> bool {
    buf[(x, y)].modifier.contains(Modifier::REVERSED)
}

fn sym(buf: &Buffer, x: u16, y: u16) -> &str {
    buf[(x, y)].symbol()
}

/// Every assertion the rule makes about one block.
#[allow(clippy::too_many_lines, clippy::many_single_char_names)]
fn check(app: &App, buf: &Buffer, targets: &Targets, b: BlockIdx, m: Marks, ctx: &str) {
    let (x, _) = app.block_span_x(b);
    let y0 = first_y(app, b);
    let rows = u16::try_from(app.layout.content_height(&app.doc, b)).unwrap();
    let text = app.doc.block_text(b);
    let ctx = format!("{ctx} block {text:?} x={x} {m:?}");

    // Nothing over text: the block's first visible character is where the
    // text column starts. (Code paints from its own content.)
    let first_char = text.chars().next().unwrap().to_string();
    assert_eq!(sym(buf, x, y0), first_char, "text overwritten: {ctx}");

    let folds: Vec<_> = targets
        .as_slice()
        .iter()
        .filter(|t| matches!(t.action, Action::FoldAt(_)) && t.zone.y == y0)
        .collect();

    if x == 0 {
        // No margin: no marks, and no fold target on top of the text.
        assert!(folds.is_empty(), "fold target over text: {ctx}");
        return;
    }
    let s1 = x - 1;
    let bar = m.changed || m.focused;

    // The change, on every content row.
    if m.changed {
        for r in 0..rows {
            let y = y0 + r;
            let s = sym(buf, s1, y);
            assert!(
                s == "▎" || reversed(buf, s1, y),
                "change lost on row {r} ({s:?}): {ctx}"
            );
        }
    }
    // The cursor's bar, on every row a point mark does not need.
    if m.focused {
        for r in 1..rows {
            assert_eq!(sym(buf, s1, y0 + r), "┃", "focus row {r}: {ctx}");
        }
    }
    // A bar shared with another state is cut, never dropped.
    if m.changed && m.focused {
        for r in 0..rows {
            assert!(reversed(buf, s1, y0 + r), "focus+change row {r}: {ctx}");
        }
    }

    // The bookmark keeps the state column's first-row cell.
    if m.bookmarked {
        assert_eq!(sym(buf, s1, y0), "▪", "bookmark lost: {ctx}");
        assert_eq!(reversed(buf, s1, y0), bar, "bookmark cut by bar: {ctx}");
    }

    // The fold marker: the fold column when there is one; the state column
    // when that is all there is and no bookmark needs it.
    let fold_at = if x >= 2 {
        Some(x - 2)
    } else if m.bookmarked {
        None
    } else {
        Some(s1)
    };
    if m.folded {
        match fold_at {
            Some(fx) => {
                assert_eq!(sym(buf, fx, y0), "▸", "fold marker lost: {ctx}");
                if fx == s1 {
                    assert_eq!(reversed(buf, fx, y0), bar, "fold cut by bar: {ctx}");
                }
                assert_eq!(folds.len(), 1, "one fold target: {ctx}");
                assert_eq!((folds[0].zone.x, folds[0].zone.w), (fx, 1), "{ctx}");
            }
            None => assert!(folds.is_empty(), "a target with no marker: {ctx}"),
        }
        // The fold state shows in the row regardless.
        let row: String = (x..x + 20).map(|c| sym(buf, c, y0)).collect();
        assert!(row.contains('…'), "no ellipsis: {ctx} {row:?}");
    } else {
        assert!(folds.is_empty(), "stray fold target: {ctx}");
    }
    // Every fold target sits on a painted marker.
    for t in &folds {
        let s = sym(buf, t.zone.x, t.zone.y);
        assert!(s == "▸" || s == "▾", "target on {s:?}: {ctx}");
    }

    // With no marks at all, the margin is blank.
    if !bar && !m.bookmarked && !m.folded {
        assert_eq!(sym(buf, s1, y0), " ", "stray mark: {ctx}");
    }
}

fn apply(app: &mut App, b: BlockIdx, m: Marks) {
    if m.changed {
        app.changed.push(b);
        app.changed.sort();
    }
    if m.bookmarked {
        app.marks.push(app.doc.node_for_block(b).doc.start);
    }
    if m.folded {
        let at = app.doc.node_for_block(b).doc.start;
        update(app, Action::FoldAt(at));
    }
    if m.focused {
        app.code_focus = Some(b);
    }
}

/// Widths giving a left margin of 0, 1, 2 and 4 cells for prose.
const WIDTHS: [u16; 4] = [19, 40, 80, 100];

#[test]
fn every_mark_survives_every_combination_at_every_margin() {
    for cols in WIDTHS {
        for mask in 0u8..8 {
            let bit = |i: u8| mask & (1 << i) != 0;
            let mut app = app_at(cols);
            let head = block(&app, "Tail");
            let short = block(&app, "Short");
            let long = block(&app, "A paragraph");
            let code = block(&app, "code one");
            let heading = Marks {
                changed: bit(0),
                bookmarked: bit(1),
                folded: bit(2),
                focused: false,
            };
            let prose = Marks {
                folded: false,
                ..heading
            };
            let code_marks = Marks {
                folded: false,
                focused: bit(2),
                ..heading
            };
            apply(&mut app, head, heading);
            apply(&mut app, short, prose);
            apply(&mut app, long, prose);
            apply(&mut app, code, code_marks);
            assert!(app.layout.content_height(&app.doc, long) > 1, "{cols}");
            let (buf, targets) = frame(&app, cols);
            let ctx = format!("cols={cols} mask={mask:03b}");
            check(&app, &buf, &targets, head, heading, &ctx);
            check(&app, &buf, &targets, short, prose, &ctx);
            check(&app, &buf, &targets, long, prose, &ctx);
            check(&app, &buf, &targets, code, code_marks, &ctx);
        }
    }
}

#[test]
fn the_margins_these_widths_give_are_the_ones_named() {
    let margins: Vec<u16> = WIDTHS
        .iter()
        .map(|&c| {
            let app = app_at(c);
            app.block_span_x(block(&app, "Short")).0
        })
        .collect();
    assert_eq!(margins, [0, 1, 2, 4]);
}

#[test]
fn clicking_the_fold_marker_in_a_one_cell_margin_still_unfolds_a_changed_heading() {
    let mut app = app_at(40);
    let head = block(&app, "Tail");
    app.changed.push(head);
    let at = app.doc.node_for_block(head).doc.start;
    update(&mut app, Action::FoldAt(at));
    let id = app.doc.node_for_block(head).id;
    assert!(app.folded.contains(&id));
    let (buf, targets) = frame(&app, 40);
    let y = first_y(&app, head);
    let hit = targets.hit(0, y).expect("the marker is a target");
    assert_eq!(sym(&buf, 0, y), "▸");
    assert!(reversed(&buf, 0, y), "and wears the change");
    update(&mut app, hit.action);
    assert!(!app.folded.contains(&id), "the click unfolded it");
}

#[test]
fn an_open_summary_marker_follows_the_same_rule() {
    // `▾` is always on a `<details>` summary, so it meets the change bar and
    // the bookmark far more often than a folded heading's `▸` does.
    let src =
        "Intro.\n\n<details>\n<summary>The fine print</summary>\n\nhidden words\n\n</details>\n";
    for (cols, bookmarked) in [(40u16, false), (40, true), (80, false), (80, true)] {
        let mut app = App::new("t.md".into(), Document::parse(src), cols, ROWS);
        app.breadcrumb = false;
        app.on_resize(cols, ROWS);
        let summary = block(&app, "The fine print");
        app.changed.push(summary);
        if bookmarked {
            app.marks.push(app.doc.node_for_block(summary).doc.start);
        }
        let (buf, targets) = frame(&app, cols);
        let (x, _) = app.block_span_x(summary);
        let y = first_y(&app, summary);
        let ctx = format!("cols={cols} bookmarked={bookmarked} x={x}");
        assert!(
            reversed(&buf, x - 1, y) || sym(&buf, x - 1, y) == "▎",
            "change lost: {ctx}"
        );
        let fold: Vec<_> = targets
            .as_slice()
            .iter()
            .filter(|t| matches!(t.action, Action::FoldAt(_)))
            .collect();
        let expect = if x >= 2 {
            Some(x - 2)
        } else if bookmarked {
            None
        } else {
            Some(x - 1)
        };
        if let Some(fx) = expect {
            assert_eq!(sym(&buf, fx, y), "▾", "{ctx}");
            assert_eq!(fold.len(), 1, "{ctx}");
            assert_eq!((fold[0].zone.x, fold[0].zone.y), (fx, y), "{ctx}");
        } else {
            assert_eq!(sym(&buf, x - 1, y), "▪", "{ctx}");
            assert!(fold.is_empty(), "{ctx}");
        }
    }
}
