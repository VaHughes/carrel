//! The card view's legend keeps its `·` separators on every row it wraps to.
//!
//! The legend is the header line wrapped like prose. Its separators are
//! paint-time decoration over the synthetic gap between two header cells, so
//! a wrapped row has to find its gaps by byte, not by counting columns from
//! the start of the line — which only the first row begins at.
use carrel::{app::App, render};
use carrel_core::Document;
use ratatui::{Terminal, backend::TestBackend};

fn screen(md: &str, cols: u16, rows: u16) -> Vec<String> {
    let mut app = App::new("t.md".into(), Document::parse(md), cols, rows);
    app.breadcrumb = false;
    app.on_resize(cols, rows);
    let mut terminal = Terminal::new(TestBackend::new(cols, rows)).unwrap();
    terminal.draw(|f| render::draw(f, &app)).unwrap();
    let buf = terminal.backend().buffer();
    (0..rows)
        .map(|y| (0..cols).map(|x| buf[(x, y)].symbol()).collect())
        .collect()
}

/// The legend: every row above the first card rule, minus the scrollbar.
fn legend(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .skip_while(|l| l.trim().is_empty())
        .take_while(|l| !l.contains('─'))
        .map(|l| l.trim_end_matches(['█', '│', ' ']).trim().to_string())
        .collect()
}

const LATIN: &str = "| alpha | beta | gamma | delta | epsilon | zeta |\n\
|---|---|---|---|---|---|\n| a | b | c | d | e | f |\n";

#[test]
fn a_wrapped_legend_row_keeps_its_separators() {
    let rows = legend(&screen(LATIN, 28, 14));
    assert_eq!(
        rows,
        ["alpha · beta · gamma", "delta · epsilon · zeta"],
        "{rows:?}"
    );
}

#[test]
fn a_wrapped_cjk_legend_puts_its_separators_in_the_gaps() {
    let md = "| 名前 | 都市 | 国家 | 言語 | 通貨 |\n|---|---|---|---|---|\n\
| 甲 | 乙 | 丙 | 丁 | 戊 |\n";
    let rows = legend(&screen(md, 20, 14));
    assert!(rows.len() > 1, "the legend wraps: {rows:?}");
    // The row break falls inside `国家` (CJK breaks between ideographs), so
    // all four gaps are on screen, and each carries a separator with a blank
    // on either side.
    assert!(rows[1].starts_with('家'), "{rows:?}");
    let seps: usize = rows.iter().map(|r| r.matches('·').count()).sum();
    assert_eq!(seps, 4, "{rows:?}");
    for r in &rows {
        assert!(!r.contains("··") && !r.contains(" · ·"), "{rows:?}");
        for (i, _) in r.match_indices('·') {
            assert!(r[..i].ends_with(' ') && r[i + '·'.len_utf8()..].starts_with(' '));
        }
    }
}

#[test]
fn a_separator_never_dangles_at_a_row_break() {
    // So narrow that every header cell is a row of its own: there is no
    // cell on either side of any gap, so nothing to separate.
    let md = "| alpha | beta | gamma | delta |\n|---|---|---|---|\n| a | b | c | d |\n";
    let rows = legend(&screen(md, 12, 14));
    assert_eq!(rows, ["alpha", "beta", "gamma", "delta"], "{rows:?}");
}
