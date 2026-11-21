//! Math holding a no-break space, an em space or an ideographic space.
//!
//! The LaTeX parser's lexer panics on one of these straight after a number
//! or a unit it is reading as a length (`\kern 1pt` and a no-break space):
//! it steps over "one whitespace character" as one byte. The guard for that
//! once refused any formula that had such a space after a letter or digit
//! anywhere and a row break, a `kern` or an `align` anywhere else — which is
//! most matrices and aligned blocks pasted from a web page. Every formula
//! here rendered before that guard existed, and renders.

use carrel_core::{Document, math};

/// `S` stands for the space under test.
const FORMULAS: &[&str] = &[
    r"\begin{aligned} aS&= b \\ c &= d \end{aligned}",
    r"\begin{align} aS&= b \end{align}",
    r"\begin{cases} xS& y \\ z & w \end{cases}",
    r"\begin{pmatrix} aS& b \\ c & d \end{pmatrix}",
    r"\begin{bmatrix} 1S& 0 \\ 0 & 1 \end{bmatrix}",
    r"\begin{vmatrix} aS& b \\ c & d \end{vmatrix}",
    r"\begin{array}{cc} aS& b \\ c & d \end{array}",
    r"\begin{gathered} aS= b \\ c = d \end{gathered}",
    r"\begin{split} aS&= b \\ &= c \end{split}",
    r"\begin{aligned} x &= 1S\text{m} \\ y &= 2S\text{kg} \end{aligned}",
    r"\begin{cases} 1 & \text{if }xS> 0 \\ 0 & \text{otherwiseSthen} \end{cases}",
    r"\text{skipSit}",
    r"\text{aSb}\hspace{1em}c",
    r"\text{alignSthe kernS1pt} + x",
    r"5S\text{kg}",
    r"v = 3S\mathrm{m}/\mathrm{s}",
    r"\hspace{1em}Sx",
    r"\operatorname{kernel}Sx",
    r"f(x)S=S\frac{1}{2}Sx^2",
    r"\frac{aS}{bS}",
    r"\sqrt{xS+ 1}",
    r"\sqrt[3]{xS+ 1}",
    r"\sum_{iS= 1}^{nS} i",
    r"\left( aS\right)",
    r"\mathbf{x}S\cdot \mathbf{y}S",
    r"\underbrace{aS+ b}_{\text{sumShere}}",
    r"\alpha 2S\beta",
    r"E = mcS^2",
    r"aS\quad b",
    r"\overline{aS+ b}S= c",
];

const SPACES: [char; 3] = ['\u{a0}', '\u{2003}', '\u{3000}'];

#[test]
fn math_with_a_wide_space_in_it_renders() {
    let mut count = 0;
    for formula in FORMULAS {
        for space in SPACES {
            let formula = formula.replace('S', &space.to_string());
            assert!(math::parse(&formula).is_some(), "{formula:?}");
            count += 1;
        }
    }
    assert!(count >= 60);
}

/// The space is the author's where it is text: only a length's lexer is
/// kept from it.
#[test]
fn text_in_math_keeps_its_no_break_space() {
    for (formula, shown) in [
        ("\\text{skip\u{a0}it}", "skip\u{a0}it"),
        ("\\text{5\u{a0}kg} + \\hspace{1em}x", "5\u{a0}kg"),
        ("\\text{a\u{3000}b}\\hspace{1em}c", "a\u{3000}b"),
        ("\\operatorname{arg\u{a0}max}\\kern 1pt x", "arg\u{a0}max"),
    ] {
        let doc = Document::parse(&format!("a ${formula}$ z\n"));
        assert!(doc.text.contains(shown), "{formula:?}: {:?}", doc.text);
        assert!(!doc.text.contains('\\'), "shown as source: {:?}", doc.text);
    }
}

/// What panicked the lexer renders too, the space read as the plain one
/// TeX would take it for.
#[test]
fn a_wide_space_after_a_length_is_a_space() {
    for (wide, plain) in [
        ("a\\kern 1pt\u{a0}x", "a\\kern 1pt x"),
        ("a\\hspace{1em\u{2003}}x", "a\\hspace{1em }x"),
        (
            "\\begin{array}{c} a \\\\[1pt\u{a0}] b \\end{array}",
            "\\begin{array}{c} a \\\\[1pt ] b \\end{array}",
        ),
        (
            "\\genfrac{}{}{1pt\u{3000}}{}{a}{b}",
            "\\genfrac{}{}{1pt }{}{a}{b}",
        ),
        (
            "\\begin{alignat}{2\u{a0}} a &= b \\end{alignat}",
            "\\begin{alignat}{2 } a &= b \\end{alignat}",
        ),
    ] {
        let plain = math::parse(plain);
        assert!(plain.is_some(), "{plain:?}");
        assert_eq!(math::parse(wide), plain, "{wide:?}");
    }
}
