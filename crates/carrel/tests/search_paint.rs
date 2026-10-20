//! Painting search hits costs what is on screen, not what is on screen
//! times the hits on it.

use std::time::Instant;

use carrel::action::{Action, Direction, SearchKey};
use carrel::app::{App, update};
use carrel_core::Document;
use ratatui::{Terminal, backend::TestBackend};

/// A frame of `clusters` letters, each carrying `marks` combining acutes,
/// with the prompt open on a search for the mark itself: every mark is a
/// hit. A row is 100 CELLS whatever `marks` is, so its bytes and its hits
/// both grow with `marks`.
fn searched(clusters: usize, marks: usize) -> App {
    let letter = format!("x{}", "\u{301}".repeat(marks));
    let src = format!("{}\n", letter.repeat(clusters));
    let mut app = App::new("marks.md".into(), Document::parse(&src), 100, 12);
    app.on_resize(100, 12);
    update(&mut app, Action::SearchOpen(Direction::Forward));
    update(&mut app, Action::SearchKey(SearchKey::Char('\u{301}')));
    assert_eq!(
        app.matches.as_ref().map(carrel_core::Matches::len),
        Some(clusters * marks),
        "every mark is a hit"
    );
    app
}

fn paint_secs(app: &App) -> f64 {
    let mut term = Terminal::new(TestBackend::new(100, 12)).unwrap();
    let mut p = carrel::render::Painted::default();
    let t = Instant::now();
    term.draw(|f| {
        carrel::render::draw_full(f, app, &mut p, &mut std::collections::HashMap::new());
    })
    .unwrap();
    t.elapsed().as_secs_f64()
}

/// Each hit was placed by walking its row from the first cluster, so a row
/// cost its bytes times its hits — and a row is bounded in cells, not in
/// bytes. A giant token searched from its end (the review's case: 100,000
/// `x`, `G`, `/x`) froze the reader for 16 s once rows grew to 34 KB; a
/// letter heavy with combining marks gets there with no layout bug at all.
/// Four times the marks is four times the bytes and four times the hits:
/// linear is ~4x, the walk per hit was ~16x.
#[test]
fn painting_a_rows_hits_is_linear_in_the_row() {
    let small = searched(500, 10);
    let large = searched(500, 40);
    paint_secs(&small);
    paint_secs(&large);
    let (a, b) = (paint_secs(&small), paint_secs(&large));
    assert!(
        b < a * 10.0,
        "4x the marks took {:.1}x the time to paint ({a:.4}s -> {b:.4}s). \
         Linear is ~4x; a walk per hit is ~16x.",
        b / a
    );
}

/// The review's case itself: one giant token, `G`, then `/x`. Every `x` is
/// a hit and the view sits at the token's end. Rows of a run wider than
/// 65,535 cells once overflowed a `u16` and ~34 KB landed on one row; with
/// a walk per hit, that row cost 34,000 walks of 34 KB.
#[test]
fn a_giant_token_searched_from_its_end_paints_in_linear_time() {
    let at_end = |n: usize| {
        let src = format!("# W\n\n{} end\n\nafter\n", "x".repeat(n));
        let mut app = App::new("big.md".into(), Document::parse(&src), 100, 12);
        app.on_resize(100, 12);
        update(&mut app, Action::GoToEnd);
        update(&mut app, Action::SearchOpen(Direction::Forward));
        update(&mut app, Action::SearchKey(SearchKey::Char('x')));
        assert_eq!(app.matches.as_ref().map(carrel_core::Matches::len), Some(n));
        app
    };
    let (small, large) = (at_end(25_000), at_end(100_000));
    paint_secs(&small);
    paint_secs(&large);
    let (a, b) = (paint_secs(&small), paint_secs(&large));
    assert!(
        b < a * 10.0,
        "4x the token took {:.1}x the time to paint ({a:.4}s -> {b:.4}s). Linear is ~4x.",
        b / a
    );
}
