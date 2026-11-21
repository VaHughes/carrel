//! Math that overflowed the LaTeX parser's stack, or panicked its lexer,
//! run through the real parser — in a process of its own, because a stack
//! overflow is an abort no test harness survives.
//!
//! `pulldown-latex` recurses once per command that takes the next command
//! as its bare argument, and `math::parse` refuses such a chain before the
//! parser sees it. The refusal used to scan bytes where the parser reads
//! tokens, so anything the parser skips (a `%` comment, a no-break space)
//! or takes as an argument where the scan saw a break (`+`, `[n]`, a
//! `\left…\right` group) hid a chain of any length. Each shape here
//! aborted the reader; all of them are source text now.

#![cfg(unix)]

use carrel_core::{Document, math};
use std::process::Command;

/// The stack a home-search thread gets, and the smallest the parser is
/// ever run on.
const STACK: usize = 2 << 20;

/// Each link repeated to a chain far past any stack.
const LINKS: usize = 3_000;

const COMMANDS: &[&str] = &[
    "\\sqrt",
    "\\frac",
    "\\hat",
    "\\mathrm",
    "\\text",
    "\\not",
    "\\overset",
    "\\textcolor{red}",
    "\\sideset",
    "\\genfrac",
    "\\xrightarrow",
    "\\pmod",
    "\\mathop",
    "\\underline",
    "\\big",
    "\\kern",
    "\\relax",
    "\\substack",
    "\\binom",
    "\\operatorname",
];

/// What may stand between two links: nothing to the parser, one argument,
/// several, a group of each kind, an error.
const BETWEEN: &[&str] = &[
    "",
    " ",
    "%c\n",
    "\u{a0}",
    "\u{2003}",
    "\x0b",
    "+",
    "a",
    "~",
    "'",
    "(",
    "{a}",
    "{a}{b}",
    "{}{}{}{}1",
    "[ab]",
    "[+]",
    "[",
    "]",
    "}",
    "\\alpha ",
    "\\,",
    "\\{",
    "\\frac a b",
    "\\left(a\\right)",
    "\\left.",
    "\\right)",
    "\\begin{matrix}a\\end{matrix}",
    "\\begin{matrix}",
    "\\end{matrix}",
    "\\\\[1pt]",
    "\\limits",
    "^",
    "_a",
    "1.5pt",
    "\\big(",
    "\\begingroup ",
    "\\endgroup ",
];

fn shapes() -> Vec<String> {
    let mut shapes = Vec::new();
    for command in COMMANDS {
        for between in BETWEEN {
            let link = format!("{command}{between}");
            let tail = ["x", "x y z", "", "}", "\\right)"][shapes.len() % 5];
            shapes.push(format!("{}{tail}", link.repeat(LINKS)));
        }
    }
    // Recursion that is not an argument chain: a run of definitions, each
    // giving the parser no event, and a run of comments in its lexer.
    shapes.push("\\def\\a{}".repeat(LINKS));
    shapes.push("\\let\\a=x".repeat(LINKS));
    shapes.push("\\relax ".repeat(LINKS));
    shapes.push("%c\n".repeat(100 * LINKS));
    // A chain a macro writes, which no scan of the source can see.
    shapes.push("\\def\\a#1{#1#1#1#1}\\def\\b#1{\\a{\\a{\\a{#1}}}}\\b{\\b{\\sqrt}}x".to_string());
    // The lexer steps one byte into a character of several: after a
    // number or a unit, and where `\char` wants its digit.
    for panics in [
        "\\char\u{e9}",
        "\\char 65\u{a0}x",
        "\\kern 1pt\u{a0}x",
        "\\hspace{1em\u{2003}}x",
        "\\begin{alignat}{\u{e9}}a\\end{alignat}",
        "\\begin{alignat}{2\u{a0}}a\\end{alignat}",
        "\\begin{array}{c}a\\\\[1pt\u{a0}]b\\end{array}",
        "\\genfrac{}{}{1pt\u{a0}}{}ab",
        "\\buildrel a\\",
    ] {
        shapes.push(panics.to_string());
    }
    shapes
}

/// The child: every shape through the parser, on a thread of [`STACK`].
#[test]
fn hostile_math_child() {
    if std::env::var_os("CARREL_MATH_STACK_CHILD").is_none() {
        return;
    }
    let worker = std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(|| {
            let shapes = shapes();
            for shape in &shapes {
                let _ = math::parse(shape);
            }
            // And as a document meets it, inline and display.
            for shape in shapes.iter().step_by(23) {
                let _ = Document::parse(&format!("a ${shape}$ b\n\n$$\n{shape}\n$$\n"));
            }
            println!("survived {}", shapes.len());
        })
        .unwrap();
    worker.join().unwrap();
}

#[test]
fn hostile_math_never_aborts_the_reader() {
    // A core limit of one byte: a regression here is an abort, and it
    // should fail this test without leaving a core dump behind.
    let out = Command::new("sh")
        .arg("-c")
        .arg("ulimit -c 1 2>/dev/null; exec \"$0\" \"$@\"")
        .arg(std::env::current_exe().unwrap())
        .args(["hostile_math_child", "--exact", "--nocapture"])
        .env("CARREL_MATH_STACK_CHILD", "1")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("survived"),
        "the parser went down on hostile math: {:?}\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
            .lines()
            .rev()
            .take(12)
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// The refusal is the documented one: the formula's source is the text.
/// (Chains short enough here that the parser, were it reached, survives.)
#[test]
fn a_refused_formula_is_shown_as_its_source() {
    for formula in [
        "\\sqrt%c\n".repeat(70) + "x",
        "\\frac+".repeat(70) + "x",
        "\\def\\a{x}\\a".to_string(),
    ] {
        assert_eq!(math::parse(&formula), None, "{formula:?}");
        let doc = Document::parse(&format!("a ${formula}$ b\n"));
        assert!(doc.text.contains('\\'), "{:?}", doc.text);
    }
    // And ordinary math beside a comment still renders.
    let doc = Document::parse("a $x^2 % squared\n y$ b\n");
    assert!(doc.text.contains("a x²y b"), "{:?}", doc.text);
}
