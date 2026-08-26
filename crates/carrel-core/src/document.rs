//! The document model — space 2.
//!
//! See architecture.md (private notes repo) §1.2. This is the part that **cannot be copied from
//! Helix, Zed, Neovim, or VS Code**, because those are editors and an editor has
//! source space == document space. A markdown reader does not.
//!
//! # The invariant that defines [`Document::text`]
//!
//! > `Document::text` contains exactly the characters the renderer will emit as
//! > *content* cells, in reading order, **unwrapped**.
//! >
//! > Positional decoration — bullets, quote bars, table borders, wrap
//! > indicators, indent guides — is **not** in it.
//! >
//! > Substituted content the user *sees* — entity-decoded `&`, smart quotes, a
//! > link's visible text — **is**.
//!
//! That buys both halves at once: search never matches something invisible, and
//! never fails to match something visible-as-content.
//!
//! # Why [`Prov`] is mandatory
//!
//! `Event::Text` from pulldown-cmark is **not always a source substring**. Entity
//! references are decoded (`&amp;` → `&`) and `ENABLE_SMART_PUNCTUATION` rewrites
//! `.`, `-`, `"`, `'`. Byte lengths differ, so `doc = src - block_start` is false
//! in general and a provenance table is required to get back to the source.

use std::borrow::Cow;
use std::ops::Range;
use std::sync::{Arc, OnceLock};

use pulldown_cmark::{CodeBlockKind, Event, LinkType, Options, Parser, Tag, TagEnd};
use unicode_segmentation::UnicodeSegmentation;

use crate::highlight::{self, Token};
use crate::layout::{cluster_width, display_width};
use crate::position::{BlockIdx, DocByte, NodeId, SrcByte};

/// Tab stop, in display cells.
const TAB_STOP: u16 = 4;

/// Inline style flags. A bitset so runs coalesce cheaply.
///
/// The core stores **semantic** style, never a colour and never an ANSI code.
/// Each frontend maps these to SGR attributes or to CSS.
#[derive(Copy, Clone, PartialEq, Eq, Default, Debug)]
pub struct Style(pub u8);

impl Style {
    pub const NONE: Self = Self(0);
    pub const EMPHASIS: Self = Self(1 << 0);
    pub const STRONG: Self = Self(1 << 1);
    pub const CODE: Self = Self(1 << 2);
    pub const STRIKETHROUGH: Self = Self(1 << 3);
    pub const LINK: Self = Self(1 << 4);
    pub const SUPERSCRIPT: Self = Self(1 << 5);
    pub const SUBSCRIPT: Self = Self(1 << 6);
    pub const MATH: Self = Self(1 << 7);

    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
    #[must_use]
    const fn insert(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
    #[must_use]
    const fn remove(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }
}

/// Index into [`Document::links`].
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct LinkId(pub u32);

/// GFM extended autolinks for bare `www.` runs.
///
/// Absent upstream in pulldown-cmark, so hand-rolled to the GFM rules that
/// actually matter: the run ends at whitespace or `<`; trailing `?!.,:*_~` are
/// trimmed; and a trailing `)` is trimmed **only** when the parens are
/// unbalanced, so `www.example.com/a_(b)` keeps its closing paren while
/// `(www.example.com)` does not.
///
/// Returns byte ranges into `text` paired with the URL to register. The scheme
/// is `http://` per GFM — carrel does not guess at transport.
/// One `x^2^`-style run: where the delimiters sit and what they enclose.
struct ScriptRun {
    /// Byte index of the opening delimiter.
    open: usize,
    /// The enclosed text, delimiters excluded.
    content: Range<usize>,
    /// Byte index of the closing delimiter.
    close: usize,
    style: Style,
}

/// Find `x^2^` / `H~2~O` runs — the attached forms pulldown-cmark declines.
///
/// **The rules are narrow on purpose.** `~` and `^` are ordinary characters in
/// prose, so a permissive scan would eat things that are not scripts at all.
/// A run is recognised only when every one of these holds:
///
/// 1. **The opener is attached to the preceding character**, which must be
///    alphanumeric. This is precisely the case upstream skips — anything after
///    whitespace or at the start of a run either already parsed or is
///    unmatched, and either way is not ours to touch. It is also what keeps
///    `cp ~/src~/dst` alone: that `~` follows a space.
/// 2. **The content is non-empty, has no whitespace, and is at most
///    [`MAX_SCRIPT`] bytes.** Real scripts are short — `2`, `10`, `n+1`. The
///    whitespace rule alone stops a stray `~` from swallowing half a sentence
///    to reach the next one.
/// 3. **The content holds no delimiter of either kind**, so `~~strike~~`
///    (which upstream owns) and `a^b~c^` cannot be misread.
/// 4. **`~` is skipped when doubled** — `~~` is strikethrough, upstream's.
/// 5. **URL-shaped tokens are skipped whole.** A bare `https://…` is NOT
///    autolinked here (only `www.`, `<…>` and explicit links are), so it
///    reaches this scan as ordinary text — and turning
///    `…/wiki/X^2^` into `…/wiki/X2` would hand the reader a URL they cannot
///    use. Checked against the whitespace-delimited token, not the line.
///
/// Runs are returned in order and never overlap: the scan resumes after each
/// closing delimiter.
fn attached_scripts(text: &str) -> Vec<ScriptRun> {
    /// Longest content a script run may enclose. `H~2~O` is 1; a chemical
    /// formula or an exponent that runs past this is prose with punctuation
    /// in it, not a script.
    const MAX_SCRIPT: usize = 16;

    let bytes = text.as_bytes();
    let mut out: Vec<ScriptRun> = Vec::new();
    let mut at = 0usize;
    while at < bytes.len() {
        let delim = bytes[at];
        if delim != b'^' && delim != b'~' {
            at += 1;
            continue;
        }
        // Rule 4: `~~` belongs to strikethrough.
        if delim == b'~' && (bytes.get(at + 1) == Some(&b'~') || (at > 0 && bytes[at - 1] == b'~'))
        {
            at += 1;
            continue;
        }
        // Rule 1: attached to an alphanumeric. `char_indices` is not needed —
        // a UTF-8 continuation byte is never alphanumeric ASCII, and a
        // multi-byte letter is not one we can cheaply classify here, so the
        // conservative answer is the right one.
        if at == 0 || !bytes[at - 1].is_ascii_alphanumeric() {
            at += 1;
            continue;
        }
        // Rules 2 and 3: scan for the closer.
        let start = at + 1;
        let mut scan = start;
        let closed = loop {
            match bytes.get(scan) {
                None => break None,
                Some(&c) if c == delim => break Some(scan),
                // Whitespace, the other delimiter, or too far: not a script.
                Some(&c) if c.is_ascii_whitespace() || c == b'^' || c == b'~' => break None,
                Some(_) if scan - start >= MAX_SCRIPT => break None,
                Some(_) => scan += 1,
            }
        };
        let Some(close) = closed else {
            at += 1;
            continue;
        };
        if close == start {
            at += 1; // empty content
            continue;
        }
        // Rule 5, last because it is the only check that walks: a long line
        // with no whitespace would otherwise cost a token scan per `~` in it.
        // By here we have a complete candidate, so this runs once per run,
        // not once per delimiter.
        if let Some(token_end) = url_token_end(bytes, at) {
            at = token_end;
            continue;
        }
        out.push(ScriptRun {
            open: at,
            content: start..close,
            close,
            style: if delim == b'^' {
                Style::SUPERSCRIPT
            } else {
                Style::SUBSCRIPT
            },
        });
        at = close + 1;
    }
    out
}

/// If `at` falls inside a URL-shaped whitespace-delimited token, where that
/// token ends. Used to skip the token whole — see [`attached_scripts`] rule 5.
fn url_token_end(bytes: &[u8], at: usize) -> Option<usize> {
    let start = bytes[..at]
        .iter()
        .rposition(u8::is_ascii_whitespace)
        .map_or(0, |p| p + 1);
    let end = bytes[at..]
        .iter()
        .position(u8::is_ascii_whitespace)
        .map_or(bytes.len(), |p| at + p);
    let token = &bytes[start..end];
    let urlish = token.windows(3).any(|w| w == b"://")
        || token.starts_with(b"www.")
        || token.starts_with(b"mailto:");
    urlish.then_some(end)
}

fn extended_autolinks(text: &str) -> Vec<(Range<usize>, String)> {
    const TRAIL: &[char] = &['?', '!', '.', ',', ':', '*', '_', '~'];
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(rel) = text[i..].find("www.") {
        let start = i + rel;
        // Must begin at a boundary, or `youwww.example.com` would linkify.
        let at_boundary = start == 0
            || matches!(
                bytes[start - 1],
                b' ' | b'\t' | b'\n' | b'*' | b'_' | b'(' | b'~'
            );
        let mut end = start;
        while end < bytes.len() && !matches!(bytes[end], b' ' | b'\t' | b'\n' | b'<') {
            end += 1;
        }
        if at_boundary {
            let mut run = &text[start..end];
            loop {
                let cut = run.trim_end_matches(TRAIL);
                let cut =
                    if cut.ends_with(')') && cut.matches('(').count() < cut.matches(')').count() {
                        &cut[..cut.len() - 1]
                    } else {
                        cut
                    };
                if cut.len() == run.len() {
                    break;
                }
                run = cut;
            }
            // `www.` alone is not a host; there must be a dot after it.
            if run.len() > 4 && run[4..].contains('.') {
                out.push((start..start + run.len(), format!("http://{run}")));
            }
        }
        i = end.max(start + 4);
    }
    out
}

/// A styled run over [`Document::text`].
#[derive(Clone, Debug)]
pub struct Inline {
    pub doc: Range<u32>,
    pub style: Style,
    /// The link this run belongs to, if any. The URL itself lives in
    /// [`Document::links`], **never** in `Document::text` — searching for
    /// "example.com" must not match an invisible destination.
    pub link: Option<LinkId>,
}

/// What kind of thing a [`Node`] is.
///
/// Containers are never layout-atomic; leaves always are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeKind {
    // --- containers ---
    Root,
    List {
        ordered: bool,
    },
    BlockQuote,
    // --- leaves (layout-atomic) ---
    /// A list item's own text.
    ///
    /// In a *tight* list pulldown-cmark emits text directly inside `Item` with
    /// no enclosing `Paragraph`, so `Item` has to be able to be a leaf or the
    /// text belongs to no block at all. In a *loose* list the first paragraph
    /// reuses this node and any subsequent one opens a `Paragraph` block.
    Item,
    Paragraph,
    Heading {
        level: u8,
    },
    CodeBlock {
        lang: Option<Box<str>>,
    },
    /// The label line of a GFM alert (`> [!NOTE]`): the word "Note" as its
    /// own one-line block, synthetic but searchable — the source's `[!NOTE]`
    /// marker is consumed by the parser, so this restores the visible word.
    AlertLabel {
        kind: AlertKind,
    },
    Table {
        /// Max-content width per column, in display cells. **Width-independent**
        /// — a property of the text alone, like `Node::indent` — which is why a
        /// layout-looking quantity is allowed in the document model.
        cols: Box<[u16]>,
        /// Absolute doc-byte offset of every cell's first byte, stride
        /// `cols.len()`, header row first. A row missing trailing cells pads
        /// with its line-end offset, so the table is rectangular. Byte
        /// offsets are width-independent — same standing as `cols`.
        cell_starts: Box<[u32]>,
    },
    /// A display math block (`$$…$$`). The doc text is the **LaTeX source**,
    /// so math is searchable and no byte is discarded; turning it into art is
    /// the frontend's job entirely, because a box layout is cells and cells
    /// must never enter this crate.
    Math,
    /// The term line of a definition list.
    DefTerm,
    /// A definition body, indented under its [`NodeKind::DefTerm`]. The `:`
    /// marker is consumed by the parser, never rendered as prose.
    DefDetails,
    /// A YAML (`---`) or TOML (`+++`) frontmatter block.
    ///
    /// The doc text is the **raw body**, so the metadata is searchable and no
    /// byte is discarded. Splitting it into keys and values is the frontend's
    /// paint-time job, exactly like the table `│` separators — and deliberately
    /// NOT a YAML parse, because a *reader* must never fail on exotic YAML.
    Metadata {
        /// Max-content display width of the key column, capped at 16 cells.
        /// **Width-independent** — a property of the text alone, so it has the
        /// same standing here as `Table::cols` and `Node::indent`.
        key_col: u16,
    },
    /// A paragraph that contains **only** an image — the figure pattern. The
    /// node's doc text is the alt text, so the placeholder stays searchable.
    /// Pixels, dimensions, and protocols are the frontend's problem entirely:
    /// an image's row-height depends on the terminal's font pixel size, which
    /// this crate must never know.
    Image {
        url: Box<str>,
    },
    Rule,
}

/// GFM alert (admonition) kinds, `> [!NOTE]` through `> [!CAUTION]`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AlertKind {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

impl AlertKind {
    /// The visible label word.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Note => "Note",
            Self::Tip => "Tip",
            Self::Important => "Important",
            Self::Warning => "Warning",
            Self::Caution => "Caution",
        }
    }
}

/// What kind of list marker a [`Prefix`] renders.
///
/// The text is authoritative for painting; this is the semantic form, so the
/// GTK frontend can render an ordered item as `<li>` rather than reproducing
/// the terminal's `"10. "`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Marker {
    Bullet,
    Ordered(u64),
}

/// Decoration rendered before a block's **first** row.
///
/// Not part of [`Document::text`], therefore not searchable — searching for
/// `-` must not match every bullet in the document.
///
/// This is first-row-only, which is why quote bars are **not** here: a wrapped
/// blockquote needs its bar on every row, so quote depth is a separate property
/// ([`Document::quote_depth`]). architecture.md (private notes repo) §2.1 lumps the two
/// together and is wrong to.
#[derive(Clone, Debug)]
pub struct Prefix {
    /// Rendered text: `"- "`, `"10. "`, `"- [x] "`.
    pub text: Box<str>,
    /// Display cells. Equals `display_width(&text)`, stored to avoid remeasuring.
    pub width: u16,
    pub marker: Marker,
    /// `Some(done)` for a task-list item.
    pub task: Option<bool>,
}

/// A structural node. The tree is expressed via `parent` + `children`.
#[derive(Debug)]
pub struct Node {
    pub id: NodeId,
    pub parent: Option<NodeId>,
    /// Contiguous run in [`Document::nodes`].
    pub children: Range<u32>,
    pub kind: NodeKind,
    /// Slice of [`Document::text`]. For a container this spans all descendants.
    pub doc: Range<u32>,
    /// Slice of [`Document::source`], from pulldown-cmark's `into_offset_iter()`.
    pub src: Range<u32>,
    /// Style runs over `doc`, sorted and non-overlapping. Empty for containers.
    pub inlines: Box<[Inline]>,
    /// Semantic syntax tokens, sorted and non-overlapping — **lazy**. Computed
    /// on first request via [`Document::tokens`], because highlighting through
    /// `fancy-regex` measured ~230 KiB/s: a code-heavy document must not pay
    /// that on the open path. `Inline`'s bitset cannot carry syntax scopes,
    /// and overloading it would tangle two unrelated systems.
    tokens: OnceLock<Box<[Token]>>,
    /// Left inset in display cells. **Width-independent**, which is why a `u16`
    /// is allowed here: it is a property of list/quote nesting, not of layout.
    /// A width-*dependent* quantity (a height, a row count) must never appear in
    /// this crate — that is the API shape that ended Helix's frontend-agnostic
    /// view layer.
    pub indent: u16,
    /// Rendered before the first row's content. See [`Prefix`].
    pub prefix: Option<Prefix>,
    /// How many `BlockQuote`s enclose this block. The bar repeats on **every**
    /// row, so unlike [`Prefix`] this cannot be first-row decoration.
    pub quote_depth: u8,
    /// `Some` when this block sits inside a GFM alert: the painter colours
    /// the quote bars by kind. Width-independent semantics, like everything
    /// else here — colour choices stay in the frontends.
    pub alert: Option<AlertKind>,
    /// The display column where each enclosing quote's `│ ` begins, outermost
    /// first; `quote_cols.len() == quote_depth`. Bars are NOT at `d * 2`: a
    /// quote nested inside a list item hangs at the item's indent (Q14), so
    /// each column is recorded when its quote opens. Width-independent, like
    /// `indent`.
    pub quote_cols: Box<[u16]>,
    /// `Some(i)` iff this node is layout-atomic, i.e. `layout_order[i] == id`.
    pub block: Option<BlockIdx>,
}

/// One maximal run of uniform doc↔src correspondence.
#[derive(Clone, Debug)]
pub struct Prov {
    pub doc: Range<u32>,
    pub src: Range<u32>,
    pub kind: ProvKind,
}

/// One GFM task-list item, as [`Document::tasks`] reports it.
#[derive(Clone, Debug)]
pub struct TaskMark {
    pub done: bool,
    /// Doc byte of the item's first row.
    pub at: u32,
}

/// A `<details>…</details>` region: foldable like a section, with its
/// `<summary>` text as the visible fold header.
///
/// Doc bytes only — width-independent, like every coordinate here. The body
/// is *not* a span of its own because it is ordinary blocks; the fold span is
/// `(summary.end, end)`, mirroring how a heading's body runs from the heading's
/// own `doc.end` to its section end.
#[derive(Clone, Debug)]
pub struct DetailsRegion {
    /// The visible summary text. A region is only recorded when this is
    /// non-empty — without a summary there is nothing to keep visible, so
    /// there is no fold to offer.
    pub summary: Range<u32>,
    /// Just past the last byte of the foldable body.
    pub end: u32,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ProvKind {
    /// Affine, `doc.len() == src.len()`. The common case, ~99% of entries.
    Verbatim,
    /// Lengths differ: entity refs, smart punctuation, a `SoftBreak` rendered as
    /// a space. `to_src` collapses to `src.start`.
    Substituted,
    /// Display text with no source at all — table padding, separators.
    Synthetic,
}

/// A parsed markdown document.
#[derive(Debug)]
pub struct Document {
    /// The raw markdown, verbatim. Never mutated.
    pub source: Arc<str>,
    /// **Space 2.** The flattened, unwrapped display text. The only thing search
    /// ever runs over. See the module docs for the invariant that defines it.
    pub text: String,
    /// Arena of all nodes, in document order.
    pub nodes: Box<[Node]>,
    /// Layout-atomic nodes in reading order. Contiguous and non-overlapping in
    /// doc space. [`BlockIdx`] indexes **this**, not `nodes`.
    pub layout_order: Box<[NodeId]>,
    /// Link destinations, indexed by [`LinkId`], in document order.
    pub links: Box<[Box<str>]>,
    /// Parallel to `links`: which entries are `[[wikilinks]]`. Private —
    /// [`Document::is_wikilink`] is the API.
    wiki: Box<[bool]>,
    /// Sorted by `doc.start`, contiguous, non-overlapping.
    prov: Box<[Prov]>,
    /// `<details>` regions, in document order. Nested regions appear both
    /// alone and inside their parent's span; the fold layer resolves that,
    /// the same way it resolves nested headings.
    pub details: Box<[DetailsRegion]>,
}

impl Document {
    /// Parse markdown into the document model.
    #[must_use]
    pub fn parse(source: &str) -> Self {
        Builder::new(source).run()
    }

    /// Map a doc-space offset back to a source offset. O(log P).
    ///
    /// Search never calls this. It exists for "open the source at this point"
    /// and for re-anchoring after a reload.
    #[must_use]
    pub fn to_src(&self, d: DocByte) -> SrcByte {
        let i = self.prov.partition_point(|p| p.doc.end <= d.0);
        match self.prov.get(i) {
            None => SrcByte(self.source.len() as u32),
            Some(p) => match p.kind {
                ProvKind::Verbatim => SrcByte(p.src.start + (d.0 - p.doc.start)),
                ProvKind::Substituted | ProvKind::Synthetic => SrcByte(p.src.start),
            },
        }
    }

    /// Whether this link came from `[[wikilink]]` syntax. Wikilink targets
    /// are note names, not URIs — the frontends resolve them against the
    /// tree; the core only records what kind of thing the destination is.
    #[must_use]
    pub fn is_wikilink(&self, id: LinkId) -> bool {
        self.wiki.get(id.0 as usize).copied().unwrap_or(false)
    }

    /// Resolve a `#fragment` to the doc offset of its heading, GitHub-style:
    /// lowercase, keep letters/digits/hyphens/underscores, spaces become
    /// hyphens, everything else drops, duplicates take `-1`, `-2`, … in
    /// document order. Byte offsets in, byte offsets out — both frontends
    /// resolve fragments through this one function.
    #[must_use]
    pub fn fragment_target(&self, fragment: &str) -> Option<u32> {
        let mut seen: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        for node in &self.nodes {
            if !matches!(node.kind, NodeKind::Heading { .. }) {
                continue;
            }
            let text = &self.text[node.doc.start as usize..node.doc.end as usize];
            let base = slugify(text);
            let n = seen.entry(base.clone()).or_insert(0);
            let slug = if *n == 0 { base } else { format!("{base}-{n}") };
            *n += 1;
            if slug == fragment {
                return Some(node.doc.start);
            }
        }
        None
    }

    /// The headings enclosing doc byte `at`, outermost first — the derived
    /// section index (spec: section-index design, notes repo). Empty before
    /// the first heading. Levels along the path strictly increase but are
    /// not necessarily consecutive: a document that skips H2 gets the path
    /// it wrote. Section structure is **by level only** — a heading inside a
    /// blockquote or list participates exactly like any other, which is what
    /// every outline tool does.
    ///
    /// Derived per call, never stored, like the outline it feeds: one walk
    /// over the headings, a stack where level *k* pops everything ≥ *k*.
    #[must_use]
    pub fn section_path(&self, at: u32) -> Vec<NodeId> {
        let mut stack: Vec<(u8, NodeId)> = Vec::new();
        for node in &self.nodes {
            let NodeKind::Heading { level } = node.kind else {
                continue;
            };
            if node.doc.start > at {
                break;
            }
            while stack.last().is_some_and(|&(l, _)| l >= level) {
                stack.pop();
            }
            stack.push((level, node.id));
        }
        stack.into_iter().map(|(_, id)| id).collect()
    }

    /// Where `heading`'s section ends: the `doc.start` of the next heading
    /// with level ≤ its own, else `text.len()`. The fold span — defined in
    /// the same place as [`Self::section_path`] so a breadcrumb and a fold
    /// can never disagree about what a section is.
    #[must_use]
    pub fn section_end(&self, heading: NodeId) -> u32 {
        let node = &self.nodes[heading.0 as usize];
        debug_assert!(
            matches!(node.kind, NodeKind::Heading { .. }),
            "section_end takes a heading"
        );
        let NodeKind::Heading { level } = node.kind else {
            return u32::try_from(self.text.len()).unwrap_or(u32::MAX);
        };
        self.nodes[heading.0 as usize + 1..]
            .iter()
            .find_map(|n| match n.kind {
                NodeKind::Heading { level: l } if l <= level => Some(n.doc.start),
                _ => None,
            })
            .unwrap_or_else(|| u32::try_from(self.text.len()).unwrap_or(u32::MAX))
    }

    /// The node for a block index.
    #[must_use]
    pub fn node_for_block(&self, b: BlockIdx) -> &Node {
        &self.nodes[self.layout_order[b.get()].get()]
    }

    /// Which block contains a doc offset. O(log B).
    #[must_use]
    pub fn block_at_doc(&self, d: DocByte) -> BlockIdx {
        let i = self
            .layout_order
            .partition_point(|n| self.nodes[n.get()].doc.end <= d.0);
        BlockIdx(i.min(self.layout_order.len().saturating_sub(1)) as u32)
    }

    #[must_use]
    pub fn block_count(&self) -> usize {
        self.layout_order.len()
    }

    /// How many block quotes enclose a block.
    ///
    /// Separate from [`Prefix`] because the quote bar repeats on every row of a
    /// wrapped block, where a list marker appears only on the first.
    #[must_use]
    pub fn quote_depth(&self, b: BlockIdx) -> u8 {
        self.node_for_block(b).quote_depth
    }

    /// Semantic syntax tokens for a block, highlighted **on first request**.
    ///
    /// Lazy because `fancy-regex` highlighting measured ~230 KiB/s — a typical
    /// fenced block costs single-digit milliseconds on its first paint, and
    /// nothing at all on open. Empty for prose, untagged code, unknown
    /// languages, and blocks over `highlight::MAX_HIGHLIGHT_BYTES`.
    #[must_use]
    pub fn tokens(&self, b: BlockIdx) -> &[Token] {
        let n = self.node_for_block(b);
        n.tokens.get_or_init(|| match &n.kind {
            NodeKind::CodeBlock { lang: Some(l) } => highlight::highlight(
                l,
                &self.text[n.doc.start as usize..n.doc.end as usize],
                n.doc.start,
            )
            .into_boxed_slice(),
            _ => Box::default(),
        })
    }

    /// The display text of one block.
    #[must_use]
    pub fn block_text(&self, b: BlockIdx) -> &str {
        let n = self.node_for_block(b);
        &self.text[n.doc.start as usize..n.doc.end as usize]
    }

    /// Every footnote reference occurrence, in reading order: `(name, byte)`.
    ///
    /// References are the literal `[^name]` runs a reader sees. Candidates
    /// inside code blocks are skipped — a code sample that mentions `[^x]`
    /// is prose about footnotes, not one.
    #[must_use]
    pub fn footnote_refs(&self) -> Vec<(Box<str>, u32)> {
        let mut out = Vec::new();
        let bytes = self.text.as_bytes();
        let mut i = 0usize;
        while i + 1 < bytes.len() {
            // Always advance past this byte (or past the whole candidate), so
            // a stray `[^` with no closer cannot spin forever.
            if bytes[i] == b'['
                && bytes[i + 1] == b'^'
                && let Some(close) = self.text[i + 2..].find(']')
            {
                let end = i + 2 + close;
                let name = &self.text[i + 2..end];
                let at = i as u32;
                i = end + 1;
                let node = self.node_for_block(self.block_at_doc(DocByte(at)));
                if !name.is_empty() && !matches!(node.kind, NodeKind::CodeBlock { .. }) {
                    out.push((name.into(), at));
                }
                continue;
            }
            i += 1;
        }
        out
    }

    /// Every GFM task-list item, in reading order.
    ///
    /// A task is any list item whose marker carried a `- [ ]` / `- [x]`;
    /// `at` is its block start, so a jump lands on the item's first row.
    #[must_use]
    pub fn tasks(&self) -> Vec<TaskMark> {
        (0..self.block_count())
            .map(|i| BlockIdx(i as u32))
            .filter_map(|b| {
                let n = self.node_for_block(b);
                let done = n.prefix.as_ref()?.task?;
                Some(TaskMark {
                    done,
                    at: n.doc.start,
                })
            })
            .collect()
    }

    /// Every footnote definition, in document order: `(name, byte)`.
    ///
    /// A definition's `[^name]:` label is decoration — it lives on the
    /// block's [`Prefix`], never in [`Document::text`] — so definitions are
    /// found through their prefixes and reported at their block start.
    #[must_use]
    pub fn footnote_defs(&self) -> Vec<(Box<str>, u32)> {
        (0..self.block_count())
            .map(|i| BlockIdx(i as u32))
            .filter_map(|b| {
                let n = self.node_for_block(b);
                let label: &str = n.prefix.as_ref()?.text.as_ref();
                let rest = label.strip_prefix("[^")?;
                let close = rest.find(']')?;
                rest[close + 1..]
                    .starts_with(':')
                    .then(|| (rest[..close].into(), n.doc.start))
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// One captured `push()` call, replayed when the table's column widths are
/// known. Replaying the *pushes* — not the strings — is what keeps inline
/// styles, links, and provenance intact through the buffering.
struct PushCall {
    text: String,
    src: Range<usize>,
    prov: ProvKind,
    style: Style,
    link: Option<LinkId>,
}

/// A table being buffered: cells of recorded pushes, per row.
#[derive(Default)]
struct TableRec {
    rows: Vec<Vec<Vec<PushCall>>>,
    row: Vec<Vec<PushCall>>,
    cell: Vec<PushCall>,
    /// Display column within the current cell, so tabs captured here expand
    /// relative to the CELL start. Expanding at capture is what keeps the
    /// width measurement and the replayed emission agreeing — a raw tab
    /// measures 1 cell but would replay as 1–4.
    cell_col: u16,
}

struct Open {
    kind: NodeKind,
    doc_start: u32,
    src_start: u32,
    indent: u16,
    prefix: Option<Prefix>,
    quote_depth: u8,
    alert: Option<AlertKind>,
    quote_cols: Vec<u16>,
    inlines: Vec<Inline>,
    /// The one image seen in this block, if any.
    image: Option<Box<str>>,
    /// Content other than a single image's alt text: text before or after the
    /// image, or a second image. Any of these keeps the block a paragraph.
    mixed: bool,
}

/// A `<details>` seen but not yet closed.
#[derive(Default)]
struct PendingDetails {
    /// Where the summary text began — just past `<summary>`'s `>`.
    summary_start: Option<u32>,
    /// Where it ended — where `</summary>` sat. Absent when no `<summary>`
    /// was written at all; such a region is dropped at close time, because a
    /// fold with nothing to keep visible is not a fold.
    summary_end: Option<u32>,
}

/// Strips tags from raw HTML, keeping the text content (research Q16:
/// an HTML block must not swallow its contents — a `<details>` body has to
/// stay readable and searchable). State survives across events because a
/// block-level tag can arrive split over several `Event::Html` lines.
///
/// This is a tag skipper, not an HTML parser: enough to keep text, drop
/// `<script>`/`<style>` bodies and comments, turn `<br>` into a break, and
/// surface an `<img>`'s alt text. Anything fancier belongs to a real
/// sanitizer the binary deliberately does not carry.
#[derive(Default)]
struct HtmlStrip {
    /// Buffers the current tag's text while inside `<...>`, so a tag split
    /// across events (or a quoted `>` inside an attribute) stays one tag.
    tag: Option<String>,
    in_comment: bool,
    /// Dropping a container's whole body until this closer appears.
    skip_until: Option<&'static str>,
    /// Tags completed by the most recent [`HtmlStrip::strip`] call, in order,
    /// each with where in that call's *output* the tag sat. The builder drains
    /// this to give structure-bearing tags (`<details>`, `<summary>`) to the
    /// document model; everything here is lowercase.
    completed: Vec<HtmlMark>,
}

/// One completed tag and where it landed.
#[derive(Debug)]
struct HtmlMark {
    /// The whole tag, lowercased — `<summary open>` as written.
    tag: String,
    /// Offset within the stripped output produced by the same `strip()` call,
    /// i.e. how much visible text preceded the tag in that chunk.
    at_out: usize,
}

impl HtmlStrip {
    /// Append `input`'s visible text to `out`.
    fn strip(&mut self, input: &str, out: &mut String) {
        for ch in input.chars() {
            if self.in_comment {
                if let Some(t) = &mut self.tag {
                    t.push(ch);
                    if t.ends_with("-->") {
                        self.in_comment = false;
                        self.tag = None;
                    }
                }
                continue;
            }
            if let Some(skip) = self.skip_until {
                if let Some(t) = &mut self.tag {
                    t.push(ch);
                    if t.to_ascii_lowercase().ends_with(skip) {
                        self.skip_until = None;
                        self.tag = Some(String::new()); // consume to the tag's `>`
                    }
                }
                continue;
            }
            match (&mut self.tag, ch) {
                (None, '<') => self.tag = Some(String::from("<")),
                (None, c) => out.push(c),
                (Some(t), '>') => {
                    t.push('>');
                    let tag = std::mem::take(t).to_ascii_lowercase();
                    self.tag = None;
                    self.completed.push(HtmlMark {
                        at_out: out.len(),
                        tag: tag.clone(),
                    });
                    if tag.starts_with("<!--") {
                        // `<!-- -->` already ended; a longer comment sets state.
                        if !tag.ends_with("-->") {
                            self.in_comment = true;
                            self.tag = Some(tag);
                        }
                    } else if tag.starts_with("<br") {
                        out.push('\n');
                    } else if tag.starts_with("<script") {
                        self.skip_until = Some("</script");
                        self.tag = Some(String::new());
                    } else if tag.starts_with("<style") {
                        self.skip_until = Some("</style");
                        self.tag = Some(String::new());
                    } else if tag.starts_with("<img") {
                        Self::push_attr(&tag, "alt", out);
                    }
                }
                (Some(t), c) => {
                    t.push(c);
                    if t == "<!--" {
                        self.in_comment = true;
                    }
                }
            }
        }
    }

    /// Append the value of `name="..."` (or `'...'`) from a raw tag, if any.
    fn push_attr(tag: &str, name: &str, out: &mut String) {
        let Some(i) = tag.find(&format!("{name}=")) else {
            return;
        };
        let rest = &tag[i + name.len() + 1..];
        let Some(quote) = rest.chars().next().filter(|c| *c == '"' || *c == '\'') else {
            return;
        };
        if let Some(end) = rest[1..].find(quote) {
            out.push_str(&rest[1..=end]);
        }
    }
}

/// GitHub's heading-anchor slug rules, near enough: lowercase; letters,
/// digits, `-` and `_` survive; spaces become `-`; the rest vanishes.
fn slugify(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.trim().chars() {
        if ch.is_alphanumeric() || ch == '-' || ch == '_' {
            out.extend(ch.to_lowercase());
        } else if ch == ' ' {
            out.push('-');
        }
    }
    out
}

fn alert_kind(k: pulldown_cmark::BlockQuoteKind) -> AlertKind {
    use pulldown_cmark::BlockQuoteKind as B;
    match k {
        B::Note => AlertKind::Note,
        B::Tip => AlertKind::Tip,
        B::Important => AlertKind::Important,
        B::Warning => AlertKind::Warning,
        B::Caution => AlertKind::Caution,
    }
}

struct Builder<'a> {
    source: &'a str,
    text: String,
    nodes: Vec<Node>,
    layout_order: Vec<NodeId>,
    prov: Vec<Prov>,
    open: Option<Open>,
    style: Style,
    /// Destinations seen so far; `Inline.link` indexes this.
    links: Vec<Box<str>>,
    /// Parallel to `links`: `true` when the entry came from `[[wikilink]]`
    /// syntax, whose destination is a note name rather than a URI.
    wiki: Vec<bool>,
    /// The link currently open, applied to every run pushed inside it.
    current_link: Option<LinkId>,
    /// Inside an image's alt text. Alt pushes must not mark the block mixed.
    in_image: bool,
    /// `Some` while a table is being buffered. `push()` records instead of
    /// appending, and `End(Table)` replays with alignment padding.
    table: Option<TableRec>,
    /// Carries tag state across the `Event::Html` lines of one HTML block.
    html: HtmlStrip,
    /// The `<details>` regions being accumulated, if any — a STACK, because
    /// one details can nest inside another and each summary belongs to its
    /// own opener. Spans HTML blocks, paragraphs and everything else between
    /// `<details>` and `</details>`; unlike [`Self::html`], whose state one
    /// block owns.
    pending_details: Vec<PendingDetails>,
    /// Finished regions, in document order.
    details: Vec<DetailsRegion>,
    /// Enclosing GFM alerts, innermost last (alerts can nest, in principle).
    alerts: Vec<AlertKind>,
    /// The column each enclosing quote's bar sits at, outermost first.
    quote_cols: Vec<u16>,
    /// `Some(label)` while inside a footnote definition whose first block has
    /// not yet opened; that block takes the label as its prefix.
    footnote: Option<String>,
    indent: u16,
    /// Display column since the last newline in `text`, for tab expansion.
    col: u16,
    /// Enclosing lists, innermost last. `Some(n)` is an ordered list whose next
    /// item is numbered `n`; `None` is a bullet list.
    lists: Vec<Option<u64>>,
    /// Indent contributed by the current item's marker, so `End(Item)` removes
    /// exactly what `Start(Item)` added. Marker widths differ per item, so a
    /// fixed decrement would drift.
    item_indent: Vec<u16>,
    quote_depth: u8,
}

impl<'a> Builder<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            text: String::with_capacity(source.len()),
            nodes: Vec::new(),
            layout_order: Vec::new(),
            prov: Vec::new(),
            open: None,
            style: Style::NONE,
            links: Vec::new(),
            wiki: Vec::new(),
            current_link: None,
            in_image: false,
            table: None,
            html: HtmlStrip::default(),
            pending_details: Vec::new(),
            details: Vec::new(),
            alerts: Vec::new(),
            quote_cols: Vec::new(),
            footnote: None,
            indent: 0,
            col: 0,
            lists: Vec::new(),
            item_indent: Vec::new(),
            quote_depth: 0,
        }
    }

    /// Max-content display width of a frontmatter key column, capped at 16.
    ///
    /// Deliberately **not** a YAML parse: split each line at its first `:` (or
    /// `=` for TOML). A line that does not split — a nested map, a list item, a
    /// block scalar, a comment — contributes nothing, because it renders as a
    /// raw continuation line under its parent. A *reader* must never fail on
    /// exotic YAML; the worst outcome here is an unaligned line, not a lost
    /// document.
    /// Push text, turning bare `www.` runs into links on the way.
    ///
    /// Skipped entirely inside an explicit link (`current_link` is set) — a
    /// `[label](url)` whose label happens to contain `www.` must not sprout a
    /// second, nested destination.
    /// True while the open block is a code block.
    ///
    /// **Every hand-rolled inline pass must check this.** pulldown-cmark does
    /// not run its own inline parsing inside a code block, so anything we add
    /// on top has to decline as well — a URL in a code sample became a live
    /// OSC 8 link for six days because `push_linkified` did not ask
    /// (found 2026-08-21, by probing before adding a second such pass).
    fn in_code_block(&self) -> bool {
        self.open
            .as_ref()
            .is_some_and(|o| matches!(o.kind, NodeKind::CodeBlock { .. }))
    }

    fn push_linkified(&mut self, t: &str, src: Range<usize>) {
        if self.current_link.is_some() || self.in_code_block() {
            self.push(t, src, ProvKind::Verbatim);
            return;
        }
        let hits = extended_autolinks(t);
        if hits.is_empty() {
            self.push_scripted(t, src, ProvKind::Verbatim);
            return;
        }
        let mut cursor = 0usize;
        for (range, url) in hits {
            if range.start > cursor {
                // The prefix's source range is unknowable byte-for-byte once
                // we split, so it is Substituted, not Verbatim — the same rule
                // entity decoding already follows.
                self.push_scripted(&t[cursor..range.start], src.clone(), ProvKind::Substituted);
            }
            let id = LinkId(self.links.len() as u32);
            // Raw, exactly like the explicit-link path above: control
            // characters are stripped at OSC 8 collection, which is the one
            // place that policy lives.
            self.links.push(url.into_boxed_str());
            self.wiki.push(false);
            let prev_link = self.current_link;
            let prev_style = self.style;
            self.current_link = Some(id);
            self.style = self.style.insert(Style::LINK);
            self.push(&t[range.clone()], src.clone(), ProvKind::Substituted);
            self.current_link = prev_link;
            self.style = prev_style;
            cursor = range.end;
        }
        if cursor < t.len() {
            self.push_scripted(&t[cursor..], src, ProvKind::Substituted);
        }
    }

    /// `x^2^` and `H~2~O` — the attached forms pulldown-cmark declines.
    ///
    /// Upstream only opens `^…^` / `~…~` at a word boundary, so the spellings
    /// people actually write stay literal (verified against 0.13.4, which is
    /// still the newest release as of 2026-08-21). This closes exactly that
    /// gap and nothing wider: the delimiters are consumed and the content
    /// becomes a `SUPERSCRIPT` / `SUBSCRIPT` run, which is what the parsed
    /// form already produces — so paint, search, and selection cannot tell
    /// the two apart.
    ///
    /// The scan is deliberately narrow, because `~` and `^` are ordinary
    /// characters in prose. See [`attached_scripts`] for the rules.
    fn push_scripted(&mut self, t: &str, src: Range<usize>, kind: ProvKind) {
        let runs = attached_scripts(t);
        if runs.is_empty() {
            self.push(t, src, kind);
            return;
        }
        let mut cursor = 0usize;
        for run in runs {
            if run.open > cursor {
                self.push(&t[cursor..run.open], src.clone(), ProvKind::Substituted);
            }
            let prev = self.style;
            self.style = self.style.insert(run.style);
            // The delimiters are dropped, so this run's source range is longer
            // than its text: Substituted, exactly like `Event::Code`.
            self.push(&t[run.content.clone()], src.clone(), ProvKind::Substituted);
            self.style = prev;
            cursor = run.close + 1;
        }
        if cursor < t.len() {
            self.push(&t[cursor..], src, ProvKind::Substituted);
        }
    }

    fn meta_key_col(body: &str) -> u16 {
        body.lines()
            .filter(|l| !l.starts_with([' ', '\t', '#', '-']))
            .filter_map(|l| l.split_once(':').or_else(|| l.split_once('=')))
            .map(|(k, _)| display_width(k.trim_end()))
            .max()
            .unwrap_or(0)
            .min(16)
    }

    fn opts() -> Options {
        Options::ENABLE_TABLES
            | Options::ENABLE_GFM
            | Options::ENABLE_FOOTNOTES
            | Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_TASKLISTS
            | Options::ENABLE_SMART_PUNCTUATION
            | Options::ENABLE_WIKILINKS
            | Options::ENABLE_MATH
            | Options::ENABLE_DEFINITION_LIST
            | Options::ENABLE_SUPERSCRIPT
            | Options::ENABLE_SUBSCRIPT
            | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
            | Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS
    }

    /// Append display text and record its provenance.
    fn push(&mut self, s: &str, src: Range<usize>, kind: ProvKind) {
        if s.is_empty() {
            return;
        }
        // Inside a table, capture the call instead of applying it: alignment
        // needs every cell's width before any cell can be emitted.
        if let Some(t) = self.table.as_mut() {
            let expanded = if kind == ProvKind::Synthetic {
                advance_col(s, &mut t.cell_col);
                Cow::Borrowed(s)
            } else {
                expand_tabs(s, &mut t.cell_col)
            };
            // Expansion changes byte length, which is Substituted territory —
            // the same rule the non-table path applies.
            let prov = if kind == ProvKind::Verbatim && expanded.len() != s.len() {
                ProvKind::Substituted
            } else {
                kind
            };
            t.cell.push(PushCall {
                text: expanded.into_owned(),
                src,
                prov,
                style: self.style,
                link: self.current_link,
            });
            return;
        }
        // Synthetic pushes are structure, not content: the block separator and
        // the table cell separator, which marks where one cell ends and paint
        // renders as a space. Expanding those would destroy the marker.
        let s: &str = &if kind == ProvKind::Synthetic {
            advance_col(s, &mut self.col);
            Cow::Borrowed(s)
        } else {
            expand_tabs(s, &mut self.col)
        };

        if !self.in_image
            && let Some(o) = self.open.as_mut()
            && o.image.is_some()
        {
            // Text after the image: "![a](u) and more" is prose, not a figure.
            o.mixed = true;
        }

        let start = self.text.len() as u32;
        self.text.push_str(s);
        let end = self.text.len() as u32;

        // Verbatim only when the byte lengths genuinely correspond. Tab
        // expansion changes the length, which lands the run in Substituted
        // exactly as entity decoding and smart punctuation already do.
        let kind = if kind == ProvKind::Verbatim && src.len() == s.len() {
            ProvKind::Verbatim
        } else if kind == ProvKind::Synthetic {
            ProvKind::Synthetic
        } else {
            ProvKind::Substituted
        };

        self.prov.push(Prov {
            doc: start..end,
            src: src.start as u32..src.end as u32,
            kind,
        });

        if self.style != Style::NONE
            && let Some(o) = self.open.as_mut()
        {
            o.inlines.push(Inline {
                doc: start..end,
                style: self.style,
                link: self.current_link,
            });
        }
    }

    /// Replay a buffered table with alignment padding.
    ///
    /// Column widths are **max-content display widths** — width-independent,
    /// so this is space-2 construction like tab expansion, not layout. Each
    /// table row becomes one logical line whose cells are padded with
    /// synthetic spaces; a row wider than a viewport wraps through the
    /// ordinary reflow layer later, complete and searchable, never truncated.
    fn flush_table(&mut self, src_end: usize) {
        let Some(t) = self.table.take() else { return };
        let rows = t.rows;

        // Max-content width per column.
        let mut cols: Vec<u16> = Vec::new();
        for row in &rows {
            for (c, cell) in row.iter().enumerate() {
                let w = cell
                    .iter()
                    .fold(0u16, |a, p| a.saturating_add(display_width(&p.text)));
                if c == cols.len() {
                    cols.push(w);
                } else if let Some(slot) = cols.get_mut(c) {
                    *slot = (*slot).max(w);
                }
            }
        }

        // Replay. A three-cell gap between columns, so the paint-time
        // separator sits centred with a space either side.
        let ncols = cols.len();
        let mut cell_starts: Vec<u32> = Vec::with_capacity(rows.len() * ncols);
        for row in rows {
            let n = row.len();
            for (c, cell) in row.into_iter().enumerate() {
                cell_starts.push(self.text.len() as u32);
                let mut w = 0u16;
                for call in cell {
                    w = w.saturating_add(display_width(&call.text));
                    let (style, link) = (self.style, self.current_link);
                    self.style = call.style;
                    self.current_link = call.link;
                    self.push(&call.text, call.src, call.prov);
                    self.style = style;
                    self.current_link = link;
                }
                // Pad every cell but the row's last to its column width.
                if c + 1 < n {
                    let target = cols.get(c).copied().unwrap_or(w);
                    let pad = usize::from(target.saturating_sub(w)) + 3;
                    self.push(&" ".repeat(pad), src_end..src_end, ProvKind::Synthetic);
                }
            }
            // Missing cells sit at the line end, keeping the stride.
            for _ in n..ncols {
                cell_starts.push(self.text.len() as u32);
            }
            self.push("\n", src_end..src_end, ProvKind::Synthetic);
        }

        if let Some(o) = self.open.as_mut() {
            o.kind = NodeKind::Table {
                cols: cols.into_boxed_slice(),
                cell_starts: cell_starts.into_boxed_slice(),
            };
        }
    }

    /// Is the currently open block an `Item` that has no text yet?
    fn open_item_is_empty(&self) -> bool {
        self.open
            .as_ref()
            .is_some_and(|o| o.kind == NodeKind::Item && o.doc_start as usize == self.text.len())
    }

    /// Feed one completed HTML tag to the `<details>` state machine.
    ///
    /// `at` is the tag's position in doc space — where the visible text it
    /// delimits starts or stops. Tags that carry no structure for this model
    /// are ignored; the stripper already decided their text's fate. Openers
    /// push and closers pop, so a nested `<summary>` belongs to the region
    /// it was written in rather than clobbering its parent's.
    fn details_mark(&mut self, tag: &str, at: u32) {
        if tag.starts_with("</details") {
            if let Some(p) = self.pending_details.pop()
                // A summary with nothing visible in it folds nothing: drop
                // the region rather than offer a fold whose header is blank.
                && let (Some(s), Some(e)) = (p.summary_start, p.summary_end)
                && s < e
                && !self.text[s as usize..e as usize].trim().is_empty()
            {
                self.details.push(DetailsRegion {
                    summary: s..e,
                    end: at,
                });
            }
        } else if tag.starts_with("<details") {
            self.pending_details.push(PendingDetails::default());
        } else if tag.starts_with("<summary") {
            if let Some(p) = self.pending_details.last_mut() {
                p.summary_start = Some(at);
            }
        } else if tag.starts_with("</summary")
            && let Some(p) = self.pending_details.last_mut()
        {
            p.summary_end = Some(at);
        }
    }

    fn open_is_item(&self) -> bool {
        self.open.as_ref().is_some_and(|o| o.kind == NodeKind::Item)
    }

    fn open_block(&mut self, kind: NodeKind, src: &Range<usize>) {
        // Leaves never nest; if one is somehow open, close it first.
        if self.open.is_some() {
            self.close_block(src.end);
        }
        self.open = Some(Open {
            kind,
            doc_start: self.text.len() as u32,
            src_start: src.start as u32,
            indent: self.indent,
            prefix: None,
            quote_depth: self.quote_depth,
            alert: self.alerts.last().copied(),
            quote_cols: self.quote_cols.clone(),
            inlines: Vec::new(),
            image: None,
            mixed: false,
        });
        // The first block inside a footnote definition takes the label.
        if let Some(label) = self.footnote.take()
            && let Some(o) = self.open.as_mut()
        {
            let width = display_width(&label);
            o.prefix = Some(Prefix {
                text: label.into_boxed_str(),
                width,
                marker: Marker::Bullet,
                task: None,
            });
        }
    }

    /// The marker for the item about to open, advancing the ordered counter.
    fn next_marker(&mut self) -> (String, Marker) {
        match self.lists.last_mut() {
            Some(Some(n)) => {
                let cur = *n;
                *n = n.saturating_add(1);
                (format!("{cur}. "), Marker::Ordered(cur))
            }
            _ => ("- ".to_string(), Marker::Bullet),
        }
    }

    fn close_block(&mut self, src_end: usize) {
        let Some(o) = self.open.take() else { return };
        let doc_end = self.text.len() as u32;

        // An empty `Item` is still a block: its marker must paint. This bites
        // in two shapes — an item holding only a nested list, and an item
        // whose FIRST child is a code block/table/heading (opening the child
        // closes the still-empty item). An early return here used to swallow
        // both, silently dropping the bullet or the ordered number and leaving
        // a numbering gap. An empty block wraps to one empty row, and the
        // prefix paints on it.

        // A paragraph that held exactly one image and nothing else is a
        // figure. Items are deliberately excluded: an image in a list renders
        // as its alt text, inline.
        let kind = match (&o.kind, &o.image, o.mixed) {
            (NodeKind::Paragraph, Some(_), false) => NodeKind::Image {
                url: o.image.clone().unwrap_or_default(),
            },
            _ => o.kind.clone(),
        };

        let id = NodeId(self.nodes.len() as u32);
        let block = BlockIdx(self.layout_order.len() as u32);
        self.layout_order.push(id);

        self.nodes.push(Node {
            id,
            // Container linkage waits for its real consumer, the GUI's HTML
            // emitter (<ul>/<blockquote> nesting). Section ancestry never
            // needed it: headings are flat siblings, and `section_path` /
            // `section_end` derive the section tree from levels alone.
            parent: None,
            children: 0..0,
            kind,
            alert: o.alert,
            quote_cols: o.quote_cols.into_boxed_slice(),
            doc: o.doc_start..doc_end,
            src: o.src_start..src_end as u32,
            inlines: coalesce(o.inlines).into_boxed_slice(),
            tokens: OnceLock::new(),
            indent: o.indent,
            prefix: o.prefix,
            quote_depth: o.quote_depth,
            block: Some(block),
        });

        // Blocks are separated by exactly one '\n' in `text`, and the separator
        // is NOT part of any block's doc range. It exists so a search for a
        // phrase cannot silently run across a block boundary.
        let at = self.source.len().min(src_end);
        self.push("\n", at..at, ProvKind::Synthetic);
    }

    // `match_same_arms`: several arms share an empty or one-call body but are
    // kept separate deliberately, because each documents a distinct decision
    // about what does and does not enter space 2. Merging them would collapse
    // the reasoning into an unreadable alternation.
    #[allow(clippy::too_many_lines, clippy::match_same_arms)]
    fn run(mut self) -> Document {
        let parser = Parser::new_ext(self.source, Self::opts());
        // Adjacent `Text` events over contiguous source are ONE text run, and
        // have to be seen as one: pulldown-cmark splits at a delimiter run it
        // then declines to use, so `x^2^` arrives as `Text("x^2") Text("^")`.
        // Any hand-rolled inline pass that looks at one event at a time is
        // blind to a closer that landed in the next one — which is exactly
        // how `x^2^` stayed literal while `H~2~O` worked.
        //
        // Coalescing is safe because there is nothing between two adjacent
        // events by definition, and the merged run keeps its `Verbatim`
        // standing only when the byte lengths still correspond, which `push`
        // already checks.
        let mut events = parser.into_offset_iter().peekable();
        while let Some((ev, src)) = events.next() {
            let (ev, src) = match ev {
                Event::Text(first) => {
                    let mut joined: Option<String> = None;
                    let mut end = src.end;
                    while let Some((Event::Text(next), nsrc)) = events.peek() {
                        if nsrc.start != end {
                            break;
                        }
                        joined
                            .get_or_insert_with(|| first.to_string())
                            .push_str(next);
                        end = nsrc.end;
                        events.next();
                    }
                    match joined {
                        Some(j) => (Event::Text(j.into()), src.start..end),
                        None => (Event::Text(first), src),
                    }
                }
                other => (other, src),
            };
            match ev {
                // --- containers: contribute indent and structure only ---
                // The bar is "│ ", two cells, on every row of every enclosed
                // block — which is why depth is tracked as well as indent.
                Event::Start(Tag::BlockQuote(k)) => {
                    self.quote_cols.push(self.indent);
                    self.indent += 2;
                    self.quote_depth = self.quote_depth.saturating_add(1);
                    // A quote opening as an item's FIRST content: the item's
                    // node (still empty, opened before this quote) reuses for
                    // the text to come, so its geometry snapshot must follow
                    // — otherwise the text renders one bar short (Q14).
                    if self.open_item_is_empty()
                        && let Some(o) = self.open.as_mut()
                    {
                        o.indent = self.indent;
                        o.quote_depth = self.quote_depth;
                        o.quote_cols.clone_from(&self.quote_cols);
                    }
                    // A GFM alert: the parser consumed the `[!NOTE]` line, so
                    // restore the visible word as a one-line block of its own
                    // — synthetic like table padding, and just as searchable.
                    if let Some(kind) = k.map(alert_kind) {
                        self.alerts.push(kind);
                        self.open_block(NodeKind::AlertLabel { kind }, &src);
                        let at = src.start..src.start;
                        self.push(kind.label(), at, ProvKind::Synthetic);
                        self.close_block(src.start);
                    }
                }
                Event::End(TagEnd::BlockQuote(k)) => {
                    self.indent = self.indent.saturating_sub(2);
                    self.quote_depth = self.quote_depth.saturating_sub(1);
                    self.quote_cols.pop();
                    if k.is_some() {
                        self.alerts.pop();
                    }
                }
                Event::Start(Tag::Item) => {
                    // Reserve the marker's REAL width. A flat +2 makes `avail`
                    // wrong for every ordered item past 9.
                    let (text, marker) = self.next_marker();
                    let width = display_width(&text);
                    self.indent = self.indent.saturating_add(width);
                    self.item_indent.push(width);
                    self.open_block(NodeKind::Item, &src);
                    if let Some(o) = self.open.as_mut() {
                        o.prefix = Some(Prefix {
                            text: text.into_boxed_str(),
                            width,
                            marker,
                            task: None,
                        });
                    }
                }
                Event::End(TagEnd::Item) => {
                    self.close_block(src.end);
                    let w = self.item_indent.pop().unwrap_or(0);
                    self.indent = self.indent.saturating_sub(w);
                }
                Event::Start(Tag::List(start)) => self.lists.push(start),
                Event::End(TagEnd::List(_)) => {
                    self.lists.pop();
                }
                Event::Start(Tag::FootnoteDefinition(name)) => {
                    // The definition's first block wears `[^name]: ` as a
                    // prefix — decoration, exactly like a list marker — and
                    // its continuation rows hang under the text.
                    let label = format!("[^{name}]: ");
                    let w = display_width(&label);
                    self.indent = self.indent.saturating_add(w);
                    self.item_indent.push(w);
                    self.footnote = Some(label);
                }
                Event::End(TagEnd::FootnoteDefinition) => {
                    let w = self.item_indent.pop().unwrap_or(0);
                    self.indent = self.indent.saturating_sub(w);
                    self.footnote = None;
                }

                // --- leaf blocks ---
                // A paragraph that is the FIRST content of a list item reuses the
                // item's node, so tight and loose lists produce the same shape.
                // A second paragraph in the same item opens its own block.
                Event::Start(Tag::Paragraph) => {
                    if !self.open_item_is_empty() {
                        self.open_block(NodeKind::Paragraph, &src);
                    }
                }
                Event::End(TagEnd::Paragraph) => {
                    if !self.open_is_item() {
                        self.close_block(src.end);
                    }
                }

                // An HTML block renders as an ordinary paragraph of its
                // stripped text. The stripper's state resets per block: a tag
                // never spans blocks.
                Event::Start(Tag::HtmlBlock) => {
                    self.html = HtmlStrip::default();
                    if !self.open_item_is_empty() {
                        self.open_block(NodeKind::Paragraph, &src);
                    }
                }
                Event::End(TagEnd::HtmlBlock) => {
                    if !self.open_is_item() {
                        self.close_block(src.end);
                    }
                }

                // A definition list is a container in pulldown-cmark, but the
                // term and the details are the layout-atomic leaves, so only
                // the leaves open blocks.
                Event::Start(Tag::DefinitionList) | Event::End(TagEnd::DefinitionList) => {}
                Event::Start(Tag::DefinitionListTitle) => {
                    self.open_block(NodeKind::DefTerm, &src);
                }
                Event::End(TagEnd::DefinitionListTitle) => self.close_block(src.end),
                Event::Start(Tag::DefinitionListDefinition) => {
                    self.indent = self.indent.saturating_add(2);
                    self.open_block(NodeKind::DefDetails, &src);
                }
                Event::End(TagEnd::DefinitionListDefinition) => {
                    self.close_block(src.end);
                    self.indent = self.indent.saturating_sub(2);
                }

                // Frontmatter. The body is pushed verbatim like any other text;
                // `key_col` is computed at close from the lines actually
                // captured, because the width is a property of the whole block.
                Event::Start(Tag::MetadataBlock(_)) => {
                    // Reserve the card's gutter in the MODEL, not at paint:
                    // `indent` is width-independent, so layout wraps long
                    // values inside the card instead of under its rule.
                    self.indent = self.indent.saturating_add(2);
                    self.open_block(NodeKind::Metadata { key_col: 0 }, &src);
                }
                Event::End(TagEnd::MetadataBlock(_)) => {
                    // Compute before taking the mutable borrow of `open`:
                    // `self.text` and `self.open` cannot both be borrowed.
                    if let Some(start) = self.open.as_ref().map(|o| o.doc_start) {
                        let key_col = Self::meta_key_col(&self.text[start as usize..]);
                        if let Some(o) = self.open.as_mut() {
                            o.kind = NodeKind::Metadata { key_col };
                        }
                    }
                    self.close_block(src.end);
                    self.indent = self.indent.saturating_sub(2);
                }

                Event::Start(Tag::Heading { level, .. }) => {
                    self.open_block(NodeKind::Heading { level: level as u8 }, &src);
                }
                Event::End(TagEnd::Heading(_)) => self.close_block(src.end),

                Event::Start(Tag::CodeBlock(kind)) => {
                    let lang = match kind {
                        CodeBlockKind::Fenced(l) if !l.is_empty() => {
                            Some(l.into_string().into_boxed_str())
                        }
                        _ => None,
                    };
                    self.open_block(NodeKind::CodeBlock { lang }, &src);
                }
                Event::End(TagEnd::CodeBlock) => self.close_block(src.end),

                Event::Start(Tag::Table(_)) => {
                    self.open_block(
                        NodeKind::Table {
                            cols: Box::default(),
                            cell_starts: Box::default(),
                        },
                        &src,
                    );
                    self.table = Some(TableRec::default());
                }
                Event::End(TagEnd::Table) => {
                    self.flush_table(src.end);
                    self.close_block(src.end);
                }
                // Cell and row separators are display structure, not content.
                // A table stays ONE block on purpose: `cols` carries the
                // max-content width per column and `cell_starts` every cell's
                // first byte, both width-independent, which is what lets the
                // frontend re-chunk into card view at the recorded boundaries
                // without re-wrapping. (Q15, answered — card view shipped
                // 2026-08-11; this was a TODO pointing at it as open work.)
                Event::End(TagEnd::TableCell) => {
                    if let Some(t) = self.table.as_mut() {
                        let cell = std::mem::take(&mut t.cell);
                        t.row.push(cell);
                        t.cell_col = 0;
                    }
                }
                Event::End(TagEnd::TableRow | TagEnd::TableHead) => {
                    if let Some(t) = self.table.as_mut() {
                        let row = std::mem::take(&mut t.row);
                        t.rows.push(row);
                    }
                }
                Event::Start(Tag::TableCell | Tag::TableRow | Tag::TableHead) => {}

                Event::Rule => {
                    self.open_block(NodeKind::Rule, &src);
                    self.close_block(src.end);
                }

                // --- inline style ---
                Event::Start(Tag::Superscript) => {
                    self.style = self.style.insert(Style::SUPERSCRIPT);
                }
                Event::End(TagEnd::Superscript) => {
                    self.style = self.style.remove(Style::SUPERSCRIPT);
                }
                Event::Start(Tag::Subscript) => self.style = self.style.insert(Style::SUBSCRIPT),
                Event::End(TagEnd::Subscript) => self.style = self.style.remove(Style::SUBSCRIPT),
                Event::Start(Tag::Emphasis) => self.style = self.style.insert(Style::EMPHASIS),
                Event::End(TagEnd::Emphasis) => self.style = self.style.remove(Style::EMPHASIS),
                Event::Start(Tag::Strong) => self.style = self.style.insert(Style::STRONG),
                Event::End(TagEnd::Strong) => self.style = self.style.remove(Style::STRONG),
                Event::Start(Tag::Strikethrough) => {
                    self.style = self.style.insert(Style::STRIKETHROUGH);
                }
                Event::End(TagEnd::Strikethrough) => {
                    self.style = self.style.remove(Style::STRIKETHROUGH);
                }
                // A link's URL is not visible, so it is not in `text` — only
                // the link's *text* is, via the Text events inside it. The
                // destination goes into the links table so the reader can
                // follow it and the terminal can OSC 8 it.
                Event::Start(Tag::Link {
                    link_type,
                    dest_url,
                    ..
                }) => {
                    self.current_link = Some(LinkId(self.links.len() as u32));
                    self.links.push(dest_url.into_string().into_boxed_str());
                    self.wiki
                        .push(matches!(link_type, LinkType::WikiLink { .. }));
                    self.style = self.style.insert(Style::LINK);
                }
                Event::End(TagEnd::Link) => {
                    self.current_link = None;
                    self.style = self.style.remove(Style::LINK);
                }
                // Image alt text IS visible as a placeholder, so it stays —
                // and it is the only content an image contributes to space 2.
                // The destination is recorded so a paragraph holding nothing
                // but this image can become an Image node at close.
                Event::Start(Tag::Image { dest_url, .. }) => {
                    if let Some(o) = self.open.as_mut() {
                        let empty_so_far = o.doc_start as usize == self.text.len();
                        if o.image.is_some() || !empty_so_far {
                            // A second image, or text before this one.
                            o.mixed = true;
                        } else {
                            o.image = Some(dest_url.into_string().into_boxed_str());
                        }
                    }
                    self.in_image = true;
                }
                Event::End(TagEnd::Image) => self.in_image = false,

                // --- text ---
                // Math: the SOURCE is the doc text. Delimiters are consumed
                // by the parser, so this is Substituted, not Verbatim.
                Event::DisplayMath(t) => {
                    // `$$…$$` arrives inside a Paragraph. Opening a block here
                    // would close that paragraph while it is still empty and
                    // leave a stray blank line above every equation, so reuse
                    // the empty paragraph instead — the same move a loose list
                    // item makes with its own node.
                    let reuse = self.open.as_ref().is_some_and(|o| {
                        matches!(o.kind, NodeKind::Paragraph)
                            && o.doc_start as usize == self.text.len()
                    });
                    if reuse {
                        if let Some(o) = self.open.as_mut() {
                            o.kind = NodeKind::Math;
                        }
                    } else {
                        self.open_block(NodeKind::Math, &src);
                    }
                    let prev = self.style;
                    self.style = self.style.insert(Style::MATH);
                    self.push(&t, src.clone(), ProvKind::Substituted);
                    self.style = prev;
                    self.close_block(src.end);
                }
                // Inline math enters space 2 ALREADY RENDERED (`x^2` becomes
                // `x²`), because the display text is authoritative: painting
                // glyphs that differ from it would desynchronise wrapping,
                // selection and search at once. The same standing entity
                // decoding and smart punctuation already have — which is
                // exactly why `Prov` exists. Unparseable math keeps its source.
                Event::InlineMath(t) => {
                    let rendered = crate::math::parse(&t)
                        .map_or_else(|| t.to_string(), |e| crate::math::inline_text(&e));
                    let prev = self.style;
                    self.style = self.style.insert(Style::MATH);
                    self.push(&rendered, src, ProvKind::Substituted);
                    self.style = prev;
                }
                Event::Text(t) => self.push_linkified(&t, src),
                Event::Code(t) => {
                    // Backticks are delimiters, so the src range is longer than
                    // the text: Substituted, not Verbatim.
                    let prev = self.style;
                    self.style = self.style.insert(Style::CODE);
                    self.push(&t, src, ProvKind::Substituted);
                    self.style = prev;
                }
                // A SoftBreak is a source newline inside a paragraph. It renders
                // as a single space. This is exactly why `flexible_ws` search is
                // the default: a user searching a phrase they can see must match
                // across an author's hard-wrapped source line.
                Event::SoftBreak => self.push(" ", src, ProvKind::Substituted),
                Event::HardBreak => self.push("\n", src, ProvKind::Substituted),

                Event::FootnoteReference(name) => {
                    let s = format!("[^{name}]");
                    self.push(&s, src, ProvKind::Substituted);
                }

                // Raw HTML: tags stripped, text kept (research Q16). A block's
                // body stays readable and searchable — a `<details>` block
                // must never swallow its contents. Structure-bearing tags
                // (`<details>`, `<summary>`) are drained from the stripper
                // and recorded as fold regions before the text is pushed.
                Event::Html(t) | Event::InlineHtml(t) => {
                    let mut out = String::new();
                    self.html.strip(&t, &mut out);
                    let marks = std::mem::take(&mut self.html.completed);
                    // Blank-line hygiene: an HTML block's tag-only lines leave
                    // bare newlines behind; skip them at a block's start and
                    // after another newline so no empty rows are minted.
                    let mut leading = 0usize;
                    while out.starts_with('\n')
                        && self
                            .open
                            .as_ref()
                            .is_none_or(|o| o.doc_start as usize == self.text.len())
                    {
                        out.remove(0);
                        leading += 1;
                    }
                    while out.ends_with("\n\n") {
                        out.pop();
                    }
                    let kept = out.len();
                    if kept > 0 {
                        self.push(&out, src, ProvKind::Substituted);
                    }
                    for m in marks {
                        // Map the mark from pre-hygiene output offsets to a doc
                        // byte: leading removals shift everything down, and a
                        // chunk that ended up empty has no positions to offer.
                        let off = m.at_out.saturating_sub(leading).min(kept);
                        let at = if kept > 0 {
                            (self.text.len() - kept + off) as u32
                        } else {
                            self.text.len() as u32
                        };
                        self.details_mark(&m.tag, at);
                    }
                }
                // Task list markers are decoration, so they extend the item's
                // prefix rather than entering `text`. They arrive *inside* the
                // item, after `Start(Item)`, so the prefix is amended in place.
                Event::TaskListMarker(done) => {
                    let extra = if done { "[x] " } else { "[ ] " };
                    let w = display_width(extra);
                    self.indent = self.indent.saturating_add(w);
                    if let Some(last) = self.item_indent.last_mut() {
                        *last = last.saturating_add(w);
                    }
                    if let Some(o) = self.open.as_mut() {
                        o.indent = o.indent.saturating_add(w);
                        if let Some(p) = o.prefix.as_mut() {
                            let mut t = String::from(&*p.text);
                            t.push_str(extra);
                            p.text = t.into_boxed_str();
                            p.width = p.width.saturating_add(w);
                            p.task = Some(done);
                        }
                    }
                } // No catch-all on purpose. Every `Event` variant is now
                  // handled, so a pulldown-cmark bump that adds one fails to
                  // COMPILE rather than silently dropping the new construct —
                  // the same drift-guard instinct as the help tables.
            }
        }
        // pulldown always closes what it opens, but a buffered table that
        // somehow reached EOF unflushed would otherwise vanish silently.
        self.flush_table(self.source.len());
        self.close_block(self.source.len());

        Document {
            source: Arc::from(self.source),
            text: self.text,
            nodes: self.nodes.into_boxed_slice(),
            layout_order: self.layout_order.into_boxed_slice(),
            links: self.links.into_boxed_slice(),
            wiki: self.wiki.into_boxed_slice(),
            prov: self.prov.into_boxed_slice(),
            details: self.details.into_boxed_slice(),
        }
    }
}

/// Expand tabs to the next multiple of [`TAB_STOP`], advancing `col`.
///
/// Tabs must not survive into [`Document::text`]. `ratatui::text::Span` silently
/// drops control characters, so a tab reaching paint simply vanishes. Expanding
/// here also keeps `WidthFn` context-free: a tab's width is a function of the
/// column it lands in, which a per-cluster measurement cannot express.
///
/// `col` is the display column since the last newline, so the second tab on a
/// line expands correctly rather than to a fixed width.
fn expand_tabs<'s>(s: &'s str, col: &mut u16) -> Cow<'s, str> {
    if !s.contains('\t') {
        advance_col(s, col);
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len() + 8);
    for g in s.graphemes(true) {
        match g {
            "\t" => {
                let n = TAB_STOP - (*col % TAB_STOP);
                out.extend(std::iter::repeat_n(' ', n as usize));
                *col = col.saturating_add(n);
            }
            "\n" => {
                out.push('\n');
                *col = 0;
            }
            _ => {
                out.push_str(g);
                *col = col.saturating_add(cluster_width(g));
            }
        }
    }
    Cow::Owned(out)
}

/// Advance `col` past `s`, which is known to contain no tab.
fn advance_col(s: &str, col: &mut u16) {
    let tail = match s.rfind('\n') {
        Some(i) => {
            *col = 0;
            &s[i + 1..]
        }
        None => s,
    };
    if tail.is_ascii() {
        *col = col.saturating_add(tail.len() as u16);
    } else {
        for g in tail.graphemes(true) {
            *col = col.saturating_add(cluster_width(g));
        }
    }
}

/// Merge adjacent runs that share a style **and a link target** — two
/// adjacent links styled identically must not fuse into one destination.
fn coalesce(mut v: Vec<Inline>) -> Vec<Inline> {
    if v.len() < 2 {
        return v;
    }
    let mut out: Vec<Inline> = Vec::with_capacity(v.len());
    for run in v.drain(..) {
        match out.last_mut() {
            Some(last)
                if last.style == run.style
                    && last.link == run.link
                    && last.doc.end == run.doc.start =>
            {
                last.doc.end = run.doc.end;
            }
            _ => out.push(run),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A document with nesting, a skipped level, and a sibling return:
    /// H1 "Top" → H2 "Mid" → H4 "Deep" (skips H3) → H2 "Next".
    const SECTIONED: &str = "\
# Top\n\nintro\n\n## Mid\n\nmid body\n\n#### Deep\n\ndeep body\n\n## Next\n\ntail\n";

    fn heading_named(d: &Document, name: &str) -> NodeId {
        d.nodes
            .iter()
            .find(|n| {
                matches!(n.kind, NodeKind::Heading { .. })
                    && &d.text[n.doc.start as usize..n.doc.end as usize] == name
            })
            .map(|n| n.id)
            .expect("heading exists")
    }

    fn path_names(d: &Document, at: u32) -> Vec<String> {
        d.section_path(at)
            .into_iter()
            .map(|id| {
                let n = &d.nodes[id.0 as usize];
                d.text[n.doc.start as usize..n.doc.end as usize].to_string()
            })
            .collect()
    }

    #[test]
    fn the_section_path_stacks_enclosing_headings_outermost_first() {
        let d = Document::parse(SECTIONED);
        let deep = heading_named(&d, "Deep");
        let inside_deep = d.nodes[deep.0 as usize].doc.end + 2;
        assert_eq!(path_names(&d, inside_deep), ["Top", "Mid", "Deep"]);
    }

    #[test]
    fn a_sibling_heading_pops_back_to_its_own_level() {
        let d = Document::parse(SECTIONED);
        let next = heading_named(&d, "Next");
        let in_next = d.nodes[next.0 as usize].doc.end + 2;
        assert_eq!(path_names(&d, in_next), ["Top", "Next"]);
    }

    #[test]
    fn before_the_first_heading_the_path_is_empty() {
        let d = Document::parse("plain text first\n\n# Then a heading\n");
        assert_eq!(d.section_path(0), vec![]);
    }

    #[test]
    fn a_byte_at_a_headings_start_is_inside_its_section() {
        let d = Document::parse(SECTIONED);
        let mid = heading_named(&d, "Mid");
        let at = d.nodes[mid.0 as usize].doc.start;
        assert_eq!(path_names(&d, at), ["Top", "Mid"]);
    }

    #[test]
    fn path_levels_strictly_increase_at_every_byte() {
        let d = Document::parse(SECTIONED);
        for at in 0..=u32::try_from(d.text.len()).unwrap() {
            let levels: Vec<u8> = d
                .section_path(at)
                .into_iter()
                .map(|id| match d.nodes[id.0 as usize].kind {
                    NodeKind::Heading { level } => level,
                    _ => unreachable!("paths hold only headings"),
                })
                .collect();
            assert!(
                levels.windows(2).all(|w| w[0] < w[1]),
                "at {at}: {levels:?}"
            );
        }
    }

    #[test]
    fn a_sections_end_is_the_next_same_or_higher_heading() {
        let d = Document::parse(SECTIONED);
        let mid = heading_named(&d, "Mid");
        let next = heading_named(&d, "Next");
        assert_eq!(
            d.section_end(mid),
            d.nodes[next.0 as usize].doc.start,
            "Mid's section ends where Next begins — Deep did not close it"
        );
    }

    #[test]
    fn the_last_section_runs_to_the_end_of_the_text() {
        let d = Document::parse(SECTIONED);
        let top = heading_named(&d, "Top");
        let next = heading_named(&d, "Next");
        let len = u32::try_from(d.text.len()).unwrap();
        assert_eq!(d.section_end(top), len, "nothing outranks an H1 here");
        assert_eq!(d.section_end(next), len);
    }

    #[test]
    fn delimiters_are_not_in_display_text() {
        // The whole point of space 2: searching "boldtext" must be possible
        // even though the source has asterisks in the middle of it.
        let d = Document::parse("**bold**text");
        assert_eq!(d.text.trim_end(), "boldtext");
    }

    #[test]
    fn link_urls_are_not_searchable_but_link_text_is() {
        let d = Document::parse("see [the docs](https://example.com/secret)");
        assert!(d.text.contains("the docs"));
        assert!(!d.text.contains("example.com"));
    }

    #[test]
    fn softbreak_becomes_a_space() {
        let d = Document::parse("one two\nthree four");
        assert_eq!(d.text.trim_end(), "one two three four");
    }

    #[test]
    fn entity_decoding_makes_prov_substituted() {
        // `&amp;` is 5 source bytes and 1 display byte. If we assumed
        // doc = src - block_start, every offset after this point would be wrong.
        let d = Document::parse("a &amp; b");
        assert_eq!(d.text.trim_end(), "a & b");
        assert!(d.to_src(DocByte(0)).0 < d.source.len() as u32);
    }

    #[test]
    fn blocks_are_separated_and_do_not_overlap() {
        let d = Document::parse("# Title\n\nFirst para.\n\nSecond para.\n");
        assert_eq!(d.block_count(), 3);
        for w in d.layout_order.windows(2) {
            let a = &d.nodes[w[0].get()];
            let b = &d.nodes[w[1].get()];
            assert!(a.doc.end <= b.doc.start, "blocks overlap: {a:?} {b:?}");
        }
        assert_eq!(d.block_text(BlockIdx(0)), "Title");
    }

    #[test]
    fn to_src_round_trips_on_verbatim_text() {
        let src = "hello world";
        let d = Document::parse(src);
        let at = d.text.find("world").unwrap();
        let s = d.to_src(DocByte(at as u32));
        assert_eq!(&src[s.get()..s.get() + 5], "world");
    }

    #[test]
    fn indent_tracks_nesting() {
        let d = Document::parse("- outer\n  - inner\n");
        let indents: Vec<u16> = d
            .layout_order
            .iter()
            .map(|n| d.nodes[n.get()].indent)
            .collect();
        assert!(indents.windows(2).any(|w| w[1] > w[0]), "{indents:?}");
    }

    #[test]
    fn tight_list_items_get_their_own_blocks() {
        // pulldown-cmark emits no Paragraph inside a tight list item, so if
        // `Item` were not allowed to be a leaf the text would belong to no block.
        let d = Document::parse("- alpha\n- beta\n- gamma\n");
        assert_eq!(d.block_count(), 3);
        assert_eq!(d.block_text(BlockIdx(1)), "beta");
    }

    #[test]
    fn loose_list_second_paragraph_is_its_own_block() {
        let d = Document::parse("- one\n\n  two\n");
        let texts: Vec<&str> = (0..d.block_count())
            .map(|i| d.block_text(BlockIdx(i as u32)))
            .collect();
        assert_eq!(texts, vec!["one", "two"]);
    }

    #[test]
    fn ordered_list_indent_reserves_the_real_marker_width() {
        // "9. " is 3 cells and "10. " is 4. A flat +2 per nesting level gets
        // both wrong, which makes `avail = width - indent` wrong and wraps
        // ordered lists incorrectly.
        let d = Document::parse("9. nine\n10. ten\n");
        let indents: Vec<u16> = (0..d.block_count())
            .map(|i| d.node_for_block(BlockIdx(i as u32)).indent)
            .collect();
        assert_eq!(indents, vec![3, 4]);
    }

    #[test]
    fn list_items_carry_their_marker_as_a_prefix() {
        let d = Document::parse("- alpha\n");
        let p = d.node_for_block(BlockIdx(0)).prefix.as_ref().unwrap();
        assert_eq!(&*p.text, "- ");
        assert_eq!(p.width, 2);
        assert_eq!(p.marker, Marker::Bullet);
        // The marker is decoration, so it must not be searchable.
        assert!(!d.text.contains('-'));
    }

    #[test]
    fn task_list_markers_land_in_the_prefix_rather_than_being_discarded() {
        let d = Document::parse("- [x] done\n");
        let p = d.node_for_block(BlockIdx(0)).prefix.as_ref().unwrap();
        assert_eq!(&*p.text, "- [x] ");
        assert_eq!(p.task, Some(true));
        assert_eq!(p.width, 6);
        assert_eq!(d.node_for_block(BlockIdx(0)).indent, 6);
    }

    #[test]
    fn quote_depth_counts_nesting_because_the_bar_repeats_on_every_row() {
        let d = Document::parse("> outer\n\n> > inner\n");
        let depths: Vec<u8> = (0..d.block_count())
            .map(|i| d.quote_depth(BlockIdx(i as u32)))
            .collect();
        assert_eq!(depths, vec![1, 2]);
    }

    #[test]
    fn an_image_alone_in_a_paragraph_becomes_an_image_node() {
        let d = Document::parse("![a chart](chart.png)\n");
        match &d.node_for_block(BlockIdx(0)).kind {
            NodeKind::Image { url } => assert_eq!(&**url, "chart.png"),
            k => panic!("expected Image, got {k:?}"),
        }
        // Alt text is the node's searchable doc text — and the URL is not:
        // one hit for "chart" (the alt), none for the filename's stem beyond it.
        assert_eq!(d.block_text(BlockIdx(0)), "a chart");
        assert_eq!(crate::search::search(&d, "chart", true).len(), 1);
        assert_eq!(crate::search::search(&d, "chart.png", true).len(), 0);
    }

    #[test]
    fn an_inline_image_stays_a_paragraph_with_alt_text() {
        let d = Document::parse("before ![alt](u.png) after\n");
        assert!(matches!(
            d.node_for_block(BlockIdx(0)).kind,
            NodeKind::Paragraph
        ));
        assert_eq!(d.block_text(BlockIdx(0)), "before alt after");
    }

    #[test]
    fn two_images_in_one_paragraph_stay_a_paragraph() {
        let d = Document::parse("![a](one.png) ![b](two.png)\n");
        assert!(matches!(
            d.node_for_block(BlockIdx(0)).kind,
            NodeKind::Paragraph
        ));
    }

    #[test]
    fn an_image_in_a_list_item_stays_inline() {
        let d = Document::parse("- ![alt](u.png)\n");
        assert!(matches!(d.node_for_block(BlockIdx(0)).kind, NodeKind::Item));
    }

    #[test]
    fn an_item_whose_first_child_is_a_code_block_keeps_its_marker() {
        // Start(CodeBlock) closes the still-empty Item; the empty-item early
        // return used to swallow it, and the "1." marker vanished.
        let d = Document::parse("1. ```rust\n   x\n   ```\n2. text\n");
        let prefixes: Vec<String> = (0..d.block_count())
            .filter_map(|i| {
                d.node_for_block(BlockIdx(i as u32))
                    .prefix
                    .as_ref()
                    .map(|p| p.text.to_string())
            })
            .collect();
        assert!(
            prefixes.contains(&"1. ".to_string()),
            "the first item's marker must survive: {prefixes:?}",
        );
        assert!(prefixes.contains(&"2. ".to_string()), "{prefixes:?}");
    }

    #[test]
    fn an_item_holding_only_a_nested_list_keeps_its_bullet_too() {
        // The old early return existed for this shape; the bullet should not
        // vanish here either — it renders on its own row above the children.
        let d = Document::parse("-\n  - inner\n");
        let prefixes: Vec<String> = (0..d.block_count())
            .filter_map(|i| {
                d.node_for_block(BlockIdx(i as u32))
                    .prefix
                    .as_ref()
                    .map(|p| p.text.to_string())
            })
            .collect();
        assert_eq!(
            prefixes.len(),
            2,
            "outer bullet and inner bullet: {prefixes:?}"
        );
    }

    #[test]
    fn a_tab_inside_a_table_cell_still_aligns_columns() {
        // The tab expands to spaces relative to the cell start, so the
        // measured width and the emitted width agree.
        let d = Document::parse("| a\tb | x |\n|---|---|\n| cc | y |\n");
        let text = d.block_text(BlockIdx(0));
        let lines: Vec<&str> = text.lines().collect();
        let col_x = lines[0].find('x').unwrap();
        let col_y = lines[1].find('y').unwrap();
        assert_eq!(col_x, col_y, "second column must line up:\n{text}");
        assert!(!text.contains('\t'), "no raw tab may survive: {text:?}");
    }

    #[test]
    fn table_cells_pad_to_max_content_width_at_parse() {
        // Alignment is space-2 construction, like tab expansion: column
        // widths are max-content, which is width-independent.
        let d = Document::parse("| a | bb |\n|---|---|\n| ccc | d |\n");
        assert_eq!(d.block_text(BlockIdx(0)), "a     bb\nccc   d\n");
        match &d.node_for_block(BlockIdx(0)).kind {
            NodeKind::Table { cols, .. } => assert_eq!(&**cols, &[3u16, 2]),
            k => panic!("expected a table, got {k:?}"),
        }
    }

    #[test]
    fn table_padding_is_synthetic_and_cell_text_round_trips_to_source() {
        let src = "| alpha | beta |\n|---|---|\n| one | two |\n";
        let d = Document::parse(src);
        let text = d.block_text(BlockIdx(0));
        let at = d.node_for_block(BlockIdx(0)).doc.start + text.find("two").unwrap() as u32;
        let s = d.to_src(DocByte(at));
        assert_eq!(
            &src[s.get()..s.get() + 3],
            "two",
            "provenance through padding"
        );
    }

    #[test]
    fn cjk_cells_align_by_display_width_not_bytes() {
        let d = Document::parse("| 日本 | x |\n|---|---|\n| ab | y |\n");
        match &d.node_for_block(BlockIdx(0)).kind {
            // 日本 is 6 bytes but 4 cells; byte-based widths would say 6.
            NodeKind::Table { cols, .. } => assert_eq!(cols[0], 4),
            k => panic!("expected a table, got {k:?}"),
        }
    }

    #[test]
    fn table_cell_starts_are_recorded_per_row_with_a_fixed_stride() {
        let doc = Document::parse("| a | bb |\n|---|---|\n| ccc | d |\n");
        let node = doc.node_for_block(BlockIdx(0));
        let NodeKind::Table { cols, cell_starts } = &node.kind else {
            panic!("not a table: {:?}", node.kind)
        };
        assert_eq!(cols.len(), 2);
        assert_eq!(cell_starts.len(), 4, "2 rows x 2 cols");
        // Every start points at its cell's first byte.
        let cell = |i: usize| {
            let s = cell_starts[i] as usize;
            &doc.text[s..(s + 3).min(doc.text.len())]
        };
        assert!(cell(0).starts_with('a'), "{:?}", cell(0));
        assert!(cell(1).starts_with("bb"), "{:?}", cell(1));
        assert!(cell(2).starts_with("ccc"), "{:?}", cell(2));
        assert!(cell(3).starts_with('d'), "{:?}", cell(3));
        // Strictly ascending when all cells exist.
        assert!(cell_starts.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn a_ragged_row_pads_cell_starts_with_its_line_end() {
        // Row two has one cell; the missing second entry equals that row's end.
        let doc = Document::parse("| a | b |\n|---|---|\n| c |\n");
        let node = doc.node_for_block(BlockIdx(0));
        let NodeKind::Table { cols, cell_starts } = &node.kind else {
            panic!()
        };
        assert_eq!(cols.len(), 2);
        assert_eq!(cell_starts.len(), 4);
        let line_end = doc.text[cell_starts[2] as usize..]
            .find('\n')
            .map_or(node.doc.end, |i| cell_starts[2] + i as u32);
        assert_eq!(
            cell_starts[3], line_end,
            "missing cell sits at the line end"
        );
    }

    #[test]
    fn link_destinations_are_stored_and_runs_carry_their_ids() {
        let d = Document::parse("see [the docs](https://example.com/d) or [notes](notes.md)");
        assert_eq!(&*d.links[0], "https://example.com/d");
        assert_eq!(&*d.links[1], "notes.md");
        let n = d.node_for_block(BlockIdx(0));
        let linked: Vec<_> = n.inlines.iter().filter_map(|i| i.link).collect();
        assert_eq!(linked, vec![LinkId(0), LinkId(1)]);
    }

    #[test]
    fn adjacent_links_with_identical_styling_do_not_coalesce() {
        let d = Document::parse("[a](u1)[b](u2)");
        let n = d.node_for_block(BlockIdx(0));
        let linked: Vec<_> = n.inlines.iter().filter_map(|i| i.link).collect();
        assert_eq!(
            linked.len(),
            2,
            "two targets must stay two runs: {:?}",
            n.inlines
        );
        assert_ne!(linked[0], linked[1]);
    }

    #[test]
    fn a_tagged_code_block_carries_semantic_tokens_and_prose_does_not() {
        let d = Document::parse("plain paragraph\n\n```rust\nfn main() {}\n```\n");
        assert!(
            d.tokens(BlockIdx(0)).is_empty(),
            "prose must carry no tokens"
        );
        let code = d.node_for_block(BlockIdx(1));
        assert!(
            !d.tokens(BlockIdx(1)).is_empty(),
            "tagged rust should classify"
        );
        for t in d.tokens(BlockIdx(1)) {
            assert!(
                t.doc.start >= code.doc.start && t.doc.end <= code.doc.end,
                "token {t:?} escapes block {:?}",
                code.doc,
            );
        }
    }

    #[test]
    fn an_untagged_code_block_stays_plain() {
        let d = Document::parse("```\nsomething\n```\n");
        assert!(d.tokens(BlockIdx(0)).is_empty());
    }

    #[test]
    fn tabs_are_expanded_at_parse_to_the_next_stop() {
        // `a` occupies column 0, so the tab advances to the next stop at 4:
        // three spaces, not one. Tab stop is 4.
        let d = Document::parse("a\tb\n");
        assert_eq!(d.block_text(BlockIdx(0)), "a   b");
    }

    #[test]
    fn every_byte_of_display_text_is_covered_by_a_block_or_a_separator() {
        let src = "# T\n\npara *one*\n\n- a\n- b\n\n> quoted\n\n```rs\nlet x = 1;\n```\n";
        let d = Document::parse(src);
        let mut cursor = 0u32;
        for n in &d.layout_order {
            let node = &d.nodes[n.get()];
            let gap = &d.text[cursor as usize..node.doc.start as usize];
            assert!(
                gap.chars().all(char::is_whitespace),
                "non-separator gap {gap:?} before {node:?}"
            );
            cursor = node.doc.end;
        }
        assert!(d.text[cursor as usize..].chars().all(char::is_whitespace));
    }

    // --- Q16: raw HTML strips its tags but keeps its text ---

    #[test]
    fn an_html_block_keeps_its_text_content_searchable() {
        let doc = Document::parse(
            "before\n\n<details><summary>Click me</summary>\nhidden body text\n</details>\n\nafter\n",
        );
        assert!(doc.text.contains("Click me"), "{:?}", doc.text);
        assert!(doc.text.contains("hidden body text"), "{:?}", doc.text);
        assert!(
            !doc.text.contains('<'),
            "tags must be stripped: {:?}",
            doc.text
        );
    }

    #[test]
    fn inline_html_tags_vanish_but_br_becomes_a_break() {
        let doc = Document::parse("alpha <b>beta</b> gamma<br>delta\n");
        assert!(doc.text.contains("alpha beta gamma"), "{:?}", doc.text);
        assert!(!doc.text.contains('<'), "{:?}", doc.text);
        assert!(
            doc.text.contains("gamma\ndelta"),
            "br is a break: {:?}",
            doc.text
        );
    }

    #[test]
    fn script_style_and_comments_contribute_no_text() {
        let doc = Document::parse(
            "<div>\n<script>var evil = 1;</script>\n<style>p { color: red }</style>\n<!-- note to self -->\nvisible\n</div>\n",
        );
        assert!(doc.text.contains("visible"), "{:?}", doc.text);
        assert!(
            !doc.text.contains("evil"),
            "script bodies dropped: {:?}",
            doc.text
        );
        assert!(
            !doc.text.contains("color"),
            "style bodies dropped: {:?}",
            doc.text
        );
        assert!(
            !doc.text.contains("note to self"),
            "comments dropped: {:?}",
            doc.text
        );
    }

    #[test]
    fn an_html_img_contributes_its_alt_text() {
        let doc = Document::parse("<p><img src=\"x.png\" alt=\"a chart of numbers\"></p>\n");
        assert!(doc.text.contains("a chart of numbers"), "{:?}", doc.text);
    }

    // --- GFM alerts ---

    #[test]
    fn a_note_alert_gets_a_searchable_label_and_flagged_blocks() {
        let doc = Document::parse("> [!NOTE]\n> useful advice here\n");
        assert!(doc.text.contains("Note"), "label in text: {:?}", doc.text);
        assert!(doc.text.contains("useful advice here"));
        let label = doc.node_for_block(BlockIdx(0));
        assert!(
            matches!(
                label.kind,
                NodeKind::AlertLabel {
                    kind: AlertKind::Note
                }
            ),
            "{:?}",
            label.kind
        );
        assert_eq!(label.quote_depth, 1, "the label sits inside the quote");
        let body = doc.node_for_block(BlockIdx(1));
        assert_eq!(body.alert, Some(AlertKind::Note), "body carries the kind");
    }

    #[test]
    fn every_alert_kind_maps_and_an_ordinary_quote_stays_plain() {
        for (src, kind) in [
            ("> [!TIP]\n> t\n", AlertKind::Tip),
            ("> [!IMPORTANT]\n> t\n", AlertKind::Important),
            ("> [!WARNING]\n> t\n", AlertKind::Warning),
            ("> [!CAUTION]\n> t\n", AlertKind::Caution),
        ] {
            let doc = Document::parse(src);
            assert!(
                matches!(doc.node_for_block(BlockIdx(0)).kind, NodeKind::AlertLabel { kind: k } if k == kind),
                "{src}"
            );
        }
        let plain = Document::parse("> just a quote\n");
        assert_eq!(plain.node_for_block(BlockIdx(0)).alert, None);
        assert!(!plain.text.contains("Note"));
    }

    // --- GFM task lists ---

    #[test]
    fn tasks_are_found_with_their_state_and_order() {
        let src = "- [ ] open one\n- [x] done one\n  - [ ] nested open\nplain bullet\n";
        let doc = Document::parse(src);
        let ts = doc.tasks();
        assert_eq!(ts.len(), 3, "{ts:?}");
        assert_eq!(ts.iter().filter(|t| t.done).count(), 1);
        assert!(ts[0].at < ts[1].at && ts[1].at < ts[2].at, "reading order");
        // Each lands on its item's first byte.
        for t in &ts {
            let n = doc.node_for_block(doc.block_at_doc(DocByte(t.at)));
            assert!(n.prefix.as_ref().is_some_and(|p| p.task.is_some()));
        }
    }

    #[test]
    fn a_document_without_tasks_reports_none() {
        assert!(Document::parse("no checkboxes here\n").tasks().is_empty());
    }

    // --- <details> regions ---

    /// The GitHub shape: opener block, blank line, markdown body, blank
    /// line, closer block. The summary text stays ordinary searchable text.
    #[test]
    fn a_details_region_records_its_summary_and_end() {
        let src = "<details>\n<summary>Click me</summary>\n\nHidden body prose.\n\n</details>\n";
        let doc = Document::parse(src);
        assert_eq!(doc.details.len(), 1, "text: {:?}", doc.text);
        let r = &doc.details[0];
        assert_eq!(
            &doc.text[r.summary.start as usize..r.summary.end as usize],
            "Click me"
        );
        // The region ends at the closer, past the body's last byte.
        assert!(
            r.end as usize >= doc.text.find("prose.").unwrap() + "prose.".len(),
            "end {} covers the body",
            r.end
        );
        assert!(doc.text.contains("Hidden body prose."));
    }

    #[test]
    fn nested_details_produce_two_regions_and_inner_closes_first() {
        let src = concat!(
            "<details>\n<summary>outer</summary>\n\n",
            "<details>\n<summary>inner</summary>\n\ndeep body\n\n</details>\n\n",
            "outer tail\n\n</details>\n"
        );
        let doc = Document::parse(src);
        assert_eq!(doc.details.len(), 2);
        let inner = &doc.details[0];
        let outer = &doc.details[1];
        let sum = |r: &DetailsRegion| &doc.text[r.summary.start as usize..r.summary.end as usize];
        assert_eq!(sum(inner), "inner");
        assert_eq!(sum(outer), "outer");
        // The inner one ends before the outer tail; the outer covers it.
        assert!(inner.end <= outer.end);
        assert!(
            (inner.end as usize) <= doc.text.find("outer tail").unwrap(),
            "inner closed by its own tag, not the outer's"
        );
    }

    #[test]
    fn an_empty_summary_is_not_a_fold() {
        let doc = Document::parse("<details>\n<summary></summary>\n\nbody\n\n</details>\n");
        assert!(doc.details.is_empty());
        let doc = Document::parse("<details>\nbody without a summary\n\n</details>\n");
        assert!(doc.details.is_empty());
    }

    #[test]
    fn details_on_one_line_still_record_their_summary() {
        let doc =
            Document::parse("before <details><summary>tiny</summary>after body</details> end\n");
        assert_eq!(doc.details.len(), 1);
        let r = &doc.details[0];
        assert_eq!(
            &doc.text[r.summary.start as usize..r.summary.end as usize],
            "tiny"
        );
    }

    // --- footnote definitions get their label back ---

    #[test]
    fn a_footnote_definition_wears_its_label_as_a_prefix() {
        let doc = Document::parse("body[^a].\n\n[^a]: the definition text\n");
        let def = (0..doc.block_count())
            .map(|i| doc.node_for_block(BlockIdx(i as u32)))
            .find(|n| doc.text[n.doc.start as usize..n.doc.end as usize].contains("definition"))
            .expect("definition block exists");
        let p = def.prefix.as_ref().expect("has a prefix");
        assert_eq!(&*p.text, "[^a]: ");
        assert_eq!(p.width, display_width("[^a]: "));
        assert!(
            def.indent >= p.width,
            "continuation rows hang under the label"
        );
    }

    // --- footnote references and definitions pair up ---

    #[test]
    fn footnote_references_and_definitions_are_findable() {
        let src = "a body[^one] with two[^two].\n\n[^one]: first note\n\n[^two]: second\n";
        let doc = Document::parse(src);
        let refs = doc.footnote_refs();
        assert_eq!(refs.len(), 2, "{refs:?}");
        assert_eq!(refs[0].0.as_ref(), "one");
        assert_eq!(
            &doc.text[refs[0].1 as usize..refs[0].1 as usize + 6],
            "[^one]"
        );
        let defs = doc.footnote_defs();
        assert_eq!(defs.len(), 2, "{defs:?}");
        // Each reference pairs with the definition of the same name.
        for (name, _) in &refs {
            assert!(
                defs.iter().any(|(d, _)| d == name),
                "no definition for {name}"
            );
        }
        // Definitions sit at their block start, where a jump reveals them.
        let (dname, dbyte) = &defs[0];
        assert_eq!(dname.as_ref(), "one");
        assert_eq!(&doc.text[*dbyte as usize..*dbyte as usize + 5], "first");
    }

    #[test]
    fn a_bracket_caret_inside_code_is_not_a_footnote_reference() {
        let doc = Document::parse("text[^a] then\n\n```rust\nlet x = \"[^not one]\";\n```\n");
        let refs = doc.footnote_refs();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].0.as_ref(), "a");
    }

    #[test]
    fn an_unclosed_bracket_caret_spins_nowhere() {
        let doc = Document::parse("oops [^ no closer here\n");
        assert!(doc.footnote_refs().is_empty());
    }

    // --- fragment anchors ---

    #[test]
    fn fragments_resolve_to_headings_github_style() {
        let doc = Document::parse(
            "# My Great Section!\n\nbody\n\n## Ünïcode & Symbols?\n\nmore\n\n# Dup\n\n# Dup\n",
        );
        let at = |frag: &str| doc.fragment_target(frag);
        let start_of = |needle: &str| doc.text.find(needle).unwrap() as u32;
        assert_eq!(at("my-great-section"), Some(start_of("My Great Section!")));
        assert_eq!(at("ünïcode--symbols"), Some(start_of("Ünïcode & Symbols?")));
        assert_eq!(at("dup"), Some(start_of("Dup")));
        // GitHub numbers duplicates.
        assert!(at("dup-1").is_some());
        assert_ne!(at("dup-1"), at("dup"));
        assert_eq!(at("no-such"), None);
    }

    // --- Q14: quote bars nested under list indent ---

    #[test]
    fn quote_bar_columns_record_where_each_enclosing_quote_began() {
        // A quote inside a list item: its bar belongs at the item's indent,
        // not at the margin. And a quote>list>quote sandwich has two bars at
        // two different, non-adjacent columns.
        let doc = Document::parse("- item\n\n  > quoted inside the item\n");
        let quoted = (0..doc.block_count())
            .map(|i| doc.node_for_block(BlockIdx(i as u32)))
            .find(|n| n.quote_depth == 1)
            .expect("quoted block exists");
        assert_eq!(&*quoted.quote_cols, &[2], "bar at the item's indent");

        let deep = Document::parse("> - > double nested text\n");
        let inner = (0..deep.block_count())
            .map(|i| deep.node_for_block(BlockIdx(i as u32)))
            .find(|n| n.quote_depth == 2)
            .expect("depth-2 block exists");
        assert_eq!(
            &*inner.quote_cols,
            &[0, 4],
            "outer at 0, inner past the marker"
        );
    }

    #[test]
    fn a_wikilink_renders_its_target_as_visible_link_text() {
        let d = Document::parse("see [[Reflow Layer]] for details");
        assert!(d.text.contains("Reflow Layer"), "{:?}", d.text);
        assert!(!d.text.contains("[["), "brackets are syntax, not content");
        assert_eq!(d.links.len(), 1);
        assert_eq!(&*d.links[0], "Reflow Layer");
        assert!(d.is_wikilink(LinkId(0)));
        let n = &d.nodes[d.layout_order[0].get()];
        let link_run = n
            .inlines
            .iter()
            .find(|i| i.link.is_some())
            .expect("a link run");
        assert!(link_run.style.contains(Style::LINK));
    }

    #[test]
    fn a_piped_wikilink_shows_the_label_and_stores_the_target() {
        let d = Document::parse("see [[reflow-layer|the reflow doc]]");
        assert!(d.text.contains("the reflow doc"));
        assert!(
            !d.text.contains("reflow-layer"),
            "target is invisible: {:?}",
            d.text
        );
        assert_eq!(&*d.links[0], "reflow-layer");
        assert!(d.is_wikilink(LinkId(0)));
    }

    #[test]
    fn ordinary_links_are_not_wikilinks() {
        let d = Document::parse("[docs](https://example.com) and [[Note]]");
        assert_eq!(d.links.len(), 2);
        assert!(!d.is_wikilink(LinkId(0)));
        assert!(d.is_wikilink(LinkId(1)));
        assert!(!d.is_wikilink(LinkId(9)), "out of range is just false");
    }

    #[test]
    fn searching_a_wikilink_target_matches_label_not_target() {
        let d = Document::parse("read [[secret-file|the docs]] now");
        assert_eq!(crate::search(&d, "the docs", true).len(), 1);
        assert_eq!(crate::search(&d, "secret-file", true).len(), 0);
    }

    // --- frontmatter (Q16, 2026-08-15) ---

    #[test]
    fn yaml_frontmatter_becomes_a_metadata_block_not_a_heading() {
        let doc = Document::parse("---\ntitle: My Note\ntags: [a, b]\n---\n\n# Heading\n");
        let first = doc.node_for_block(BlockIdx(0));
        assert!(
            matches!(first.kind, NodeKind::Metadata { .. }),
            "frontmatter must not parse as a heading; got {:?}",
            first.kind
        );
        let body = &doc.text[first.doc.start as usize..first.doc.end as usize];
        assert!(
            body.contains("title"),
            "frontmatter text is retained: {body:?}"
        );
        assert!(
            body.contains("My Note"),
            "values are retained too: {body:?}"
        );
    }

    #[test]
    fn frontmatter_key_col_is_the_widest_key_capped_at_sixteen() {
        let doc = Document::parse("---\na: 1\nlonger_key: 2\n---\n");
        let NodeKind::Metadata { key_col } = doc.node_for_block(BlockIdx(0)).kind else {
            panic!("expected a metadata block");
        };
        assert_eq!(key_col, 10, "`longer_key` is 10 cells wide");
    }

    #[test]
    fn toml_frontmatter_is_a_metadata_block_too() {
        let doc = Document::parse("+++\ntitle = \"My Note\"\n+++\n\ntext\n");
        assert!(matches!(
            doc.node_for_block(BlockIdx(0)).kind,
            NodeKind::Metadata { .. }
        ));
    }

    // --- definition lists (Q16, 2026-08-15) ---

    #[test]
    fn a_definition_list_is_a_term_block_and_an_indented_details_block() {
        let doc = Document::parse("Term\n: the definition\n");
        let term = doc.node_for_block(BlockIdx(0));
        let details = doc.node_for_block(BlockIdx(1));
        assert!(
            matches!(term.kind, NodeKind::DefTerm),
            "got {:?}",
            term.kind
        );
        assert!(
            matches!(details.kind, NodeKind::DefDetails),
            "got {:?}",
            details.kind
        );
        assert_eq!(
            &doc.text[term.doc.start as usize..term.doc.end as usize],
            "Term"
        );
        assert!(details.indent >= 2, "the definition hangs under its term");
        assert!(
            !doc.text.contains(" : "),
            "the `:` marker is consumed, not rendered as prose: {:?}",
            doc.text
        );
    }

    // --- super/subscript (Q16, 2026-08-15) ---

    /// **Upstream limitation, verified 2026-08-15 against pulldown-cmark
    /// 0.13.4:** the delimiters are only recognised at a word boundary.
    /// `^2^` and `x ^2^ y` parse; `x^2^` and `H~2~O` — the forms people
    /// actually write — do not, and stay literal text. Recorded in the
    /// conformance table rather than worked around here.
    #[test]
    fn superscript_and_subscript_are_style_runs_not_literal_markers() {
        let doc = Document::parse("E = mc ^2^ and log ~2~ n\n");
        assert!(
            !doc.text.contains('^') && !doc.text.contains('~'),
            "markers are consumed: {:?}",
            doc.text
        );
        let node = doc.node_for_block(BlockIdx(0));
        let sup = node
            .inlines
            .iter()
            .find(|i| i.style.contains(Style::SUPERSCRIPT))
            .expect("a superscript run exists");
        assert_eq!(&doc.text[sup.doc.start as usize..sup.doc.end as usize], "2");
        assert!(
            node.inlines
                .iter()
                .any(|i| i.style.contains(Style::SUBSCRIPT)),
            "a subscript run exists"
        );
    }

    /// `x^2^` / `H~2~O`: upstream declines the attached forms, so
    /// [`attached_scripts`] handles them. The result must be
    /// INDISTINGUISHABLE from the parsed form — same consumed markers, same
    /// style run — or paint, search and selection would each need a special
    /// case.
    #[test]
    fn attached_scripts_become_the_same_style_runs_as_parsed_ones() {
        for (src, want, style) in [
            ("x^2^\n", "x2", Style::SUPERSCRIPT),
            ("H~2~O\n", "H2O", Style::SUBSCRIPT),
            ("E = mc^2^ exactly\n", "E = mc2 exactly", Style::SUPERSCRIPT),
            ("log~2~n\n", "log2n", Style::SUBSCRIPT),
        ] {
            let doc = Document::parse(src);
            assert_eq!(doc.text.trim_end(), want, "for {src:?}");
            let node = doc.node_for_block(BlockIdx(0));
            let run = node
                .inlines
                .iter()
                .find(|i| i.style.contains(style))
                .unwrap_or_else(|| panic!("no {style:?} run for {src:?}"));
            assert_eq!(&doc.text[run.doc.start as usize..run.doc.end as usize], "2");
        }
    }

    /// **`x^2^` arrives as TWO events** — `Text("x^2")` then `Text("^")` —
    /// because pulldown-cmark splits at a delimiter run it then declines to
    /// use. A per-event scan cannot see the closer, which is why `H~2~O`
    /// worked and `x^2^` did not until `run` coalesced adjacent text.
    #[test]
    fn adjacent_text_events_over_contiguous_source_are_one_run() {
        let mut opts = Options::empty();
        opts.insert(Options::ENABLE_SUPERSCRIPT);
        let evs: Vec<_> = Parser::new_ext("x^2^\n", opts).collect();
        let texts = evs.iter().filter(|e| matches!(e, Event::Text(_))).count();
        assert_eq!(texts, 2, "upstream still splits: {evs:?}");
        // …and carrel still sees one run through it.
        assert_eq!(Document::parse("x^2^\n").text.trim_end(), "x2");
    }

    #[test]
    fn the_attached_script_scan_declines_what_is_not_a_script() {
        for src in [
            "x^^\n",                           // empty
            "x^a b^\n",                        // whitespace inside
            "a~verylongsubscripttexthere~b\n", // past MAX_SCRIPT
            "see ~/Work and ~/Work\n",         // opener not attached
            "https://e.com/a^b^\n",            // a bare url is plain text here
            "`x^2^`\n",                        // a code span
            "```\nx^2^\n```\n",                // a code block
        ] {
            let doc = Document::parse(src);
            assert!(
                doc.text.contains('^') || doc.text.contains('~'),
                "markers were eaten in {src:?}: {:?}",
                doc.text
            );
        }
    }

    /// A `www.` autolink inside a fenced block became a live OSC 8 link for
    /// six days, because the hand-rolled pass never asked whether it was in
    /// code. Found 2026-08-21 while adding a second such pass.
    #[test]
    fn a_code_block_is_never_linkified() {
        let doc = Document::parse("```\nsee www.example.com\n```\n");
        assert!(doc.links.is_empty(), "linkified code: {:?}", doc.links);
        assert!(doc.text.contains("www.example.com"));
        // Prose next to it still linkifies.
        let doc = Document::parse("see www.example.com\n");
        assert_eq!(doc.links.len(), 1);
    }

    // --- GFM extended autolinks (Q16, 2026-08-15) ---

    #[test]
    fn bare_www_becomes_a_link_and_trailing_punctuation_stays_out_of_it() {
        let doc = Document::parse("Visit www.example.com, then stop.\n");
        let node = doc.node_for_block(BlockIdx(0));
        let link = node
            .inlines
            .iter()
            .find(|i| i.link.is_some())
            .expect("a link run exists");
        assert_eq!(
            &doc.text[link.doc.start as usize..link.doc.end as usize],
            "www.example.com",
            "the trailing comma is not part of the link"
        );
        let LinkId(i) = link.link.expect("link id");
        assert_eq!(&*doc.links[i as usize], "http://www.example.com");
    }

    #[test]
    fn autolink_scanner_trims_only_unbalanced_trailing_parens() {
        let hits = extended_autolinks("see (www.example.com/a_(b)) end");
        assert_eq!(hits.len(), 1, "one hit: {hits:?}");
        assert_eq!(
            &"see (www.example.com/a_(b)) end"[hits[0].0.clone()],
            "www.example.com/a_(b)"
        );
    }

    #[test]
    fn a_bare_www_with_no_host_is_not_a_link() {
        assert!(extended_autolinks("www. alone").is_empty());
    }

    #[test]
    fn www_inside_an_existing_link_is_not_relinkified() {
        let doc = Document::parse("[label](https://x.test) and www.example.com\n");
        let n = doc.node_for_block(BlockIdx(0));
        let count = n.inlines.iter().filter(|i| i.link.is_some()).count();
        assert_eq!(count, 2, "one explicit, one autolinked -- not three");
    }
}
