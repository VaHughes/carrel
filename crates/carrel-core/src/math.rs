//! LaTeX math → a **cell-free, width-free** expression tree.
//!
//! This module deliberately stops short of layout. A [`MathExpr`] has no rows,
//! no baseline, no width and no cells — those are the frontend's business, and
//! putting them here would be the `fn height() -> u16` shape that ended Helix's
//! frontend-agnostic view layer (discipline rule 1). The TUI turns this tree
//! into box art in `math_art.rs`; the GTK frontend will feed the same parse to
//! `pulldown_latex::mathml::push_mathml` and let `WebKitGTK` typeset it natively.
//!
//! `pulldown-latex` does the hard part: it resolves `\alpha` → `α` and `\int`
//! → `∫` itself, and it classifies every atom as ordinary/number/binary
//! operator/relation/large operator, which is exactly the semantic scope set
//! discipline rule 3 asks for. What is left here is shape: turning a
//! prefix-notation event stream into a tree.
//!
//! **On failure this returns `None`.** A reader must never show a parse error
//! where a document expected an equation; the frontend falls back to the
//! literal LaTeX source, which is honest and still searchable.

use pulldown_latex::event::{
    Content, EnvironmentFlow, Event, Grouping, ScriptPosition, ScriptType, Visual,
};
use pulldown_latex::{Parser, Storage};

/// The semantic class of an atom. A **scope**, never a colour — each frontend
/// maps these itself.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum MathClass {
    Ordinary,
    Number,
    BinaryOp,
    Relation,
    /// `∑`, `∫`, `∏` — an operator that may carry limits above and below.
    LargeOp,
    Punct,
    /// A bracket, brace, or bar that grows with what it encloses.
    Fence,
}

/// The delimiters around a matrix.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum MatrixDelim {
    None,
    Paren,
    Bracket,
    Brace,
}

/// A parsed math expression.
///
/// Contains no width, no height, no row count and no cell — see the module
/// docs. Anything layout-shaped belongs to the frontend.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum MathExpr {
    Sym {
        text: Box<str>,
        class: MathClass,
    },
    Row(Box<[MathExpr]>),
    Frac {
        num: Box<MathExpr>,
        den: Box<MathExpr>,
    },
    Sqrt {
        radicand: Box<MathExpr>,
        index: Option<Box<MathExpr>>,
    },
    Script {
        base: Box<MathExpr>,
        sub: Option<Box<MathExpr>>,
        sup: Option<Box<MathExpr>>,
        /// `true` when the scripts belong above and below the base rather than
        /// beside it — `\sum_{i}^{n}` in display style.
        limits: bool,
    },
    Matrix {
        rows: Box<[Box<[MathExpr]>]>,
        delim: MatrixDelim,
    },
}

impl MathExpr {
    /// A row of one element is that element; an empty row is an empty symbol.
    /// Keeps the tree free of one-child rows, which the layout would otherwise
    /// have to special-case everywhere.
    fn row(mut parts: Vec<MathExpr>) -> MathExpr {
        match parts.len() {
            0 => MathExpr::Sym {
                text: String::new().into_boxed_str(),
                class: MathClass::Ordinary,
            },
            1 => parts.remove(0),
            _ => MathExpr::Row(parts.into_boxed_slice()),
        }
    }

    /// Whether this needs parenthesising when it becomes an operand of an
    /// inline solidus or caret.
    ///
    /// A row does, obviously. **A fraction does too**: without parens
    /// `\frac{\frac{a+b}{c}}{z}` flattens to `(a+b)/c/z`, which reads as a
    /// different expression. A root, script or matrix does not — each already
    /// carries its own visual grouping.
    #[must_use]
    pub fn is_compound(&self) -> bool {
        matches!(self, MathExpr::Row(_) | MathExpr::Frac { .. })
    }
}

/// Parse LaTeX math into an expression tree, or `None` if it will not parse.
///
/// `None` is a normal outcome, not an error path: the frontend renders the
/// literal source, which is what a reader should show for math it cannot lay
/// out. See [`MathClass`] for what the atoms carry.
#[must_use]
pub fn parse(latex: &str) -> Option<MathExpr> {
    let storage = Storage::new();
    let mut events = Vec::new();
    for ev in Parser::new(latex, &storage) {
        events.push(ev.ok()?);
    }
    let mut cursor = 0usize;
    let expr = parse_group(&events, &mut cursor, None)?;
    // A trailing unconsumed event means the stream was malformed in a way the
    // walker silently tolerated. Refuse rather than render half an equation.
    if cursor == events.len() {
        Some(expr)
    } else {
        None
    }
}

/// Parse events until `stop` (or the end), returning them as one row.
fn parse_group(events: &[Event], cursor: &mut usize, stop: Option<usize>) -> Option<MathExpr> {
    let mut parts: Vec<MathExpr> = Vec::new();
    while *cursor < events.len() {
        if Some(*cursor) == stop {
            break;
        }
        if matches!(events[*cursor], Event::End) {
            break;
        }
        if let Step::Node(e) = parse_one(events, cursor)? {
            parts.push(e);
        }
    }
    Some(MathExpr::row(parts))
}

/// What one event yielded: a node, or nothing that affects shape.
enum Step {
    Node(MathExpr),
    /// Spacing, state changes, and group ends carry no shape of their own.
    Skip,
}

/// Parse exactly one node, advancing `cursor`.
///
/// `Script` is prefix in the stream: the event arrives, *then* the base, then
/// the operands. So the base is pulled from the stream here rather than taken
/// from the row built so far.
fn parse_one(events: &[Event], cursor: &mut usize) -> Option<Step> {
    let ev = events.get(*cursor)?;
    *cursor += 1;
    let out = match ev {
        Event::Content(c) => Step::Node(content(c)),
        Event::Begin(g) => Step::Node(begin(events, cursor, g)?),
        Event::Visual(Visual::Fraction(_)) => {
            let num = Box::new(operand(events, cursor)?);
            let den = Box::new(operand(events, cursor)?);
            Step::Node(MathExpr::Frac { num, den })
        }
        Event::Visual(Visual::SquareRoot) => Step::Node(MathExpr::Sqrt {
            radicand: Box::new(operand(events, cursor)?),
            index: None,
        }),
        Event::Visual(Visual::Root) => {
            // `\sqrt[n]{x}` — the index comes first in the stream.
            let index = Box::new(operand(events, cursor)?);
            let radicand = Box::new(operand(events, cursor)?);
            Step::Node(MathExpr::Sqrt {
                radicand,
                index: Some(index),
            })
        }
        Event::Visual(Visual::Negation) => {
            // Rendered as the operand with a combining slash: the layout
            // engine's problem, so keep the operand and let it decide.
            Step::Node(operand(events, cursor)?)
        }
        Event::Script { ty, position } => {
            let base = Box::new(operand(events, cursor)?);
            let limits = matches!(position, ScriptPosition::AboveBelow)
                || (matches!(position, ScriptPosition::Movable)
                    && matches!(
                        &*base,
                        MathExpr::Sym {
                            class: MathClass::LargeOp,
                            ..
                        }
                    ));
            let (sub, sup) = match ty {
                ScriptType::Subscript => (Some(Box::new(operand(events, cursor)?)), None),
                ScriptType::Superscript => (None, Some(Box::new(operand(events, cursor)?))),
                ScriptType::SubSuperscript => {
                    let sub = Box::new(operand(events, cursor)?);
                    let sup = Box::new(operand(events, cursor)?);
                    (Some(sub), Some(sup))
                }
            };
            Step::Node(MathExpr::Script {
                base,
                sub,
                sup,
                limits,
            })
        }
        // Spacing and state changes carry no shape. Dropping them is a
        // deliberate simplification: a reader shows the expression, not the
        // author's kerning. `EnvironmentFlow` is consumed by `begin` for
        // grids; reaching one here means it was outside any grid.
        Event::End | Event::Space { .. } | Event::StateChange(_) | Event::EnvironmentFlow(_) => {
            Step::Skip
        }
    };
    Some(out)
}

/// Parse a single operand: either one atom, or a whole `Begin`…`End` group.
fn operand(events: &[Event], cursor: &mut usize) -> Option<MathExpr> {
    loop {
        let ev = events.get(*cursor)?;
        if matches!(ev, Event::Space { .. } | Event::StateChange(_)) {
            *cursor += 1;
            continue;
        }
        return match parse_one(events, cursor)? {
            Step::Node(e) => Some(e),
            // An operand slot filled by a shapeless event is an empty box,
            // not a parse failure: `x^{}` is legal LaTeX.
            Step::Skip => Some(MathExpr::Sym {
                text: String::new().into_boxed_str(),
                class: MathClass::Ordinary,
            }),
        };
    }
}

/// A `Begin(...)` group: a plain brace group, a fenced group, or a matrix.
fn begin(events: &[Event], cursor: &mut usize, g: &Grouping) -> Option<MathExpr> {
    let delim = match g {
        Grouping::Normal => None,
        Grouping::LeftRight(l, _) => Some(match l {
            Some('[') => MatrixDelim::Bracket,
            Some('{') => MatrixDelim::Brace,
            Some('(') => MatrixDelim::Paren,
            _ => MatrixDelim::None,
        }),
        Grouping::Matrix { .. } | Grouping::Array(_) | Grouping::Cases { .. } => {
            Some(MatrixDelim::Bracket)
        }
        _ => Some(MatrixDelim::None),
    };
    let is_grid = matches!(
        g,
        Grouping::Matrix { .. } | Grouping::Array(_) | Grouping::Cases { .. }
    );

    if !is_grid {
        let inner = parse_group(events, cursor, None)?;
        expect_end(events, cursor)?;
        return Some(match delim {
            Some(MatrixDelim::None) | None => inner,
            Some(d) => MathExpr::Matrix {
                rows: vec![vec![inner].into_boxed_slice()].into_boxed_slice(),
                delim: d,
            },
        });
    }

    // A grid: cells separated by `Alignment`, rows by `NewLine`.
    let mut rows: Vec<Box<[MathExpr]>> = Vec::new();
    let mut row: Vec<MathExpr> = Vec::new();
    let mut cell: Vec<MathExpr> = Vec::new();
    loop {
        let ev = events.get(*cursor)?;
        match ev {
            Event::End => {
                *cursor += 1;
                break;
            }
            Event::EnvironmentFlow(EnvironmentFlow::Alignment) => {
                *cursor += 1;
                row.push(MathExpr::row(std::mem::take(&mut cell)));
            }
            Event::EnvironmentFlow(EnvironmentFlow::NewLine { .. }) => {
                *cursor += 1;
                row.push(MathExpr::row(std::mem::take(&mut cell)));
                rows.push(std::mem::take(&mut row).into_boxed_slice());
            }
            _ => {
                if let Step::Node(e) = parse_one(events, cursor)? {
                    cell.push(e);
                }
            }
        }
    }
    row.push(MathExpr::row(cell));
    rows.push(row.into_boxed_slice());
    Some(MathExpr::Matrix {
        rows: rows.into_boxed_slice(),
        delim: delim.unwrap_or(MatrixDelim::None),
    })
}

fn expect_end(events: &[Event], cursor: &mut usize) -> Option<()> {
    if matches!(events.get(*cursor)?, Event::End) {
        *cursor += 1;
        Some(())
    } else {
        None
    }
}

fn content(c: &Content) -> MathExpr {
    let (text, class) = match c {
        Content::Text(s) | Content::Function(s) => ((*s).to_string(), MathClass::Ordinary),
        Content::Number(s) => ((*s).to_string(), MathClass::Number),
        Content::Ordinary { content, .. } => (content.to_string(), MathClass::Ordinary),
        Content::LargeOp { content, .. } => (content.to_string(), MathClass::LargeOp),
        Content::BinaryOp { content, .. } => (content.to_string(), MathClass::BinaryOp),
        Content::Relation { content, .. } => {
            // The char pair is private upstream; the documented way out is the
            // encode buffer, which the crate says needs 8 bytes.
            let mut buf = [0u8; 8];
            let text = String::from_utf8_lossy(content.encode_utf8_to_buf(&mut buf)).into_owned();
            (text, MathClass::Relation)
        }
        Content::Delimiter { content, .. } => (content.to_string(), MathClass::Fence),
        Content::Punctuation(ch) => (ch.to_string(), MathClass::Punct),
    };
    MathExpr::Sym {
        text: text.into_boxed_str(),
        class,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fraction_parses_to_num_and_den() {
        let Some(MathExpr::Frac { num, den }) = parse(r"\frac{a+b}{c}") else {
            panic!("expected a fraction, got {:?}", parse(r"\frac{a+b}{c}"));
        };
        assert!(
            matches!(*den, MathExpr::Sym { .. }),
            "denominator is one symbol: {den:?}"
        );
        let MathExpr::Row(parts) = *num else {
            panic!("numerator is a row, got {num:?}")
        };
        assert_eq!(parts.len(), 3, "a + b");
    }

    #[test]
    fn a_superscript_binds_to_its_base_not_to_the_whole_row() {
        let Some(MathExpr::Row(parts)) = parse("E = mc^2") else {
            panic!("expected a row, got {:?}", parse("E = mc^2"))
        };
        assert!(
            parts
                .iter()
                .any(|p| matches!(p, MathExpr::Script { sup: Some(_), .. })),
            "the ^2 attaches to c: {parts:?}"
        );
    }

    #[test]
    fn symbols_arrive_already_resolved_and_classified() {
        let Some(MathExpr::Row(parts)) = parse(r"\alpha \ge 0") else {
            panic!("expected a row")
        };
        assert!(
            matches!(&parts[0], MathExpr::Sym { text, class: MathClass::Ordinary } if &**text == "α"),
            "pulldown-latex resolves \\alpha itself: {:?}",
            parts[0]
        );
        assert!(
            matches!(
                &parts[1],
                MathExpr::Sym {
                    class: MathClass::Relation,
                    ..
                }
            ),
            "a relation: {:?}",
            parts[1]
        );
    }

    #[test]
    fn a_matrix_keeps_its_rows_and_columns() {
        let src = r"\begin{matrix} a & b \\ c & d \end{matrix}";
        let Some(MathExpr::Matrix { rows, .. }) = parse(src) else {
            panic!("expected a matrix, got {:?}", parse(src));
        };
        assert_eq!(rows.len(), 2, "two rows: {rows:?}");
        assert_eq!(rows[0].len(), 2, "two columns: {:?}", rows[0]);
    }

    #[test]
    fn a_square_root_keeps_its_radicand() {
        let Some(MathExpr::Sqrt { radicand, index }) = parse(r"\sqrt{x+1}") else {
            panic!("expected a root")
        };
        assert!(index.is_none());
        assert!(radicand.is_compound(), "x+1 is a row: {radicand:?}");
    }

    #[test]
    fn a_parse_error_is_none_not_a_panic_and_not_garbage() {
        assert!(parse(r"\frac{").is_none(), "unbalanced input yields None");
        assert!(parse(r"\nosuchcommand").is_none(), "unknown command");
    }
}
