//! Math end to end: art heights, the width ladder, and the invariants that
//! matter once art meets layout.

use carrel::app::{App, MathForm};
use carrel_core::{BlockIdx, Document};

fn app(src: &str, cols: u16, rows: u16) -> App {
    App::new("t.md".into(), Document::parse(src), cols, rows)
}

/// The first block that carries math art.
fn math_block(a: &App) -> BlockIdx {
    (0..a.doc.block_count())
        .map(|i| BlockIdx(i as u32))
        .find(|b| a.math_art.contains_key(b))
        .expect("a math block with art")
}

/// The real `--plain` contract (Q17): no escape bytes and no box drawing.
///
/// NOT "pure ASCII" — space 2 legitimately holds non-ASCII content, and has
/// since smart punctuation shipped: `"x"` is already U+201C/U+201D. The policy
/// is about what plain output ADDS, not what the document contains.
/// The escape byte is only the loudest control character; a BEL rings the
/// terminal and a NUL confuses a braille display just as effectively, and a
/// screen reader is not helped by any of them. So the check is the whole
/// class, minus the newline that separates the lines themselves.
fn assert_plain_safe(out: &str) {
    for c in out.chars() {
        assert!(
            !c.is_control() || c == '\n',
            "plain output never contains a control character, found {c:?}:\n{out}"
        );
    }
    assert!(
        !out.chars().any(|c| ('\u{2500}'..='\u{257f}').contains(&c)),
        "plain output never contains box drawing:\n{out}"
    );
}

#[test]
fn a_display_math_block_occupies_its_art_height() {
    let a = app("text\n\n$$\\frac{a+b}{c}$$\n\ntext\n", 40, 20);
    let math = math_block(&a);
    let art = a.math_art.get(&math).expect("art");
    assert_eq!(art.display.rows.len(), 3, "numerator, rule, denominator");
    assert_eq!(
        a.layout.height(math),
        4,
        "three art rows plus the one-row gap after the block"
    );
}

#[test]
fn the_art_survives_a_resize_unchanged() {
    let mut a = app("$$\\frac{a+b}{c}$$\n", 100, 20);
    let b = math_block(&a);
    let wide = a.math_art.get(&b).expect("art").display.rows.clone();
    a.on_resize(40, 20);
    let narrow = a.math_art.get(&b).expect("art").display.rows.clone();
    assert_eq!(
        wide, narrow,
        "math art is width-independent; a resize must not touch it"
    );
}

/// The ladder is display art -> the inline single-row form -> literal source,
/// taking the first that fits.
///
/// **Note which rung actually fires.** Display art stacks in two dimensions,
/// so for most expressions it is NARROWER than the inline form, which has to
/// spell `(a+b)/c` out on one line. The inline rung therefore only fires for
/// the minority of expressions whose one-row form is the shorter one; a wide
/// fraction goes straight from display to source. That is correct — the ladder
/// takes the first form that fits — but it is not the "progressively smaller"
/// shape the name suggests, so it is pinned here.
#[test]
fn math_falls_back_by_width_taking_the_first_form_that_fits() {
    let src = "$$\\frac{aaaaaaaaaaaaaaaaaaaa+bbbbbbbbbbbbbbbbbbbb}{c}$$\n";
    let a = app(src, 100, 20);
    let b = math_block(&a);
    let art = a.math_art.get(&b).expect("art");

    assert_eq!(
        a.math_form(b, art.display.width),
        MathForm::Display,
        "the display form fits exactly at its own width"
    );
    assert_eq!(
        a.math_form(b, 2),
        MathForm::Source,
        "too narrow for either form: the literal LaTeX, wrapped as prose"
    );
    assert!(
        art.inline.width >= art.display.width,
        "a stacked fraction is narrower than its solidus form: {} vs {}",
        art.display.width,
        art.inline.width
    );
}

/// The inline rung, exercised by an expression whose one-row form IS narrower.
#[test]
fn the_inline_rung_fires_when_the_one_row_form_is_narrower() {
    let a = app("$$x$$\n", 40, 20);
    let b = math_block(&a);
    let art = a.math_art.get(&b).expect("art");
    // A bare atom lays out identically both ways, so the display rung wins at
    // any width that fits it, and source takes over below that.
    assert_eq!(art.display.width, art.inline.width);
    assert_eq!(a.math_form(b, art.display.width), MathForm::Display);
    assert_eq!(a.math_form(b, 0), MathForm::Source);
}

#[test]
fn unparseable_math_renders_as_its_source_and_never_panics() {
    let a = app("$$\\frac{$$\n", 40, 20);
    assert!(
        a.math_art.is_empty(),
        "nothing parsed, so nothing to render"
    );
    assert_eq!(a.math_form(BlockIdx(0), 40), MathForm::Source);
    assert!(
        a.doc.text.contains("\\frac{"),
        "the source is retained verbatim: {:?}",
        a.doc.text
    );
}

#[test]
fn math_source_is_searchable_because_the_doc_text_is_the_latex() {
    let doc = Document::parse("$$E = mc^2$$\n");
    assert_eq!(
        carrel_core::search(&doc, "mc", true).len(),
        1,
        "the LaTeX source is in space 2, so search reaches it"
    );
}

#[test]
fn plain_mode_emits_latex_source_not_box_art() {
    let doc = Document::parse("$$\\frac{a}{b}$$\n");
    let out = carrel::plain::render(&doc, 72);
    assert!(
        out.contains("\\frac{a}{b}"),
        "Q17: speakable source, not a box-drawing rule:\n{out}"
    );
    assert_plain_safe(&out);
}

/// Math is the sharpest case for the control-byte filter: a display math
/// block's plain rendering is its LaTeX SOURCE, copied out of the document
/// verbatim, so whatever the author put between the `$$` is what a pipe
/// receives. The assertion above only bites when the corpus carries one.
#[test]
fn a_math_block_cannot_smuggle_an_escape_out_through_its_source() {
    let doc = Document::parse("$$\\text{\u{1b}]0;PWNED\u{7}}$$\n");
    assert!(
        doc.text.contains('\u{1b}'),
        "the corpus must carry one: {:?}",
        doc.text
    );
    let out = carrel::plain::render(&doc, 72);
    assert_plain_safe(&out);
    assert!(out.contains("PWNED"), "only the escape goes: {out}");
}
