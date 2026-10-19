//! A README's centring `<div>` takes no rows, and the top of a document is
//! the same top whether you arrive by opening it or by `gg`.
use carrel::{
    action::Action,
    app::{App, update},
    render,
};
use carrel_core::Document;
use ratatui::{Terminal, backend::TestBackend};

const README_HEAD: &str = "<div align=\"center\">\n\n\
<img src=\"logo.png\" alt=\"Carrel\" width=\"220\">\n\n\
# carrel\n\n**A quiet place to read your markdown.**\n\n</div>\n\n\
> **carrel** *(n.)* — a small enclosure with a desk.\n\n\
one\n\ntwo\n\nthree\n\nfour\n\nfive\n\nsix\n\nseven\n\neight\n\nnine\n\nten\n";

fn top_line(app: &App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(app.cols, app.rows)).unwrap();
    terminal.draw(|f| render::draw(f, app)).unwrap();
    let buf = terminal.backend().buffer();
    let y = app.text_y();
    (0..app.cols).map(|x| buf[(x, y)].symbol()).collect()
}

#[test]
fn the_top_is_the_same_by_opening_and_by_gg() {
    for src in [
        README_HEAD,
        "-\n\nx\n\n- a\n- b\n- c\n- d\n- e\n- f\n- g\n- h\n",
    ] {
        let mut app = App::new("t.md".into(), Document::parse(src), 40, 10);
        app.breadcrumb = false;
        app.on_resize(40, 10);
        let opened = (app.view.scroll_row, top_line(&app));
        update(&mut app, Action::GoToEnd);
        assert_ne!(app.view.scroll_row, 0, "the fixture must scroll");
        update(&mut app, Action::GoToStart);
        let gg = (app.view.scroll_row, top_line(&app));
        assert_eq!(opened, gg, "{src:?}");
        // And a resize at the top keeps it there.
        app.on_resize(36, 10);
        assert_eq!(app.view.scroll_row, 0, "{src:?}");
    }
}

#[test]
fn a_readme_opens_on_its_first_words_not_on_blank_rows() {
    let mut app = App::new("README.md".into(), Document::parse(README_HEAD), 40, 10);
    app.breadcrumb = false;
    app.on_resize(40, 10);
    update(&mut app, Action::GoToEnd);
    update(&mut app, Action::GoToStart);
    assert_eq!(
        top_line(&app).trim_end_matches(['█', '│', ' ']).trim(),
        "Carrel"
    );
}
