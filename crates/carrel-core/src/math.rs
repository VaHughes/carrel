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

/// How deep the walker will follow a nesting before refusing it.
///
/// The walk below is mutual recursion — `parse_group` to `parse_one` to
/// `operand`/`begin` and back — with no bound of its own, so a document
/// carrying `\frac{\frac{\frac{…` deep enough overflows the stack, and the
/// release binary is `panic = "abort"`: not an error a frontend can render,
/// but the reader gone. This closes a hole in a public API rather than a
/// crash anyone has hit — the same input through the shipped binary did not
/// reach it, so something on that path bounds the nesting first. Which is a
/// reason not to depend on that path.
///
/// 256 is far past any equation a person writes, and refusal is the
/// documented outcome anyway: the frontend renders the literal LaTeX.
const MAX_DEPTH: u32 = 256;

/// Parse LaTeX math into an expression tree, or `None` if it will not parse.
///
/// `None` is a normal outcome, not an error path: the frontend renders the
/// literal source, which is what a reader should show for math it cannot lay
/// out. See [`MathClass`] for what the atoms carry.
#[must_use]
pub fn parse(latex: &str) -> Option<MathExpr> {
    if nesting(latex).refused() {
        return None;
    }
    let storage = Storage::new();
    let mut events = Vec::new();
    for ev in Parser::new(latex, &storage) {
        events.push(ev.ok()?);
    }
    let mut cursor = 0usize;
    let expr = parse_group(&events, &mut cursor, None, 0)?;
    // A trailing unconsumed event means the stream was malformed in a way the
    // walker silently tolerated. Refuse rather than render half an equation.
    if cursor == events.len() {
        Some(expr)
    } else {
        None
    }
}

/// How many commands may be waiting on an argument at once.
///
/// `pulldown-latex` parses a bare argument by recursion: `\sqrt\sqrt…x`,
/// `\frac 1\frac 1…x` and `\frac{1}\frac{1}…x` each nest native frames per
/// command, and overflowed a 2 MiB thread stack — what home search's
/// threads get — at about 150 deep in a debug build and 2 000 in release.
/// Braced nesting does not recurse there, which is why this bound is far
/// tighter than [`MAX_DEPTH`]. Nobody writes a bare chain longer than a
/// few; what the scan counts can also be commands it does not know, which
/// it holds open to the end of their group, and sixty-odd of those cost a
/// formula its rendering, never its text.
const MAX_BARE_CHAIN: u32 = 64;

/// How many comments may follow one another with no token between them:
/// the lexer skips a comment by calling itself, a frame per comment.
const MAX_COMMENT_RUN: u32 = 64;

/// A frame that no argument closes: a command the scan cannot count for.
const OPEN_ENDED: u8 = u8::MAX;

/// Control words that read nothing after themselves — symbols, operators,
/// relations, arrows, delimiters, dots, fixed spaces, function names, style
/// switches — sorted for `binary_search`. A test holds every one of them
/// to that against the parser itself.
#[rustfmt::skip]
const NO_ARGUMENT: &[&str] = &[
    "Alpha", "And", "Angstrom", "Bbbk", "Beta", "Bot", "Box", "Bumpeq", "Cap", "Chi",
    "Colonapprox", "Coloneq", "Coloneqq", "Colonsim", "Cup", "Dash", "Delta", "Diamond",
    "Doteq", "Downarrow", "Earth", "Epsilon", "Eqcolon", "Eqqcolon", "Eta", "Finv", "Game",
    "Gamma", "Im", "Iota", "Join", "Kappa", "Lambda", "Leftarrow", "Leftrightarrow",
    "Lleftarrow", "Longleftarrow", "Longleftrightarrow", "Longrightarrow", "Lsh", "Mu", "Nu",
    "Omega", "Omicron", "Otimes", "P", "Perp", "Phi", "Pi", "Pr", "Psi", "QED", "Re", "Rho",
    "Rightarrow", "Rrightarrow", "Rsh", "S", "Sigma", "Subset", "Supset", "Tau", "Theta",
    "Uparrow", "Updownarrow", "Upsilon", "Vdash", "Vert", "Vvdash", "Xi", "Zeta", "aleph",
    "alpha", "amalg", "angle", "approx", "approxcolon", "approxcoloncolon", "approxeq",
    "arccos", "arceq", "arcsin", "arctan", "arg", "ast", "astrosun", "asymp", "backcong",
    "backdprime", "backepsilon", "backprime", "backsim", "backsimeq", "backslash",
    "backtrprime", "ballotx", "barwedge", "because", "beta", "beth", "between", "bf", "bigcap",
    "bigcirc", "bigcup", "bigodot", "bigoplus", "bigotimes", "bigsqcap", "bigsqcup", "bigstar",
    "bigtimes", "bigtriangledown", "bigtriangleup", "biguplus", "bigvee", "bigwedge",
    "blackhourglass", "blacklozenge", "blacksquare", "blacktriangle", "blacktriangledown",
    "blacktriangleleft", "blacktriangleright", "bmod", "bot", "bowtie", "boxast", "boxbox",
    "boxcircle", "boxdot", "boxminus", "boxplus", "boxtimes", "bullet", "bumpeq", "cal", "cap",
    "cdot", "cdotp", "cdots", "cent", "centerdot", "checkmark", "chi", "circ", "circeq",
    "circlearrowleft", "circlearrowright", "circledR", "circledS", "circledast", "circledcirc",
    "circleddash", "circledequal", "circledparallel", "circledvert", "circlehbar", "clubsuit",
    "coh", "colon", "colonapprox", "coloncolon", "coloncolonapprox", "coloncolonequals",
    "coloncolonminus", "coloncolonsim", "coloneq", "coloneqq", "colonequals", "colonminus",
    "colonsim", "complement", "concavediamond", "concavediamondtickleft",
    "concavediamondtickright", "cong", "coprod", "copyright", "cos", "cosh", "cot", "coth",
    "csc", "cup", "curlyeqprec", "curlyeqsucc", "curlyvee", "curlywedge", "curvearrowleft",
    "curvearrowright", "dag", "dagger", "daleth", "dashleftarrow", "dashrightarrow", "dashv",
    "dblcolon", "ddag", "ddagger", "ddots", "deg", "degree", "delta", "det", "diagdown",
    "diagup", "diameter", "diamond", "diamondsuit", "digamma", "dim", "displaystyle", "div",
    "divideontimes", "doteq", "doteqdot", "dotplus", "dots", "dotsb", "dotsc", "dotsi", "dotsm",
    "dotso", "doublebarwedge", "doublecap", "downarrow", "downdownarrows", "downharpoonleft",
    "downharpoonright", "dprime", "ell", "emptyset", "enspace", "epsilon", "eqcirc", "eqcolon",
    "eqdef", "eqeq", "eqqcolon", "eqsim", "eqslantgtr", "eqslantless", "equiv", "eta", "eth",
    "euro", "exists", "exp", "fallingdotseq", "female", "fint", "flat", "forall", "frown",
    "fullouterjoin", "gamma", "gcd", "ge", "geq", "geqq", "geqslant", "gets", "gg", "ggg",
    "gggtr", "gimel", "gnapprox", "gneq", "gneqq", "gnsim", "gt", "gtrapprox", "gtrdot",
    "gtreqless", "gtreqqless", "gtrless", "gtrsim", "gvertneqq", "hbar", "heartsuit", "hom",
    "hookleftarrow", "hookrightarrow", "hourglass", "hslash", "iddots", "idotsin", "iff",
    "iiiint", "iiint", "iint", "imageof", "imath", "impliedby", "implies", "in", "inf", "infty",
    "int", "intBar", "intbar", "intcap", "intclockwise", "intcup", "intercal", "intlarhk",
    "intop", "intx", "iota", "isin", "it", "jmath", "kappa", "ker", "lAngle", "lBrace",
    "lBrack", "lVert", "lambda", "land", "langle", "lbrace", "lbrack", "lceil", "ldotp",
    "ldots", "le", "leadsto", "leftarrow", "leftarrowtail", "leftharpoondown", "leftharpoonup",
    "leftleftarrows", "leftmodels", "leftmoon", "leftouterjoin", "leftrightarrow",
    "leftrightarrows", "leftrightharpoons", "leftrightsquigarrow", "leftthreetimes", "leq",
    "leqq", "leqslant", "lessapprox", "lessdot", "lesseqgtr", "lesseqqgtr", "lessgtr",
    "lesssim", "lfloor", "lg", "lgroup", "lhd", "lightning", "lim", "liminf", "limsup", "ll",
    "llangle", "llcorner", "lll", "llless", "llparenthesis", "lmoustache", "ln", "lnapprox",
    "lneq", "lneqq", "lnot", "lnsim", "log", "longleftarrow", "longleftrightarrow",
    "longmapsto", "longrightarrow", "looparrowleft", "looparrowright", "lor", "lozenge",
    "lozengeminus", "lparen", "lq", "lrcorner", "lt", "ltimes", "lvert", "lvertneqq", "male",
    "maltese", "mapsfrom", "mapsto", "mathellipsis", "mathsterling", "mathstrut", "max",
    "measeq", "measuredangle", "medspace", "mho", "mid", "min", "minuscolon", "minuscoloncolon",
    "models", "mp", "mu", "multimap", "multimapboth", "multimapinv", "nLeftarrow",
    "nLeftrightarrow", "nRightarrow", "nVDash", "nVdash", "nabla", "natural", "ncong", "ne",
    "nearrow", "neg", "negmedspace", "negthickspace", "negthinspace", "neq", "nexists", "ngeq",
    "ngeqq", "ngeqslant", "ngtr", "ni", "nleftarrow", "nleftrightarrow", "nleq", "nleqq",
    "nleqslant", "nless", "nmid", "nobreakspace", "notin", "notni", "nparallel", "nprec",
    "npreceq", "nrightarrow", "nshortmid", "nshortparallel", "nsim", "nsubset", "nsubseteq",
    "nsubseteqq", "nsucc", "nsucceq", "nsupset", "nsupseteq", "nsupseteqq", "ntriangleleft",
    "ntrianglelefteq", "ntriangleright", "ntrianglerighteq", "nu", "nvDash", "nvdash",
    "nwarrow", "obar", "obslash", "odiv", "odot", "ogreaterthan", "oiiint", "oiint", "oint",
    "olessthan", "omega", "omicron", "ominus", "operp", "oplus", "origof", "oslash", "otimes",
    "otimeshat", "owns", "parallel", "parr", "partial", "permil", "perp", "phi", "pi",
    "pitchfork", "pm", "pointint", "pounds", "prec", "precapprox", "preccurlyeq", "preceq",
    "precnapprox", "precneqq", "precnsim", "precsim", "prime", "prod", "propto", "psi",
    "qprime", "qquad", "quad", "questeq", "rAngle", "rBrace", "rBrack", "rVert", "rangle",
    "ratio", "rbrace", "rbrack", "rceil", "restriction", "rfloor", "rgroup", "rhd", "rho",
    "rightarrow", "rightarrowtail", "rightharpoondown", "rightharpoonup", "rightleftarrows",
    "rightleftharpoons", "rightmoon", "rightouterjoin", "rightrightarrows", "rightsquigarrow",
    "rightthreetimes", "risingdotseq", "rm", "rmoustache", "rparen", "rppolint", "rq",
    "rrangle", "rrparenthesis", "rtimes", "rvert", "scoh", "scpolint", "scriptscriptstyle",
    "scriptstyle", "searrow", "sec", "setminus", "sf", "sgn", "sharp", "shortmid",
    "shortparallel", "sigma", "sim", "simeq", "sin", "sincoh", "sinh", "smallfrown", "smallint",
    "smallsetminus", "smallsmile", "smile", "smiley", "spadesuit", "sphericalangle", "sqcap",
    "sqcup", "sqint", "sqsubset", "sqsubseteq", "sqsupset", "sqsupseteq", "square", "sslash",
    "standardstate", "star", "stareq", "strictfi", "strictif", "strut", "subset", "subseteq",
    "subseteqq", "subsetneq", "subsetneqq", "succ", "succapprox", "succcurlyeq", "succeq",
    "succnapprox", "succneqq", "succnsim", "succsim", "sum", "sun", "sup", "supset", "supseteq",
    "supseteqq", "supsetneq", "supsetneqq", "surd", "swarrow", "tan", "tanh", "tau",
    "textstyle", "therefore", "theta", "thickapprox", "thicksim", "thickspace", "thinspace",
    "times", "to", "top", "triangle", "triangledown", "triangleleft", "trianglelefteq",
    "triangleminus", "triangleplus", "triangleq", "triangleright", "trianglerighteq",
    "triangletimes", "trprime", "tt", "twoheadleftarrow", "twoheadrightarrow", "ulcorner",
    "unlhd", "unrhd", "uparrow", "updownarrow", "upharpoonleft", "upharpoonright", "uplus",
    "upsilon", "upuparrows", "urcorner", "vDash", "varDelta", "varGamma", "varLambda",
    "varOmega", "varPhi", "varPi", "varPsi", "varSigma", "varTheta", "varUpsilon", "varXi",
    "varclubsuit", "vardiamondsuit", "varepsilon", "varheartsuit", "varkappa", "varnothing",
    "varointclockwise", "varphi", "varpi", "varpropto", "varrho", "varsigma", "varspadesuit",
    "varsubsetneq", "varsubsetneqq", "varsupsetneq", "varsupsetneqq", "vartheta", "vartriangle",
    "vartriangleleft", "vartriangleright", "vcentcolon", "vdash", "vdots", "vee", "veebar",
    "veeeq", "vert", "wedge", "wedgeq", "whitesquaretickleft", "whitesquaretickright", "with",
    "wp", "wr", "xi", "yen", "zeta",
];

/// Control words that read one argument and nothing else — fonts,
/// accents over and under, atom classes, text — sorted likewise.
#[rustfmt::skip]
const ONE_ARGUMENT: &[&str] = &[
    "Overrightarrow", "acute", "bar", "boldsymbol", "breve", "cancel", "check", "color", "ddot",
    "dot", "grave", "hat", "mathbb", "mathbbit", "mathbf", "mathbfcal", "mathbffrak",
    "mathbfit", "mathbfsfit", "mathbfsfup", "mathbfup", "mathbin", "mathcal", "mathclose",
    "mathfrak", "mathinner", "mathit", "mathnormal", "mathop", "mathopen", "mathord",
    "mathpunct", "mathrel", "mathring", "mathrm", "mathsf", "mathsfit", "mathsfup", "mathtt",
    "mathup", "not", "operatorname", "overbrace", "overbracket", "overgroup", "overleftarrow",
    "overleftharpoon", "overleftrightarrow", "overline", "overparen", "overrightarrow",
    "overrightharpoon", "pmod", "substack", "symbb", "symbbit", "symbf", "symbfcal",
    "symbffrak", "symbfit", "symbfsfit", "symbfsfup", "symbfup", "symcal", "symfrak", "symit",
    "symnormal", "symrm", "symsf", "symsfit", "symsfup", "symtt", "symup", "text", "textbf",
    "textit", "textrm", "textsf", "texttt", "tilde", "underbar", "underbrace", "underbracket",
    "undergroup", "underleftarrow", "underleftrightarrow", "underline", "underparen",
    "underrightarrow", "vec", "widecheck", "widehat", "wideparen", "widetilde",
];

/// The ones that read two: fractions, stacked relations, a color and what
/// it colors.
#[rustfmt::skip]
const TWO_ARGUMENTS: &[&str] = &[
    "binom", "cfrac", "colorbox", "dbinom", "dfrac", "frac", "overset", "stackrel", "tbinom",
    "textcolor", "tfrac", "underset",
];

const THREE_ARGUMENTS: &[&str] = &["fcolorbox"];

/// How many arguments a control word reads and nothing else, for the ones
/// the scan can count; `None` is a command it cannot.
fn arity(word: &[u8]) -> Option<u8> {
    let word = std::str::from_utf8(word).ok()?;
    [NO_ARGUMENT, ONE_ARGUMENT, TWO_ARGUMENTS, THREE_ARGUMENTS]
        .iter()
        .position(|words| words.binary_search(&word).is_ok())
        .map(|wants| wants as u8)
}

/// What the scan found: see [`nesting`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Nesting {
    /// The deepest a group sits inside others.
    groups: u32,
    /// The most commands waiting on an argument at one point.
    bare: u32,
    /// The longest run of comments with no token between them.
    comments: u32,
    /// The formula defines a macro, or the scan gave up on its size.
    unbounded: bool,
}

impl Nesting {
    fn refused(&self) -> bool {
        self.unbounded
            || self.groups > MAX_DEPTH
            || self.bare > MAX_BARE_CHAIN
            || self.comments > MAX_COMMENT_RUN
    }
}

/// What cutting groups has cost so far, and the deepest nesting seen in it.
#[derive(Default)]
struct Cost {
    spent: usize,
    groups: u32,
}

/// The parser's own search for where a group ends, on `b` from just past the
/// opener: the length of the content, or `None` where the parser reports the
/// group unbalanced. It counts `open` and `close` wherever they stand —
/// `\left` inside braces, `\right` as the start of `\rightarrow` — skips
/// comments, and lets a backslash escape the byte after it. The scan has to
/// cut where the parser cuts, so this copies its loop rather than tidying it.
///
/// `cost` takes the bytes read, and how deep the group — itself `at` deep —
/// holds others of its kind: enough of those is a refusal already, without
/// cutting each in turn.
fn group_content(b: &[u8], open: &[u8], close: &[u8], at: u32, cost: &mut Cost) -> Option<usize> {
    let (mut escaped, mut index, mut depth) = (false, 0usize, 0u32);
    let balanced = loop {
        let closes = !escaped && b[index..].starts_with(close);
        if closes && depth == 0 {
            break true;
        }
        if index + close.len() > b.len() {
            break false;
        }
        if !escaped && b[index..].starts_with(open) {
            depth += 1;
            cost.groups = cost.groups.max(at.saturating_add(depth));
            index += open.len();
            continue;
        }
        if closes {
            depth -= 1;
            index += close.len();
            continue;
        }
        match b[index] {
            b'\\' => escaped = !escaped,
            b'%' if !escaped => match b[index..].iter().position(|&c| c == b'\n') {
                Some(line) => index += line,
                None => break false,
            },
            _ => escaped = false,
        }
        index += 1;
    };
    cost.spent += index;
    balanced.then_some(index)
}

/// The byte length of the character at `i`, which is in bounds.
fn char_len(b: &[u8], i: usize) -> usize {
    let len = match b[i] {
        0..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        _ => 4,
    };
    len.min(b.len() - i)
}

/// Past the whitespace at `i` — every kind `char::is_whitespace` knows, since
/// that is what the lexer trims — stopping at `end`.
fn skip_space(s: &str, mut i: usize, end: usize) -> usize {
    while i < end {
        let Some(c) = s[i..end].chars().next().filter(|c| c.is_whitespace()) else {
            break;
        };
        i += c.len_utf8();
    }
    i
}

/// Past whitespace and comments at `i`: where the lexer's next token starts.
/// `run` keeps the most comments skipped in one go.
fn skip_blank(s: &str, mut i: usize, end: usize, run: &mut u32) -> usize {
    let b = s.as_bytes();
    let mut comments = 0;
    loop {
        i = skip_space(s, i, end);
        if i >= end || b[i] != b'%' {
            break;
        }
        comments += 1;
        i = b[i..end]
            .iter()
            .position(|&c| c == b'\n')
            .map_or(end, |line| i + line + 1);
    }
    *run = (*run).max(comments);
    i
}

/// The control sequence whose name starts at `i`, just past the backslash:
/// its name — letters, or else one character — and where the lexer stands
/// after trimming the space behind it.
fn control_sequence(s: &str, i: usize, end: usize) -> (&[u8], usize) {
    let b = &s.as_bytes()[..end];
    if i >= end {
        return (&[], end);
    }
    let letters = b[i..]
        .iter()
        .take_while(|c| c.is_ascii_alphabetic())
        .count();
    let len = letters.max(char_len(b, i));
    (&b[i..i + len], skip_space(s, i + len, end))
}

/// Past one token at `i`, as the lexer takes one where it wants a
/// delimiter: a character, or a control sequence and the space after it.
fn skip_token(s: &str, i: usize, end: usize, run: &mut u32) -> usize {
    let b = s.as_bytes();
    let i = skip_blank(s, i, end, run);
    if i >= end {
        end
    } else if b[i] == b'\\' {
        control_sequence(s, i + 1, end).1
    } else {
        i + char_len(&b[..end], i)
    }
}

/// What reads a number or a length: row breaks, kerns and skips, `\hspace`,
/// `\Space`, `\genfrac`, the `alignat` family.
const READS_NUMBERS: [&str; 8] = [
    "\\\\", "\\cr", "kern", "skip", "hspace", "\\Space", "genfrac", "align",
];

/// Would the lexer panic here? After a number or a unit it steps over one
/// whitespace character as one BYTE, which lands inside a no-break space.
/// Only [`READS_NUMBERS`] get that far: `5` and a no-break space before
/// `\text{kg}` is safe, and renders.
fn steps_into_a_character(latex: &str) -> bool {
    if latex.is_ascii() || !READS_NUMBERS.iter().any(|word| latex.contains(word)) {
        return false;
    }
    let mut after_word = false;
    latex.chars().any(|c| {
        let steps = after_word && c.is_whitespace() && !c.is_ascii();
        after_word = c.is_ascii_alphanumeric();
        steps
    })
}

/// How deeply `latex` nests, by a scan that runs before the parser is
/// allowed near it.
///
/// [`MAX_DEPTH`] in the walker came too late: `pulldown-latex` runs to
/// completion first, its cost grows with the square of the group nesting —
/// `{`×15 000 (30 KB of inline math) took a second of CPU, `{`×30 000 four
/// — and a bare-argument chain overflows its stack (see [`MAX_BARE_CHAIN`]),
/// which in the `panic = "abort"` release binary is the reader gone. Every
/// caller comes through [`parse`] — inline math at document parse, display
/// math in the frontend, and the `math_view` that home search reads — so
/// refusing here protects them all, and the refusal is the documented one:
/// the literal source is shown.
///
/// The scan reads what the parser reads, because every difference between
/// them was a way past it: a `%` comment or a no-break space between two
/// commands is nothing to the parser and so nothing here, and a group ends
/// where [`group_content`] says the parser ends it. Each group — braces,
/// `\left`…`\right`, an environment, an optional `[…]` — is parsed as its
/// own text with no native frame per level, so each is scanned on its own
/// and `groups` is how deep they sit.
///
/// Within one group the scan keeps the frames the parser would have open:
/// a command pushes one holding the arguments it still wants ([`arity`]),
/// and a character, a group or a symbol fills the innermost, closing every
/// frame that completes. `bare` is the most frames open at once. Where the
/// scan is not sure it errs toward more frames, never fewer: a command it
/// has no count for stays open to the end of its group, `^` and `_` fill
/// nothing. So `\frac+\frac+…` and `\sqrt[n]\sqrt[n]…` are the chains they
/// are, and `\hat a\hat b…` is no chain however long.
///
/// A macro definition is refused outright (`unbounded`): expansion writes
/// text the scan never sees, a run of definitions is itself a recursion in
/// the parser, and a definition is lost at the end of its formula anyway.
/// So is a formula whose groups cost more to cut than a small multiple of
/// its length, which keeps the scan linear.
#[allow(clippy::too_many_lines)]
fn nesting(latex: &str) -> Nesting {
    let b = latex.as_bytes();
    let mut found = Nesting::default();
    if steps_into_a_character(latex) {
        found.unbounded = true;
        return found;
    }
    // Groups nested one in the next to [`MAX_DEPTH`] cost half this.
    let (mut cost, budget) = (Cost::default(), 128 * b.len() + 65_536);
    // Groups still to scan, as `(start, end, depth)`.
    let mut groups: Vec<(usize, usize, u32)> = vec![(0, b.len(), 0)];
    let mut frames: Vec<u8> = Vec::new();
    // An argument arrived: it fills the innermost frame, and a frame it
    // completes is itself the argument of the one outside.
    let fill = |frames: &mut Vec<u8>| {
        while let Some(wants) = frames.last_mut() {
            if *wants == OPEN_ENDED {
                break;
            }
            *wants -= 1;
            if *wants > 0 {
                break;
            }
            frames.pop();
        }
    };
    while let Some((start, end, depth)) = groups.pop() {
        found.groups = found.groups.max(depth);
        if depth > MAX_DEPTH {
            return found;
        }
        frames.clear();
        let mut i = start;
        // Where the parser would report an error the scan reads on, with
        // a frame nothing closes: it is not always the parser's error.
        // `\sideset` takes two tokens unread, a stray `}` among them.
        loop {
            if cost.spent > budget || cost.groups > MAX_DEPTH {
                found.unbounded = cost.spent > budget;
                found.groups = found.groups.max(cost.groups);
                return found;
            }
            i = skip_blank(latex, i, end, &mut found.comments);
            if i >= end {
                break;
            }
            match b[i] {
                b'{' => {
                    if let Some(len) =
                        group_content(&b[i + 1..end], b"{", b"}", depth + 1, &mut cost)
                    {
                        groups.push((i + 1, i + 1 + len, depth + 1));
                        i += len + 2;
                        fill(&mut frames);
                    } else {
                        i += 1;
                        frames.push(OPEN_ENDED);
                    }
                }
                b'}' | b'^' | b'_' => i += 1,
                b'\\' => {
                    let (name, after) = control_sequence(latex, i + 1, end);
                    i = after;
                    match name {
                        [] => break,
                        // A definition; `\char`, whose lexer panics
                        // before a letter that is not ASCII; `\buildrel`,
                        // which reads on to `\over` as text, not tokens.
                        b"def" | b"let" | b"futurelet" | b"newcommand" | b"renewcommand"
                        | b"providecommand" | b"char" | b"buildrel" => {
                            found.unbounded = true;
                            return found;
                        }
                        b"left" => {
                            i = skip_token(latex, i, end, &mut found.comments);
                            if let Some(len) = group_content(
                                &b[i..end],
                                b"\\left",
                                b"\\right",
                                depth + 1,
                                &mut cost,
                            ) {
                                groups.push((i, i + len, depth + 1));
                                i = skip_token(latex, i + len + 6, end, &mut found.comments);
                                fill(&mut frames);
                            } else {
                                frames.push(OPEN_ENDED);
                            }
                        }
                        b"begingroup" => {
                            if let Some(len) = group_content(
                                &b[i..end],
                                b"\\begin",
                                b"\\end",
                                depth + 1,
                                &mut cost,
                            ) {
                                groups.push((i, i + len, depth + 1));
                                i += len + 4;
                                fill(&mut frames);
                            } else {
                                frames.push(OPEN_ENDED);
                            }
                        }
                        b"begin" => {
                            // The environment's name, then — for the three
                            // that read a number, and not strictly — that
                            // argument, before the search for the end.
                            let cut = |i: &mut usize, cost: &mut Cost| {
                                if *i >= end || b[*i] != b'{' {
                                    return None;
                                }
                                let len =
                                    group_content(&b[*i + 1..end], b"{", b"}", depth + 1, cost)?;
                                *i += len + 2;
                                Some(&b[*i - len - 1..*i - 1])
                            };
                            let content = cut(&mut i, &mut cost).and_then(|environment| {
                                if matches!(environment, b"alignat" | b"alignat*" | b"alignedat") {
                                    i = skip_space(latex, i, end);
                                    let number = cut(&mut i, &mut cost)?;
                                    // Its lexer steps one BYTE past a
                                    // character that is not a digit.
                                    if number.first().is_some_and(|c| !c.is_ascii()) {
                                        found.unbounded = true;
                                    }
                                }
                                let open = [b"\\begin{", environment, b"}"].concat();
                                let close = [b"\\end{", environment, b"}"].concat();
                                let len =
                                    group_content(&b[i..end], &open, &close, depth + 1, &mut cost)?;
                                Some((len, close.len()))
                            });
                            if let Some((len, close)) = content {
                                groups.push((i, i + len, depth + 1));
                                i += len + close;
                                fill(&mut frames);
                            } else {
                                frames.push(OPEN_ENDED);
                            }
                        }
                        b"right" | b"end" | b"endgroup" => frames.push(OPEN_ENDED),
                        // Two arguments it lexes and never reads — any
                        // token at all, `\left` or `}` included — and one
                        // it does.
                        b"sideset" => {
                            for _ in 0..2 {
                                i = skip_space(latex, i, end);
                                if i < end && b[i] == b'{' {
                                    i += 1;
                                    if let Some(len) =
                                        group_content(&b[i..end], b"{", b"}", depth + 1, &mut cost)
                                    {
                                        i += len + 1;
                                    }
                                } else {
                                    i = skip_token(latex, i, end, &mut found.comments);
                                }
                            }
                            frames.push(1);
                        }
                        // The lexer takes these itself where it allows
                        // them, and they are an error anywhere else.
                        b"limits" | b"nolimits" | b"hline" | b"hdashline" => {}
                        // One delimiter, which is a token and no argument.
                        b"big" | b"bigl" | b"bigr" | b"bigm" | b"Big" | b"Bigl" | b"Bigr"
                        | b"Bigm" | b"bigg" | b"biggl" | b"biggr" | b"biggm" | b"Bigg"
                        | b"Biggl" | b"Biggr" | b"Biggm" | b"middle" => {
                            i = skip_token(latex, i, end, &mut found.comments);
                            fill(&mut frames);
                        }
                        b"sqrt" | b"xleftarrow" | b"xrightarrow" | b"\\" | b"cr" => {
                            // An optional argument is a group of its own
                            // and fills nothing. Unbalanced, the parser has
                            // still stepped past the bracket.
                            if i < end && b[i] == b'[' {
                                i += 1;
                                if let Some(len) =
                                    group_content(&b[i..end], b"[", b"]", depth + 1, &mut cost)
                                {
                                    groups.push((i, i + len, depth + 1));
                                    i += len + 1;
                                }
                            }
                            if matches!(name, b"\\" | b"cr") {
                                fill(&mut frames);
                            } else {
                                frames.push(1);
                            }
                        }
                        // A control symbol: `\{`, `\,`, `\ `. One not
                        // listed is not the parser's either.
                        [c] if !c.is_ascii_alphabetic() => {
                            if b",>:;~!#%&$_|{} \t\n\r".contains(c) {
                                fill(&mut frames);
                            } else {
                                frames.push(OPEN_ENDED);
                            }
                        }
                        word => match arity(word) {
                            Some(0) => fill(&mut frames),
                            Some(wants) => frames.push(wants),
                            None => frames.push(OPEN_ENDED),
                        },
                    }
                }
                _ => {
                    i += char_len(&b[..end], i);
                    fill(&mut frames);
                }
            }
            found.bare = found.bare.max(frames.len() as u32);
        }
    }
    found
}

/// Parse events until `stop` (or the end), returning them as one row.
///
/// `depth` counts nesting, not events: it rises on the way into a group and
/// past [`MAX_DEPTH`] the parse is refused. See that constant for why.
fn parse_group(
    events: &[Event],
    cursor: &mut usize,
    stop: Option<usize>,
    depth: u32,
) -> Option<MathExpr> {
    if depth > MAX_DEPTH {
        return None;
    }
    let mut parts: Vec<MathExpr> = Vec::new();
    while *cursor < events.len() {
        if Some(*cursor) == stop {
            break;
        }
        if matches!(events[*cursor], Event::End) {
            break;
        }
        if let Step::Node(e) = parse_one(events, cursor, depth)? {
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
fn parse_one(events: &[Event], cursor: &mut usize, depth: u32) -> Option<Step> {
    if depth > MAX_DEPTH {
        return None;
    }
    let ev = events.get(*cursor)?;
    *cursor += 1;
    let out = match ev {
        Event::Content(c) => Step::Node(content(c)),
        Event::Begin(g) => Step::Node(begin(events, cursor, g, depth + 1)?),
        Event::Visual(Visual::Fraction(_)) => {
            let num = Box::new(operand(events, cursor, depth + 1)?);
            let den = Box::new(operand(events, cursor, depth + 1)?);
            Step::Node(MathExpr::Frac { num, den })
        }
        Event::Visual(Visual::SquareRoot) => Step::Node(MathExpr::Sqrt {
            radicand: Box::new(operand(events, cursor, depth + 1)?),
            index: None,
        }),
        Event::Visual(Visual::Root) => {
            // `\sqrt[n]{x}` — the index comes first in the stream.
            let index = Box::new(operand(events, cursor, depth + 1)?);
            let radicand = Box::new(operand(events, cursor, depth + 1)?);
            Step::Node(MathExpr::Sqrt {
                radicand,
                index: Some(index),
            })
        }
        Event::Visual(Visual::Negation) => {
            // Rendered as the operand with a combining slash: the layout
            // engine's problem, so keep the operand and let it decide.
            Step::Node(operand(events, cursor, depth + 1)?)
        }
        Event::Script { ty, position } => {
            let base = Box::new(operand(events, cursor, depth + 1)?);
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
                ScriptType::Subscript => {
                    (Some(Box::new(operand(events, cursor, depth + 1)?)), None)
                }
                ScriptType::Superscript => {
                    (None, Some(Box::new(operand(events, cursor, depth + 1)?)))
                }
                ScriptType::SubSuperscript => {
                    let sub = Box::new(operand(events, cursor, depth + 1)?);
                    let sup = Box::new(operand(events, cursor, depth + 1)?);
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
fn operand(events: &[Event], cursor: &mut usize, depth: u32) -> Option<MathExpr> {
    loop {
        let ev = events.get(*cursor)?;
        if matches!(ev, Event::Space { .. } | Event::StateChange(_)) {
            *cursor += 1;
            continue;
        }
        return match parse_one(events, cursor, depth)? {
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
fn begin(events: &[Event], cursor: &mut usize, g: &Grouping, depth: u32) -> Option<MathExpr> {
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
        let inner = parse_group(events, cursor, None, depth)?;
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
                if let Step::Node(e) = parse_one(events, cursor, depth)? {
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

    /// The depth guard must run BEFORE `pulldown-latex`, whose cost is
    /// quadratic in group nesting: `{`×15 000 took a second in release, and
    /// the debug build here is slower still. Refused by the scan, each of
    /// these is microseconds; reaching the parser, the first alone is tens of
    /// seconds in a debug build, so the bound has room for any machine.
    #[test]
    fn deep_nesting_is_refused_before_the_parser_runs() {
        let n = 20_000;
        let shapes = [
            format!("{}x{}", "{".repeat(n), "}".repeat(n)),
            format!("{}x{}", r"\sqrt{".repeat(n), "}".repeat(n)),
            format!("{}x{}", r"\frac{".repeat(n), "}{y}".repeat(n)),
            format!("{}{}", "_{a".repeat(n), "}".repeat(n)),
            format!("{}x{}", r"\left(".repeat(n), r"\right)".repeat(n)),
            format!(
                "{}x{}",
                r"\begin{matrix}".repeat(n),
                r"\end{matrix}".repeat(n)
            ),
        ];
        let start = std::time::Instant::now();
        for s in &shapes {
            assert!(nesting(s).refused(), "{:?}…", &s[..20]);
            assert_eq!(parse(s), None);
        }
        assert!(
            start.elapsed() < std::time::Duration::from_secs(2),
            "deep math reached the parser: {:?}",
            start.elapsed()
        );
    }

    /// A bare-argument chain recurses inside `pulldown-latex` and overflowed
    /// the stack — 150 deep was enough on a 2 MiB debug thread, which is what
    /// this test runs on, so reaching the parser aborts the test binary.
    #[test]
    fn a_bare_argument_chain_is_refused_before_the_parser_runs() {
        let n = 5_000;
        for s in [
            format!("{}x", r"\sqrt ".repeat(n)),
            format!("{}x", r"\frac 1".repeat(n)),
            format!("{}x y", r"\frac".repeat(n)),
            format!("{}x", r"\frac{1}".repeat(n)),
            format!("{}x", r"\mathrm ".repeat(n)),
        ] {
            assert!(nesting(&s).bare > MAX_BARE_CHAIN, "{:?}…", &s[..20]);
            assert_eq!(parse(&s), None);
        }
    }

    /// `binary_search` needs the lists sorted, and a typo'd order would
    /// silently hold a symbol open as a command (refusing math).
    #[test]
    fn the_argument_lists_are_sorted_and_disjoint() {
        let lists = [NO_ARGUMENT, ONE_ARGUMENT, TWO_ARGUMENTS, THREE_ARGUMENTS];
        for list in lists {
            assert!(list.windows(2).all(|w| w[0] < w[1]));
        }
        let all: Vec<&str> = lists.concat();
        let unique: std::collections::HashSet<&str> = all.iter().copied().collect();
        assert_eq!(all.len(), unique.len());
    }

    /// Every event the parser gives for `s`, or `None` where it reports an
    /// error.
    fn events(s: &str) -> Option<Vec<String>> {
        let storage = Storage::new();
        Parser::new(s, &storage)
            .map(|event| event.ok().map(|e| format!("{e:?}")))
            .collect()
    }

    /// The scan closes a frame when a listed command has had its count of
    /// arguments, and a count one short is the stack overflow the scan is
    /// there to stop. So each list is held to the parser itself: a command
    /// errors with an argument missing, parses with them all, and then has
    /// read nothing more — whatever follows parses as it does alone.
    #[test]
    fn the_argument_lists_say_what_the_parser_reads() {
        let lists = [NO_ARGUMENT, ONE_ARGUMENT, TWO_ARGUMENTS, THREE_ARGUMENTS];
        for (wants, list) in lists.iter().enumerate() {
            for word in *list {
                let with = |arguments: usize| format!("\\{word}{}", "{red}".repeat(arguments));
                let whole = with(wants);
                let alone = events(&whole).unwrap_or_else(|| panic!("{whole} is an error"));
                assert!(!alone.is_empty(), "{whole} gives no event");
                if wants > 0 {
                    assert_eq!(events(&with(wants - 1)), None, "{word} reads fewer");
                }
                for next in [
                    " b",
                    " [b]",
                    "{b}",
                    " 1.5pt",
                    "(",
                    "'",
                    "\\beta",
                    " plus 1em",
                    "=",
                    "|",
                ] {
                    let mut both = alone.clone();
                    both.extend(events(next).unwrap());
                    assert_eq!(
                        events(&format!("{whole}{next}")),
                        Some(both),
                        "{whole} reads on into {next:?}"
                    );
                }
            }
        }
        // And the control symbols the scan takes for a token.
        for symbol in b",>:;~!#%&$_|{} " {
            let symbol = format!("\\{}", *symbol as char);
            let alone = events(&symbol).unwrap_or_else(|| panic!("{symbol} is an error"));
            let mut both = alone.clone();
            both.extend(events("[b]").unwrap());
            assert_eq!(events(&format!("{symbol}[b]")), Some(both), "{symbol}");
        }
    }

    #[test]
    fn nesting_within_the_limit_still_parses() {
        let n = 100;
        let s = format!("{}x{}", r"\sqrt{".repeat(n), "}".repeat(n));
        let found = nesting(&s);
        assert_eq!((found.groups, found.bare), (n as u32, 1));
        assert!(parse(&s).is_some());
        // Near both limits, the walker's own guard decides, without a crash.
        let _ = parse(&format!("{}x{}", "{".repeat(256), "}".repeat(256)));
        let _ = parse(&format!("{}x", r"\sqrt".repeat(64)));
        // Escaped braces are delimiters, not groups; `\\` escapes itself.
        assert_eq!(nesting(r"\{\{\{x\}\}\}").groups, 0);
        assert_eq!(nesting(r"a\\{b}").groups, 1);
        assert_eq!(nesting(r"\left(\frac{a}{b}\right)").groups, 2);
        // Siblings and ordinary formulas are short chains.
        assert_eq!(nesting(r"\frac{a}{b}\frac{c}{d}").bare, 1);
        assert_eq!(nesting(r"\frac 12\frac 34").bare, 1);
        assert_eq!(nesting(&r"\alpha+\beta\cdot x_1 ".repeat(500)).bare, 0);
        // Symbols nest nothing: any number of them in a row is no chain.
        let symbols = format!("{}x", r"\alpha ".repeat(500));
        assert_eq!(nesting(&symbols).bare, 0);
        assert!(parse(&symbols).is_some(), "shown as math, not source");
        assert_eq!(
            nesting(&r"\hat a & \bar b \\ ".repeat(500)).bare,
            1,
            "a matrix row break and a cell end a chain"
        );
    }

    /// A command that has its arguments is done, however many follow it:
    /// what real formulas are made of is no chain, and renders.
    #[test]
    fn commands_with_their_arguments_are_no_chain() {
        for (period, bare) in [
            (r"\sqrt\alpha ", 1),
            (r"\vec{a}\cdot\hat b+", 1),
            (r"\frac{\partial f}{\partial x}\,\mathrm{d}x ", 1),
            (
                r"\sqrt[3]{x}\left(\frac ab\right)\big|\sum\limits_{i}^{n}",
                1,
            ),
            (
                r"\mathbf{x}_{\text{in}}^{\top}\overline{y}\overset{!}{=}",
                1,
            ),
            (r"\frac\sqrt\hat a b ", 3),
            (
                "\\begin{pmatrix}\\hat a & \\frac12 \\\\[2pt] \\hline c & d\\end{pmatrix}",
                1,
            ),
        ] {
            let s = period.repeat(300);
            assert_eq!(nesting(&s).bare, bare, "{period}");
            assert!(parse(&s).is_some(), "{period} is shown as source");
        }
        // One the scan has no count for is held open to the end of its
        // group; a command after it still closes on its own argument.
        assert_eq!(nesting(r"\kern 3pt\hat a\hat b{\hat c}\hat d").bare, 2);
        assert_eq!(nesting(&r"{\kern 3pt\kern 3pt}".repeat(70)).bare, 2);
        assert_eq!(nesting(&r"\kern 3pt".repeat(70)).bare, 70);
    }

    /// What the parser reads as nothing, the scan reads as nothing: each of
    /// these hid a chain from a scan that read bytes, and each chain is
    /// counted now. (The parser itself is run on them, in a process of
    /// their own, by `tests/math_stack.rs`.)
    #[test]
    fn a_chain_is_counted_through_what_the_parser_skips() {
        let n = 3_000;
        for link in [
            "\\sqrt%c\n",
            "\\sqrt\u{a0}",
            "\\sqrt\u{2003}",
            "\\sqrt\x0b",
            r"\frac+",
            r"\frac(",
            r"\frac~",
            r"\frac\frac a b",
            r"\sqrt[ab]",
            r"\sqrt[+]",
            r"\sideset{}{}",
            r"\sideset\left.",
            r"\sideset}\right",
            r"\genfrac{}{}{}{}1",
            r"\overset+",
            r"\xrightarrow[ab]",
            r"\frac\left(a\right)",
            r"\frac\begin{matrix}a\end{matrix}",
            r"\frac\big(",
            r"\relax ",
        ] {
            let s = format!("{}x", link.repeat(n));
            assert!(nesting(&s).refused(), "{link:?}");
            assert_eq!(parse(&s), None);
        }
        // A macro, a long run of comments, and the three shapes on which
        // the parser's lexer panics are refused whatever else they hold.
        for s in [
            r"\def\a{x}\a",
            r"\let\a=x \a",
            r"\newcommand{\a}{x}\a",
            r"\char`a",
            r"\buildrel a\over b",
            "\\kern 1pt\u{a0}x",
            "\\begin{alignat}{\u{e9}}x\\end{alignat}",
            &"%\n".repeat(65),
        ] {
            assert!(nesting(s).refused(), "{s:?}");
        }
        assert!(!nesting(&"%\n".repeat(64)).refused());
        assert!(!nesting("a\u{a0}+ b \u{a0} 1\u{a0}c").refused());
        assert!(parse("5\u{a0}\\text{kg}").is_some());
        assert!(nesting("a & 1\u{a0}\\\\ b").refused());
    }

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

    /// Nesting deeper than [`MAX_DEPTH`] is refused rather than recursed
    /// into. The walker is mutually recursive with no tail call to hope for,
    /// and the release binary is `panic = "abort"` — a stack overflow there
    /// is not an error a frontend can render, it is the reader gone.
    #[test]
    fn absurd_nesting_yields_none_rather_than_a_stack_overflow() {
        let deep = format!("{}x{}", r"\frac{".repeat(10_000), "}{1}".repeat(10_000));
        assert!(parse(&deep).is_none(), "10,000 deep must refuse");
        // The cap is far above any real equation, so ordinary math is
        // untouched by it.
        assert!(parse(r"\frac{\frac{a}{b}}{\frac{c}{d}}").is_some());
    }
}

/// The one-row rendering of an expression, as a plain string.
///
/// This is a **text transformation, not layout**: it takes no width, produces
/// no cells and no rows, and so belongs in the core beside entity decoding and
/// smart punctuation rather than in a frontend. That matters because inline
/// math has to enter `Document::text` — the display text is authoritative, and
/// painting glyphs that differ from it would desynchronise wrapping, selection
/// and search all at once.
///
/// Scripts prefer their Unicode forms (`x²`, `aᵢ`) and fall back to `^`/`_`
/// with parentheses where precedence demands them.
#[must_use]
pub fn inline_text(expr: &MathExpr) -> String {
    match expr {
        MathExpr::Sym { text, class } => {
            if matches!(class, MathClass::BinaryOp | MathClass::Relation) && !text.is_empty() {
                format!(" {text} ")
            } else {
                text.to_string()
            }
        }
        MathExpr::Row(parts) => parts.iter().map(inline_text).collect(),
        MathExpr::Frac { num, den } => {
            format!("{}/{}", grouped(num), grouped(den))
        }
        MathExpr::Sqrt { radicand, index } => {
            let idx = index.as_deref().map(inline_text).unwrap_or_default();
            format!("{idx}\u{221a}{}", grouped(radicand))
        }
        MathExpr::Script { base, sub, sup, .. } => {
            let mut out = inline_text(base);
            if let Some(s) = sub {
                if let Some(u) = unicode_script(s, SUBS) {
                    out.push_str(&u);
                } else {
                    out.push('_');
                    out.push_str(&grouped(s));
                }
            }
            if let Some(s) = sup {
                if let Some(u) = unicode_script(s, SUPERS) {
                    out.push_str(&u);
                } else {
                    out.push('^');
                    out.push_str(&grouped(s));
                }
            }
            out
        }
        MathExpr::Matrix { rows, delim } => {
            let body = rows
                .iter()
                .map(|r| r.iter().map(inline_text).collect::<Vec<_>>().join(" "))
                .collect::<Vec<_>>()
                .join("; ");
            let (l, r) = match delim {
                MatrixDelim::None => ("", ""),
                MatrixDelim::Paren => ("(", ")"),
                MatrixDelim::Bracket => ("[", "]"),
                MatrixDelim::Brace => ("{", "}"),
            };
            format!("{l}{body}{r}")
        }
    }
}

/// An operand, parenthesised when flattening would change what it means.
fn grouped(expr: &MathExpr) -> String {
    let text = inline_text(expr).trim().to_string();
    if expr.is_compound() {
        format!("({text})")
    } else {
        text
    }
}

const SUPERS: [(char, char); 14] = [
    ('0', '\u{2070}'),
    ('1', '\u{b9}'),
    ('2', '\u{b2}'),
    ('3', '\u{b3}'),
    ('4', '\u{2074}'),
    ('5', '\u{2075}'),
    ('6', '\u{2076}'),
    ('7', '\u{2077}'),
    ('8', '\u{2078}'),
    ('9', '\u{2079}'),
    ('+', '\u{207a}'),
    ('-', '\u{207b}'),
    ('n', '\u{207f}'),
    ('i', '\u{2071}'),
];

const SUBS: [(char, char); 14] = [
    ('0', '\u{2080}'),
    ('1', '\u{2081}'),
    ('2', '\u{2082}'),
    ('3', '\u{2083}'),
    ('4', '\u{2084}'),
    ('5', '\u{2085}'),
    ('6', '\u{2086}'),
    ('7', '\u{2087}'),
    ('8', '\u{2088}'),
    ('9', '\u{2089}'),
    ('+', '\u{208a}'),
    ('-', '\u{208b}'),
    ('n', '\u{2099}'),
    ('i', '\u{1d62}'),
];

/// The Unicode raised or lowered form, if EVERY character has one. `None` the
/// moment one does not — a half-translated script reads worse than `^(n+1)`.
fn unicode_script(expr: &MathExpr, table: [(char, char); 14]) -> Option<String> {
    let MathExpr::Sym { text, .. } = expr else {
        return None;
    };
    let mut out = String::new();
    for ch in text.trim().chars() {
        out.push(table.iter().find(|(from, _)| *from == ch)?.1);
    }
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod inline_tests {
    use super::*;

    #[test]
    fn a_simple_script_becomes_its_unicode_form() {
        let e = parse("E = mc^2").expect("parse");
        assert_eq!(inline_text(&e), "E = mc\u{b2}");
    }

    #[test]
    fn a_compound_script_falls_back_to_a_caret_with_parens() {
        let e = parse("x^{n+1}").expect("parse");
        assert_eq!(inline_text(&e), "x^(n + 1)");
    }

    #[test]
    fn a_nested_fraction_keeps_its_grouping() {
        let e = parse(r"\frac{\frac{a+b}{c}}{z}").expect("parse");
        assert_eq!(inline_text(&e), "((a + b)/c)/z");
    }
}
