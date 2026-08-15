//! `MathExpr` → box art. TeX-lite: every node becomes a box with a baseline,
//! and composition aligns baselines.
//!
//! **This module never sees LaTeX.** It takes the parsed tree, which is what
//! makes it testable against hand-built expressions — the same seam that makes
//! `pack.rs` testable against hand-built break units. Every test below builds
//! its own `MathExpr`; not one of them mentions a backslash.
//!
//! Art is **width-independent**: the same expression lays out identically at
//! any terminal width. Width only selects between [`Mode::Display`] and
//! [`Mode::Inline`], and past that the literal-source fallback — see
//! `App::math_form`. That is the project's central invariant in math's terms,
//! and `art_is_the_same_at_every_width` exists to make a violation loud.
//!
//! NO RATATUI — this is pure text, and `scripts/check-discipline.sh` rule 6
//! keeps it that way.

use carrel_core::{MathClass, MathExpr, MatrixDelim, cluster_width};

/// A laid-out expression: rectangular text with a baseline row.
///
/// Every row is exactly `width` display cells, so paint can write them without
/// measuring. `baseline` is always a valid index into `rows`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MathBox {
    pub rows: Vec<String>,
    pub baseline: usize,
    pub width: u16,
}

/// Display math may grow tall; inline math is pinned to one row because a
/// paragraph row is one row.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Mode {
    Display,
    Inline,
}

impl MathBox {
    fn sym(s: &str) -> Self {
        Self {
            rows: vec![s.to_string()],
            baseline: 0,
            width: cluster_width(s),
        }
    }

    fn blank(width: u16) -> Self {
        Self {
            rows: vec![" ".repeat(width as usize)],
            baseline: 0,
            width,
        }
    }

    fn height(&self) -> usize {
        self.rows.len()
    }

    /// Rows above the baseline.
    fn ascent(&self) -> usize {
        self.baseline
    }

    /// Rows at and below the baseline.
    fn descent(&self) -> usize {
        self.rows.len() - self.baseline
    }

    /// Pad every row to `width`, centring the existing content.
    fn centred(mut self, width: u16) -> Self {
        if width <= self.width {
            return self;
        }
        let total = usize::from(width - self.width);
        let left = total / 2;
        let right = total - left;
        for row in &mut self.rows {
            let mut s = " ".repeat(left);
            s.push_str(row);
            s.push_str(&" ".repeat(right));
            *row = s;
        }
        self.width = width;
        self
    }

    /// Horizontal concatenation with baselines aligned.
    ///
    /// Height is `max(ascent) + max(descent)`; each operand is padded with
    /// blank rows above and below so the result stays rectangular. Ragged rows
    /// would break paint, which writes rows without measuring them.
    ///
    /// By value, not by reference, because a `Row` folds with
    /// `.reduce(MathBox::beside)`, and `reduce` requires `(Self, Self) -> Self`.
    #[allow(clippy::needless_pass_by_value)]
    fn beside(self, other: Self) -> Self {
        let ascent = self.ascent().max(other.ascent());
        let descent = self.descent().max(other.descent());
        let width = self.width.saturating_add(other.width);
        let mut rows = Vec::with_capacity(ascent + descent);
        for i in 0..ascent + descent {
            let mut row = String::new();
            row.push_str(&pick(&self, i, ascent));
            row.push_str(&pick(&other, i, ascent));
            rows.push(row);
        }
        Self {
            rows,
            baseline: ascent,
            width,
        }
    }

    /// Stack `self` above `below`, with `self`'s rows first. The baseline is
    /// given explicitly because only the caller knows which row it is.
    fn stack(boxes: Vec<Self>, baseline: usize) -> Self {
        let width = boxes.iter().map(|b| b.width).max().unwrap_or(0);
        let mut rows = Vec::new();
        for b in boxes {
            rows.extend(b.centred(width).rows);
        }
        let baseline = baseline.min(rows.len().saturating_sub(1));
        Self {
            rows,
            baseline,
            width,
        }
    }
}

/// The row of `b` that belongs at output row `i`, given the output ascent.
/// Rows outside the box are blank padding.
fn pick(b: &MathBox, i: usize, ascent: usize) -> String {
    let top = ascent - b.ascent();
    if i < top || i >= top + b.height() {
        " ".repeat(b.width as usize)
    } else {
        b.rows[i - top].clone()
    }
}

/// Lay an expression out as box art.
///
/// Takes no width: art is a function of the expression alone. See the module
/// docs for why that matters.
#[must_use]
pub fn lay_out(expr: &MathExpr, mode: Mode) -> MathBox {
    match expr {
        MathExpr::Sym { text, class } => {
            // A binary operator or relation breathes; an ordinary atom does
            // not, or `abc` would render as `a b c`.
            let spaced = matches!(class, MathClass::BinaryOp | MathClass::Relation);
            if spaced && !text.is_empty() {
                MathBox::sym(&format!(" {text} "))
            } else {
                MathBox::sym(text)
            }
        }
        MathExpr::Row(parts) => parts
            .iter()
            .map(|p| lay_out(p, mode))
            .reduce(MathBox::beside)
            .unwrap_or_else(|| MathBox::sym("")),
        MathExpr::Frac { num, den } => frac(num, den, mode),
        MathExpr::Sqrt { radicand, index } => sqrt(radicand, index.as_deref(), mode),
        MathExpr::Script {
            base,
            sub,
            sup,
            limits,
        } => script(base, sub.as_deref(), sup.as_deref(), *limits, mode),
        MathExpr::Matrix { rows, delim } => matrix(rows, *delim, mode),
    }
}

fn frac(num: &MathExpr, den: &MathExpr, mode: Mode) -> MathBox {
    let n = lay_out(num, mode);
    let d = lay_out(den, mode);
    if mode == Mode::Inline {
        // One row only, so a stacked rule is not available: fall back to a
        // solidus, parenthesising either operand that would otherwise change
        // meaning. `(a+b)/c` and `a/b` are both unambiguous; `a+b/c` is not.
        let ntext = paren_if_compound(num, &n);
        let dtext = paren_if_compound(den, &d);
        return MathBox::sym(&format!("{ntext}/{dtext}"));
    }
    // A fraction whose own operand is a fraction needs a visibly longer rule,
    // or the two stack into one ambiguous ladder: TeX distinguishes the levels
    // by rule length and so must we.
    let nested = matches!(num, MathExpr::Frac { .. }) || matches!(den, MathExpr::Frac { .. });
    let width = n
        .width
        .max(d.width)
        .saturating_add(if nested { 2 } else { 0 });
    let rule = MathBox::sym(&"─".repeat(width as usize));
    let baseline = n.height();
    MathBox::stack(vec![n, rule, d], baseline)
}

fn sqrt(radicand: &MathExpr, index: Option<&MathExpr>, mode: Mode) -> MathBox {
    let r = lay_out(radicand, mode);
    if mode == Mode::Inline {
        let inner = if radicand.is_compound() {
            format!("({})", one_row(&r))
        } else {
            one_row(&r)
        };
        let idx = index.map_or_else(String::new, |i| one_row(&lay_out(i, mode)));
        return MathBox::sym(&format!("{idx}√{inner}"));
    }
    // The overbar spans the radicand; the radical sign sits on the baseline.
    let bar = MathBox::sym(&format!(" {}", "‾".repeat(r.width as usize)));
    let sign = MathBox::sym("√").beside(r);
    let baseline = bar.height() + sign.baseline;
    MathBox::stack(vec![bar, sign], baseline)
}

fn script(
    base: &MathExpr,
    sub: Option<&MathExpr>,
    sup: Option<&MathExpr>,
    limits: bool,
    mode: Mode,
) -> MathBox {
    let b = lay_out(base, mode);

    if limits && mode == Mode::Display {
        // A big operator carries its limits above and below, centred.
        let mut boxes = Vec::new();
        let mut baseline = 0;
        if let Some(s) = sup {
            let sb = lay_out(s, mode);
            baseline += sb.height();
            boxes.push(sb);
        }
        boxes.push(b);
        if let Some(s) = sub {
            boxes.push(lay_out(s, mode));
        }
        return MathBox::stack(boxes, baseline);
    }

    // Prefer the Unicode forms: they keep the expression one row tall, which
    // is required inline and simply nicer in display.
    let raised = sup.and_then(|s| unicode_script(s, SUPERS));
    let lowered = sub.and_then(|s| unicode_script(s, SUBS));
    if (sup.is_none() || raised.is_some()) && (sub.is_none() || lowered.is_some()) {
        let mut out = one_row(&b);
        if let Some(s) = lowered {
            out.push_str(&s);
        }
        if let Some(s) = raised {
            out.push_str(&s);
        }
        return MathBox::sym(&out);
    }

    if mode == Mode::Inline {
        let mut out = one_row(&b);
        if let Some(s) = sub {
            let sb = lay_out(s, mode);
            out.push('_');
            out.push_str(&wrap_if_compound(s, &sb));
        }
        if let Some(s) = sup {
            let sb = lay_out(s, mode);
            out.push('^');
            out.push_str(&wrap_if_compound(s, &sb));
        }
        return MathBox::sym(&out);
    }

    // Display: raise a real box for the superscript, lower one for the
    // subscript, both beside the base.
    let mut boxes = Vec::new();
    let mut baseline = 0;
    if let Some(s) = sup {
        let sb = lay_out(s, mode);
        baseline += sb.height();
        boxes.push(sb);
    }
    boxes.push(MathBox::blank(0));
    if let Some(s) = sub {
        boxes.push(lay_out(s, mode));
    }
    let scripts = MathBox::stack(boxes, baseline);
    // Align the base's baseline with the script stack's blank middle row.
    let lifted = MathBox::stack(
        vec![MathBox::blank(b.width); baseline]
            .into_iter()
            .chain(std::iter::once(b))
            .collect(),
        baseline,
    );
    lifted.beside(scripts)
}

fn matrix(rows: &[Box<[MathExpr]>], delim: MatrixDelim, mode: Mode) -> MathBox {
    let laid: Vec<Vec<MathBox>> = rows
        .iter()
        .map(|r| r.iter().map(|c| lay_out(c, mode)).collect())
        .collect();
    let cols = laid.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<u16> = (0..cols)
        .map(|c| {
            laid.iter()
                .filter_map(|r| r.get(c))
                .map(|b| b.width)
                .max()
                .unwrap_or(0)
        })
        .collect();

    if mode == Mode::Inline {
        let body = laid
            .iter()
            .map(|r| r.iter().map(one_row).collect::<Vec<_>>().join(" "))
            .collect::<Vec<_>>()
            .join("; ");
        let (l, r) = inline_fences(delim);
        return MathBox::sym(&format!("{l}{body}{r}"));
    }

    // Each grid row is its cells side by side, padded to the column widths.
    let mut body: Vec<MathBox> = Vec::new();
    for row in laid {
        let mut acc: Option<MathBox> = None;
        for (c, cell) in row.into_iter().enumerate() {
            let padded = cell.centred(widths[c]);
            let piece = match acc {
                None => padded,
                Some(a) => a.beside(MathBox::sym(" ")).beside(padded),
            };
            acc = Some(piece);
        }
        if let Some(a) = acc {
            body.push(a);
        }
    }
    let height: usize = body.iter().map(MathBox::height).sum();
    let grid = MathBox::stack(body, height / 2);
    fence(grid, delim)
}

/// Wrap a grid in stretched delimiters. Two-row grids use the two-piece forms;
/// taller ones get a middle piece repeated.
fn fence(grid: MathBox, delim: MatrixDelim) -> MathBox {
    let Some((top, mid, bot)) = fence_pieces(delim) else {
        return grid;
    };
    let h = grid.height();
    let left: Vec<String> = (0..h)
        .map(|i| {
            if i == 0 {
                top.0
            } else if i + 1 == h {
                bot.0
            } else {
                mid.0
            }
            .to_string()
        })
        .collect();
    let right: Vec<String> = (0..h)
        .map(|i| {
            if i == 0 {
                top.1
            } else if i + 1 == h {
                bot.1
            } else {
                mid.1
            }
            .to_string()
        })
        .collect();
    let rows = left
        .into_iter()
        .zip(grid.rows)
        .zip(right)
        .map(|((l, m), r)| format!("{l}{m}{r}"))
        .collect::<Vec<_>>();
    MathBox {
        baseline: grid.baseline,
        width: grid.width.saturating_add(2),
        rows,
    }
}

type FencePair = (char, char);

fn fence_pieces(delim: MatrixDelim) -> Option<(FencePair, FencePair, FencePair)> {
    match delim {
        MatrixDelim::None => None,
        MatrixDelim::Bracket => Some((('⎡', '⎤'), ('⎢', '⎥'), ('⎣', '⎦'))),
        MatrixDelim::Paren => Some((('⎛', '⎞'), ('⎜', '⎟'), ('⎝', '⎠'))),
        MatrixDelim::Brace => Some((('⎧', '⎫'), ('⎪', '⎪'), ('⎩', '⎭'))),
    }
}

fn inline_fences(delim: MatrixDelim) -> (&'static str, &'static str) {
    match delim {
        MatrixDelim::None => ("", ""),
        MatrixDelim::Bracket => ("[", "]"),
        MatrixDelim::Paren => ("(", ")"),
        MatrixDelim::Brace => ("{", "}"),
    }
}

/// Flatten a box to one line, joining rows with a space. Only ever called on
/// boxes that are already one row in inline mode; the join is a safety net.
fn one_row(b: &MathBox) -> String {
    b.rows.join(" ").trim().to_string()
}

fn paren_if_compound(expr: &MathExpr, laid: &MathBox) -> String {
    if expr.is_compound() {
        format!("({})", one_row(laid))
    } else {
        one_row(laid)
    }
}

fn wrap_if_compound(expr: &MathExpr, laid: &MathBox) -> String {
    paren_if_compound(expr, laid)
}

const SUPERS: [(char, char); 14] = [
    ('0', '⁰'),
    ('1', '¹'),
    ('2', '²'),
    ('3', '³'),
    ('4', '⁴'),
    ('5', '⁵'),
    ('6', '⁶'),
    ('7', '⁷'),
    ('8', '⁸'),
    ('9', '⁹'),
    ('+', '⁺'),
    ('-', '⁻'),
    ('n', 'ⁿ'),
    ('i', 'ⁱ'),
];

const SUBS: [(char, char); 14] = [
    ('0', '₀'),
    ('1', '₁'),
    ('2', '₂'),
    ('3', '₃'),
    ('4', '₄'),
    ('5', '₅'),
    ('6', '₆'),
    ('7', '₇'),
    ('8', '₈'),
    ('9', '₉'),
    ('+', '₊'),
    ('-', '₋'),
    ('n', 'ₙ'),
    ('i', 'ᵢ'),
];

/// The Unicode raised/lowered form of an expression, if every character has
/// one. Returns `None` the moment one does not — a half-translated script
/// reads worse than an honest `^(n+1)`.
fn unicode_script(expr: &MathExpr, table: [(char, char); 14]) -> Option<String> {
    let MathExpr::Sym { text, .. } = expr else {
        return None;
    };
    let mut out = String::new();
    for ch in text.trim().chars() {
        let mapped = table.iter().find(|(from, _)| *from == ch)?.1;
        out.push(mapped);
    }
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sym(s: &str) -> MathExpr {
        MathExpr::Sym {
            text: s.into(),
            class: MathClass::Ordinary,
        }
    }

    fn num(s: &str) -> MathExpr {
        MathExpr::Sym {
            text: s.into(),
            class: MathClass::Number,
        }
    }

    fn row(parts: Vec<MathExpr>) -> MathExpr {
        MathExpr::Row(parts.into_boxed_slice())
    }

    fn frac_expr() -> MathExpr {
        MathExpr::Frac {
            num: Box::new(row(vec![sym("a"), sym("+"), sym("b")])),
            den: Box::new(sym("c")),
        }
    }

    fn sample_expressions() -> Vec<MathExpr> {
        vec![
            sym("x"),
            row(vec![sym("a"), sym("+"), sym("b")]),
            frac_expr(),
            MathExpr::Sqrt {
                radicand: Box::new(row(vec![sym("x"), sym("+"), num("1")])),
                index: None,
            },
            MathExpr::Script {
                base: Box::new(sym("x")),
                sub: None,
                sup: Some(Box::new(num("2"))),
                limits: false,
            },
            MathExpr::Script {
                base: Box::new(sym("x")),
                sub: None,
                sup: Some(Box::new(row(vec![sym("n"), sym("+"), num("1")]))),
                limits: false,
            },
            MathExpr::Script {
                base: Box::new(MathExpr::Sym {
                    text: "∑".into(),
                    class: MathClass::LargeOp,
                }),
                sub: Some(Box::new(sym("i"))),
                sup: Some(Box::new(sym("n"))),
                limits: true,
            },
            MathExpr::Matrix {
                rows: vec![
                    vec![sym("a"), sym("bb")].into_boxed_slice(),
                    vec![sym("cc"), sym("d")].into_boxed_slice(),
                ]
                .into_boxed_slice(),
                delim: MatrixDelim::Bracket,
            },
            // Nested two deep.
            MathExpr::Frac {
                num: Box::new(frac_expr()),
                den: Box::new(sym("z")),
            },
        ]
    }

    #[test]
    fn a_symbol_is_one_row_on_the_baseline() {
        let b = lay_out(&sym("x"), Mode::Display);
        assert_eq!(b.rows, vec!["x".to_string()]);
        assert_eq!(b.baseline, 0);
        assert_eq!(b.width, 1);
    }

    #[test]
    fn a_row_concatenates_and_reports_its_measured_width() {
        let b = lay_out(&row(vec![sym("a"), sym("b"), sym("c")]), Mode::Display);
        assert_eq!(b.rows, vec!["abc".to_string()]);
        assert_eq!(b.width, 3);
    }

    #[test]
    fn width_is_measured_per_cluster_never_per_char() {
        let b = lay_out(&row(vec![sym("∫"), sym("α")]), Mode::Display);
        assert_eq!(b.width, cluster_width("∫α"));
    }

    #[test]
    fn a_fraction_stacks_over_a_rule_and_the_rule_is_the_baseline() {
        let b = lay_out(&frac_expr(), Mode::Display);
        assert_eq!(
            b.rows.len(),
            3,
            "numerator, rule, denominator: {:?}",
            b.rows
        );
        assert_eq!(b.baseline, 1, "the rule row is the baseline");
        assert!(b.rows[1].chars().all(|c| c == '─'), "rule: {:?}", b.rows);
        assert!(b.rows[2].contains('c'), "denominator: {:?}", b.rows);
    }

    #[test]
    fn a_square_root_gets_an_overbar_spanning_its_radicand() {
        let b = lay_out(
            &MathExpr::Sqrt {
                radicand: Box::new(sym("x")),
                index: None,
            },
            Mode::Display,
        );
        assert_eq!(b.rows.len(), 2, "{:?}", b.rows);
        assert!(b.rows[0].contains('‾'), "overbar: {:?}", b.rows);
        assert!(b.rows[1].contains('√'), "radical sign: {:?}", b.rows);
    }

    #[test]
    fn a_simple_superscript_prefers_the_unicode_form_and_stays_one_row() {
        let b = lay_out(
            &MathExpr::Script {
                base: Box::new(sym("x")),
                sub: None,
                sup: Some(Box::new(num("2"))),
                limits: false,
            },
            Mode::Display,
        );
        assert_eq!(
            b.rows,
            vec!["x²".to_string()],
            "no reason to grow to two rows"
        );
    }

    #[test]
    fn a_complex_superscript_raises_a_real_box() {
        let b = lay_out(
            &MathExpr::Script {
                base: Box::new(sym("x")),
                sub: None,
                sup: Some(Box::new(row(vec![sym("n"), sym("+"), num("1")]))),
                limits: false,
            },
            Mode::Display,
        );
        assert_eq!(b.rows.len(), 2, "n+1 has no Unicode form: {:?}", b.rows);
        assert_eq!(b.baseline, 1, "the base sits on the baseline");
    }

    #[test]
    fn a_big_operator_with_limits_stacks_them_above_and_below() {
        let b = lay_out(&sample_expressions()[6], Mode::Display);
        assert_eq!(b.rows.len(), 3, "{:?}", b.rows);
        assert_eq!(b.baseline, 1, "the operator is the baseline");
        assert!(b.rows[1].contains('∑'), "{:?}", b.rows);
    }

    #[test]
    fn a_matrix_column_aligns_inside_stretched_fences() {
        let b = lay_out(&sample_expressions()[7], Mode::Display);
        assert_eq!(b.rows.len(), 2, "{:?}", b.rows);
        assert!(b.rows[0].starts_with('⎡'), "top fence: {:?}", b.rows);
        assert!(b.rows[1].starts_with('⎣'), "bottom fence: {:?}", b.rows);
        assert_eq!(
            cluster_width(&b.rows[0]),
            cluster_width(&b.rows[1]),
            "columns align, so every row is the same width: {:?}",
            b.rows
        );
    }

    // --- the invariants ---

    #[test]
    fn art_is_the_same_at_every_width() {
        // `lay_out` takes no width, so this holds by construction. The test
        // exists so that threading one in fails loudly.
        for expr in sample_expressions() {
            let a = lay_out(&expr, Mode::Display);
            let b = lay_out(&expr, Mode::Display);
            assert_eq!(a, b, "art is a pure function of the expression");
        }
    }

    #[test]
    fn every_box_measures_what_it_claims() {
        for expr in sample_expressions() {
            for mode in [Mode::Display, Mode::Inline] {
                let b = lay_out(&expr, mode);
                assert!(b.baseline < b.rows.len(), "baseline outside the box: {b:?}");
                let widest = b.rows.iter().map(|r| cluster_width(r)).max().unwrap_or(0);
                assert_eq!(b.width, widest, "declared width disagrees: {b:?}");
                for r in &b.rows {
                    assert_eq!(cluster_width(r), b.width, "ragged row breaks paint: {b:?}");
                }
            }
        }
    }

    // --- inline mode ---

    #[test]
    fn inline_mode_is_always_exactly_one_row() {
        for expr in sample_expressions() {
            let b = lay_out(&expr, Mode::Inline);
            assert_eq!(
                b.rows.len(),
                1,
                "a paragraph row is one row tall; {expr:?} gave {:?}",
                b.rows
            );
            assert_eq!(b.baseline, 0);
        }
    }

    #[test]
    fn an_inline_fraction_becomes_a_solidus_form() {
        let b = lay_out(
            &MathExpr::Frac {
                num: Box::new(sym("a")),
                den: Box::new(sym("b")),
            },
            Mode::Inline,
        );
        assert_eq!(b.rows, vec!["a/b".to_string()]);
    }

    #[test]
    fn an_inline_fraction_parenthesises_a_compound_numerator() {
        let b = lay_out(&frac_expr(), Mode::Inline);
        assert_eq!(
            b.rows,
            vec!["(a+b)/c".to_string()],
            "precedence must survive"
        );
    }

    #[test]
    fn a_nested_inline_fraction_keeps_its_grouping() {
        let b = lay_out(
            &MathExpr::Frac {
                num: Box::new(frac_expr()),
                den: Box::new(sym("z")),
            },
            Mode::Inline,
        );
        assert_eq!(
            b.rows,
            vec!["((a+b)/c)/z".to_string()],
            "without the parens this reads as a different expression"
        );
    }

    #[test]
    fn a_nested_display_fraction_has_a_longer_outer_rule() {
        let b = lay_out(
            &MathExpr::Frac {
                num: Box::new(frac_expr()),
                den: Box::new(sym("z")),
            },
            Mode::Display,
        );
        let rules: Vec<usize> = b
            .rows
            .iter()
            .filter(|r| r.trim().chars().all(|c| c == '─'))
            .map(|r| r.trim().chars().count())
            .collect();
        assert_eq!(rules.len(), 2, "two rules: {:?}", b.rows);
        assert!(
            rules[1] > rules[0],
            "the outer rule is longer, so the levels are distinguishable: {:?}",
            b.rows
        );
    }

    #[test]
    fn an_inline_script_without_a_unicode_form_falls_back_to_caret() {
        let b = lay_out(&sample_expressions()[5], Mode::Inline);
        assert_eq!(b.rows, vec!["x^(n+1)".to_string()]);
    }
}
