//! Mermaid → Unicode box art, off-thread — wave F, Q24.
//!
//! Same shape as the image pipeline: requests collected from the document,
//! rendered on a background thread, results drained per frame, and art
//! arrival is just another reflow through the anchor machinery. Rendering is
//! best-effort decoration: a block that is not drawn stays syntax-highlighted
//! source. An unsupported family or a parse error does so silently; a block
//! refused by a limit below says so ([`Refused`]), because a reader who saw
//! a chart drawn yesterday cannot otherwise tell a limit from a typo.
//!
//! # A diagram is bounded by what it costs, before it is drawn
//!
//! The renderer's cost is not a function of the source's size. A sequence
//! diagram allocates a grid of participants × messages: 3,000 one-line
//! messages between 3,000 participants are 49 KB of source and more than
//! 4 GB of grid, and the allocation failure aborts the process with the
//! terminal still in the alternate screen. A left-to-right flowchart is
//! laid out in time that grows with its edges times the height of its
//! tallest rank: one node with 1,500 children took more than a minute. A
//! thread cannot be stopped from outside, so every limit is decided before
//! the step it protects — and each is a limit on COST, set from
//! measurements, with room above the largest diagram a person draws. None
//! of them is a limit on how wide or tall a drawing may be: a chart wider
//! than the window is cut at its edge like a long code line, as it always
//! was.
//!
//! | limit | value | what it bounds |
//! |---|---|---|
//! | [`MAX_SOURCE_BYTES`] | 64 KiB, comments aside | the parse |
//! | [`MAX_NESTING`] | 64 | the parse of nested `{ }`, then nested blocks |
//! | [`MAX_FAN_OUT`] | 2,000 | what `a & b --> c & d` multiplies out to |
//! | [`MAX_ITEMS`] | 2,000 | entities + relationships, every family |
//! | [`MAX_FLOW_NODES`] | 400 | a flowchart's widest possible rank |
//! | [`MAX_FLOW_WORK`] | 160,000 | a flowchart's nodes × edges |
//! | [`MAX_RELATION_WORK`] | 30,000,000 | a class or ER diagram's nodes² × edges |
//! | [`MAX_FLOW_HUBS`] | 40,000 | the sum of its nodes' squared degrees |
//! | [`MAX_SEQUENCE_CELLS`] | 2,000,000 | a sequence diagram's grid, estimated |
//! | [`MAX_ART_CELLS`] | 1,000,000 | the art that comes back, and a grid |
//!
//! and per document: [`MAX_DOC_SOURCE_BYTES`] of source requested,
//! [`MAX_DOC_ART_CELLS`] of art kept, [`RENDER_BUDGET`] of rendering.
//!
//! ## What was measured
//!
//! Release build, `merman` 0.8.0-alpha.5, one diagram per process, time of
//! the render and peak RSS of the process (8 MB with nothing drawn).
//!
//! Ordinary diagrams — 53 of them, the largest a 300-node, 320-edge
//! flowchart of 620 items and a 300-message sequence diagram: every one
//! under 10 ms and 20 MB. All are drawn, with the largest at 75% of the
//! node limit, 60% of the work limit and 5% of the hub limit; the widest
//! art is 1,255 columns.
//!
//! The worst shapes found that pass every limit — nothing that passes
//! costs more than about a second:
//!
//! | shape | time | peak RSS |
//! |---|---|---|
//! | class or ER diagram, 200 nodes, 750 relations at random | 1.0–1.1 s | 12 MB |
//! | class or ER diagram, 125 nodes, 1,850 relations between two halves | 0.9–1.0 s | 15 MB |
//! | left-to-right flowchart, 19 hubs of 20 children in one rank (380 nodes) | 0.8 s | 56 MB |
//! | left-to-right flowchart, one node with 199 children | 0.25 s | 25 MB |
//! | left-to-right flowchart, 200 nodes wired crosswise to 200 more | 0.2 s | 21 MB |
//! | top-down flowchart, 380 nodes in one rank | 0.04 s | 48 MB |
//! | flowchart at the art limit, 1,005 rows × 995 columns | 0.01 s | 48 MB |
//! | sequence diagram, 120 participants created one by one | 0.02 s | 50 MB |
//! | state diagram at 2,000 items, any shape | 0.1 s | 41 MB |
//! | gantt, mind map, timeline, journey, kanban, git graph at 2,000 items | under 0.05 s | under 20 MB |
//! | any family, the parse of 64 KiB | under 0.2 s | under 25 MB |
//!
//! and what the limits refuse: one node with 499 children (2.7 s, 108 MB,
//! by the hub limit; 1,500 children, over a minute); 300 classes or
//! entities with 1,600 relations at random (4 to 6.5 s, by the relation
//! work limit); 14 KB of two
//! 1,000-name `&` lists (a million edges, 1.5 s and 1.8 GB of parse, by the
//! fan-out limit); class namespaces nested 1,700 deep (1.3 s of parse, by
//! the nesting limit); the 3,000-participant sequence diagram (by the grid
//! estimate).
//!
//! **The numbers belong to that version: re-measure before a `merman`
//! bump**, as its pin already demands.
//!
//! NO RATATUI — `scripts/check-discipline.sh` rule 6.

use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use carrel_core::{BlockIdx, Document, NodeKind};
use merman_core::diagram::RenderSemanticModel;
use merman_core::resources::{InputResourceLimitId, InputResourcePolicy, ModelComplexity};

/// One mermaid block worth rendering: `(block, source)`.
pub type Request = (BlockIdx, String);

/// Why a mermaid block was not drawn, when the reason is a limit. A block
/// that is simply not drawable — a family the renderer does not know, a
/// syntax error — has no entry: it was never going to be a drawing.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    /// The diagram itself is past a limit of this module.
    TooLarge,
    /// The diagram is fine; the document's diagrams together reached
    /// [`MAX_DOC_SOURCE_BYTES`], [`MAX_DOC_ART_CELLS`] or [`RENDER_BUDGET`].
    Budget,
}

impl Refused {
    /// What the reader is told, under the block.
    #[must_use]
    pub const fn caption(self) -> &'static str {
        match self {
            Self::TooLarge => "too large to draw — shown as text",
            Self::Budget => "not drawn: this document has too many diagrams — shown as text",
        }
    }
}

/// What the worker says about one block: its art, or the limit that kept it
/// source.
pub type Reply = (BlockIdx, Result<Vec<String>, Refused>);

/// The longest source handed to the parser, by [`effective_len`] — comment
/// lines aside, since the parser drops them at no cost. The largest ordinary
/// diagram measured is 10 KB; 500 labelled nodes and their edges are 25 KB.
/// Parsing is not linear in this everywhere (2,000 sibling subgraphs or
/// 3,500 sequence messages take tens of milliseconds at half this size),
/// which is what keeps it from being larger.
pub const MAX_SOURCE_BYTES: usize = 64 * 1024;

/// What the source's `&` lists may multiply out to, by [`fan_out`]'s bound.
///
/// `a & b --> c & d` is four edges, and the parser builds every one before
/// anything can count them: 14 KB of two 1,000-name lists is a million
/// edges, 1.5 s and 1.8 GB of parse. So this one is read off the text. It
/// is [`MAX_ITEMS`]: more edges than that would be refused after the parse
/// anyway.
pub const MAX_FAN_OUT: usize = MAX_ITEMS;

/// Entities plus relationships, as `merman` counts them for every family:
/// nodes and edges, participants and messages, tasks, commits (a state
/// transition counts four, a chart's data point five). The families with
/// no limit of their own below — state, gantt, mind map, timeline, journey,
/// kanban, packet, git graph — are drawn in 0.1 s and 41 MB at this many,
/// whatever their shape. It does NOT bound the graph-shaped ones, whose
/// cost is in how their items are wired: see [`MAX_FLOW_WORK`] and
/// [`MAX_RELATION_WORK`]. The largest ordinary diagram measured has 620.
pub const MAX_ITEMS: usize = 2_000;

/// Text the parsed model may hold. The model repeats a label several times
/// over (a mind map, twenty times), so this sits well above
/// [`MAX_SOURCE_BYTES`]; it is a ceiling on what the parse may retain.
const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;

/// Nested blocks, subgraphs and composite states — and, read off the text
/// before the parse ([`brace_depth`]), nested `{ }`: class namespaces 1,700
/// deep are 32 KB and 1.3 s of parse. Each level costs columns on both
/// sides, so nothing legible is this deep.
pub const MAX_NESTING: usize = 64;

/// A flowchart's nodes. Its layout puts every node of a rank side by side,
/// and what that costs is the widest rank — which is at most all of them:
/// top-down, 130 KB of working memory a node (380 in one rank, 48 MB);
/// left-to-right, time (see [`MAX_FLOW_WORK`]). The largest ordinary
/// flowchart measured has 300; at 500 the worst shape found took 1.3 to
/// 1.8 s, which is why it is not 500.
pub const MAX_FLOW_NODES: usize = 400;

/// A flowchart's nodes × edges. Left to right, routing an edge costs time
/// in proportion to the height of the drawing, which is ten rows a node of
/// its tallest rank: measured, up to 0.9 µs an edge a row, so 9 µs for
/// each node × edge when every node shares a rank. At this limit that is
/// 1.4 s by the formula and 0.8 s in the worst shape found. The largest
/// ordinary flowchart measured is 96,000.
pub const MAX_FLOW_WORK: usize = 160_000;

/// The sum, over a flowchart's nodes, of the square of each one's edges.
/// One node with hundreds of children is the cheapest way to build a tall
/// rank — 499 children are 2.7 s and 108 MB inside both limits above — and
/// nothing a person draws: the busiest ordinary flowchart measured sums to
/// 2,400, an 8 × 8 `&` product to 1,024. At this limit one hub has 199
/// children (0.2 s).
pub const MAX_FLOW_HUBS: usize = 40_000;

/// A class or ER diagram's nodes² × edges ([`relation_work`]). These two
/// are laid out by a layout of their own, whose time on a tangled graph
/// grows with the square of the classes and with the relations: measured,
/// 17 to 50 ns for each node² × edge when the relations fall at random,
/// and next to nothing for a tree or a hub. At this limit that is at most
/// 1.1 s (200 classes, 750 relations); 300 classes with 1,600 relations,
/// which every other limit passes, took 4 to 6.5 s. An ordinary large
/// diagram — 100 classes, 150 relations — is a twentieth of it. A state
/// diagram needs no such bound: at [`MAX_ITEMS`] it draws in 0.2 s whatever
/// its shape.
pub const MAX_RELATION_WORK: usize = 30_000_000;

/// What a class or ER diagram is measured by against
/// [`MAX_RELATION_WORK`].
#[must_use]
pub const fn relation_work(nodes: usize, edges: usize) -> usize {
    nodes.saturating_mul(nodes).saturating_mul(edges)
}

/// The grid a sequence diagram may allocate, by [`sequence_cells`]'s
/// estimate. Its renderer budgets nothing, so this is the limit that stands
/// between a small file and an out-of-memory abort. At about 45 bytes a
/// cell, a diagram at the limit is at most some 90 MB while it is drawn;
/// the estimate runs high, so an ordinary one is a quarter of that or
/// less. The largest ordinary sequence diagram measured estimates 430,000.
pub const MAX_SEQUENCE_CELLS: usize = 2_000_000;

/// The art one diagram may come back as, in cells — a few megabytes of
/// strings — and the grid the flowchart, class, ER, state and chart
/// renderers may allocate, which they check themselves: 41 bytes a cell
/// while it is drawn, so 45 MB here. Not a limit on width or height: the
/// widest ordinary art measured is 1,255 columns and 144,000 cells.
pub const MAX_ART_CELLS: usize = 1_000_000;

/// How much mermaid source one document may ask to have drawn, in document
/// order, comments included: this one bounds what is copied to the worker.
pub const MAX_DOC_SOURCE_BYTES: usize = 4 * 1024 * 1024;

/// How many cells of art one document may keep — tens of megabytes of
/// strings at most. Ten thousand diagrams, each inside its own limits, are
/// otherwise gigabytes.
pub const MAX_DOC_ART_CELLS: usize = 8_000_000;

/// How long one document's diagrams may take to draw, all told. The limits
/// above keep any one diagram to about a second at the very worst and an
/// ordinary one to milliseconds; this is for the document made of
/// thousands of them, whose reader would otherwise have a core busy for as
/// long as the document is open. Checked between diagrams.
pub const RENDER_BUDGET: Duration = Duration::from_secs(5);

/// What a document's mermaid blocks ask of the worker.
#[derive(Debug, Default)]
pub struct Plan {
    /// Every block to draw, in layout order.
    pub requests: Vec<Request>,
    /// Blocks refused from the text alone, never copied.
    pub refused: Vec<(BlockIdx, Refused)>,
}

impl Plan {
    /// Nothing to draw and nothing to say.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.requests.is_empty() && self.refused.is_empty()
    }
}

/// Every ` ```mermaid ` block, in layout order: to be drawn, or refused
/// here because its text alone is past a limit ([`admits`]) or the document
/// has asked for [`MAX_DOC_SOURCE_BYTES`] already.
#[must_use]
pub fn plan(doc: &Document) -> Plan {
    let mut plan = Plan::default();
    let mut spent = 0usize;
    for b in (0..doc.block_count()).map(|i| BlockIdx(u32::try_from(i).unwrap_or(u32::MAX))) {
        let node = doc.node_for_block(b);
        if !matches!(&node.kind, NodeKind::CodeBlock { lang: Some(l) } if &**l == "mermaid") {
            continue;
        }
        let src = &doc.text[node.doc.start as usize..node.doc.end as usize];
        // Asked here as well as in `Renderer::render`, so an oversized
        // block is never even copied.
        if let Err(why) = admits(src) {
            plan.refused.push((b, why));
        } else if spent.saturating_add(src.len()) > MAX_DOC_SOURCE_BYTES {
            plan.refused.push((b, Refused::Budget));
        } else {
            spent += src.len();
            plan.requests.push((b, src.to_string()));
        }
    }
    plan
}

/// `merman`, with every limit of this module set on it.
pub struct Renderer(merman::ascii::HeadlessAsciiRenderer);

impl std::fmt::Debug for Renderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Renderer").finish_non_exhaustive()
    }
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer {
    #[must_use]
    pub fn new() -> Self {
        let mut policy = InputResourcePolicy::default();
        for (id, value) in [
            // The source is measured by `admits`, comments aside; this one
            // counts them, so it only has to be out of the way.
            (InputResourceLimitId::MaxSourceBytes, MAX_DOC_SOURCE_BYTES),
            (InputResourceLimitId::MaxModelItems, MAX_ITEMS),
            (InputResourceLimitId::MaxModelTextBytes, MAX_TEXT_BYTES),
            (InputResourceLimitId::MaxModelNestingDepth, MAX_NESTING),
        ] {
            // Refused only for a zero, which none of these is; a test reads
            // the limits back off the policy.
            let _ = policy.apply_limit(id, value);
        }
        let ascii = merman::ascii::AsciiRenderOptions::default().with_max_grid_cells(MAX_ART_CELLS);
        Self(
            merman::ascii::HeadlessAsciiRenderer::new()
                .with_resource_policy(policy)
                .with_ascii_options(ascii),
        )
    }

    /// The art for one diagram. `Err(None)` when it is simply not drawable
    /// — a family the renderer does not draw, a parse error — and
    /// `Err(Some(_))` when a limit of this module kept it source, each
    /// decided before the step it protects.
    ///
    /// # Errors
    ///
    /// As above: the block stays source either way.
    pub fn render(&self, src: &str) -> Result<Vec<String>, Option<Refused>> {
        use merman::ascii::{AsciiError, HeadlessAsciiError};

        admits(src)?;
        let parsed = self
            .0
            .engine
            .parse_diagram_for_render_model_sync(src, self.0.parse)
            .ok()
            .flatten()
            .ok_or(None)?;
        let model = parsed.model();
        // The renderer checks its policy again on the way in; asked here
        // first so the estimates below never walk an oversized model.
        self.0
            .resource_policy()
            .check_render_model(model)
            .map_err(|_| Refused::TooLarge)?;
        match model {
            RenderSemanticModel::Sequence(seq) => {
                let nesting = ModelComplexity::from_render_model(model).nesting_depth;
                if sequence_cells(seq, nesting) > MAX_SEQUENCE_CELLS {
                    return Err(Some(Refused::TooLarge));
                }
            }
            RenderSemanticModel::Flowchart(flow) => {
                let cost = FlowCost::of(flow);
                if !cost.fits() {
                    return Err(Some(Refused::TooLarge));
                }
            }
            RenderSemanticModel::Class(class) => {
                if relation_work(class.classes.len(), class.relations.len()) > MAX_RELATION_WORK {
                    return Err(Some(Refused::TooLarge));
                }
            }
            RenderSemanticModel::Er(er) => {
                if relation_work(er.entities.len(), er.relationships.len()) > MAX_RELATION_WORK {
                    return Err(Some(Refused::TooLarge));
                }
            }
            _ => {}
        }
        let art = self.0.render_model(model).map_err(|e| match e {
            HeadlessAsciiError::Resource(_)
            | HeadlessAsciiError::Ascii(AsciiError::RenderLimitExceeded { .. }) => {
                Some(Refused::TooLarge)
            }
            _ => None,
        })?;
        let mut lines = Vec::new();
        let mut cells = 0usize;
        for line in art.lines() {
            cells = cells.saturating_add(line.chars().count());
            if cells > MAX_ART_CELLS {
                return Err(Some(Refused::TooLarge));
            }
            lines.push(line.to_string());
        }
        if lines.is_empty() {
            return Err(None);
        }
        Ok(lines)
    }
}

/// What a flowchart's layout can cost, from the parsed model alone: see
/// [`MAX_FLOW_NODES`], [`MAX_FLOW_WORK`] and [`MAX_FLOW_HUBS`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct FlowCost {
    pub nodes: usize,
    /// Nodes × edges.
    pub work: usize,
    /// The sum of each node's squared degree.
    pub hubs: usize,
}

impl FlowCost {
    #[must_use]
    pub fn of(flow: &merman_core::diagrams::flowchart::FlowchartModel) -> Self {
        let mut degree = std::collections::HashMap::<&str, usize>::new();
        for edge in &flow.edges {
            for end in [edge.from.as_str(), edge.to.as_str()] {
                *degree.entry(end).or_default() += 1;
            }
        }
        Self {
            nodes: flow.nodes.len(),
            work: flow.nodes.len().saturating_mul(flow.edges.len()),
            hubs: degree
                .values()
                .map(|d| d.saturating_mul(*d))
                .fold(0, usize::saturating_add),
        }
    }

    #[must_use]
    pub const fn fits(self) -> bool {
        self.nodes <= MAX_FLOW_NODES && self.work <= MAX_FLOW_WORK && self.hubs <= MAX_FLOW_HUBS
    }
}

/// May this source be parsed at all? The limits that are read off the text,
/// because the parse is what they bound.
///
/// # Errors
///
/// [`Refused::TooLarge`] when it may not.
pub fn admits(src: &str) -> Result<(), Refused> {
    if src.len() <= MAX_DOC_SOURCE_BYTES
        && effective_len(src) <= MAX_SOURCE_BYTES
        && fan_out(src) <= MAX_FAN_OUT
        && brace_depth(src) <= MAX_NESTING
    {
        Ok(())
    } else {
        Err(Refused::TooLarge)
    }
}

/// Is this line a `%%` comment? The parser drops these whole; a `%%{…}%%`
/// directive is not one.
fn is_comment(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with("%%") && !line.starts_with("%%{")
}

/// The bytes of `src` the parser has to read: every line that is not a
/// `%%` comment. A two-node graph under 19 KB of commentary costs what a
/// two-node graph costs.
#[must_use]
pub fn effective_len(src: &str) -> usize {
    src.split_inclusive('\n')
        .filter(|line| !is_comment(line))
        .map(str::len)
        .sum()
}

/// The line that names the diagram's family: the first that is not blank,
/// a comment, a directive, or inside the `---` front matter a source may
/// open with.
fn header(src: &str) -> &str {
    let mut lines = src.lines().map(str::trim).peekable();
    if lines.peek() == Some(&"---") {
        lines.next();
        lines.find(|line| *line == "---");
    }
    lines
        .find(|line| !line.is_empty() && !line.starts_with("%%"))
        .unwrap_or_default()
}

/// How deep the source's `{ }` nest, comments aside. Counted off the text,
/// so a brace in a label counts too — which can only refuse a diagram
/// holding dozens of unclosed ones.
///
/// An ER diagram is the exception and counts as flat: its crow's feet are
/// braces that never close (`A ||--o{ B`), one to a relationship, and
/// nothing in its grammar nests.
#[must_use]
pub fn brace_depth(src: &str) -> usize {
    if header(src).starts_with("erDiagram") {
        return 0;
    }
    let (mut depth, mut deepest) = (0usize, 0usize);
    for line in src.lines().filter(|line| !is_comment(line)) {
        for b in line.bytes() {
            match b {
                b'{' => {
                    depth += 1;
                    deepest = deepest.max(depth);
                }
                b'}' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
    }
    deepest
}

/// An upper bound on the edges the source's `&` lists multiply out to.
///
/// A list of `s` names joined by `&` meets the list on the other side of
/// its arrow in `s × t` edges, and at most two arrows touch it, so all the
/// lists together make no more than the sum of their squares. A list
/// cannot continue past a line break (its arrow can), so a line holding
/// `k` ampersands holds lists whose squares sum to at most `(k + 1)²`.
///
/// It over-counts — an `&` in a label is not a list — which costs a diagram
/// with hundreds of them its drawing and nothing else.
#[must_use]
pub fn fan_out(src: &str) -> usize {
    src.lines()
        .filter(|line| !is_comment(line))
        .map(|line| line.bytes().filter(|&b| b == b'&').count())
        .filter(|&k| k > 0)
        .map(|k| (k + 1).saturating_mul(k + 1))
        .fold(0usize, usize::saturating_add)
}

/// An upper estimate of the grid a sequence diagram's renderer allocates,
/// from the parsed model alone.
///
/// Columns: each participant's lifeline stands as far from the next as the
/// longer of its own label and the longest message or note label needs,
/// plus spacing; each level of `loop`/`alt`/`par` nesting adds a frame on
/// both sides. Rows: six per message or note — an arrow takes three, a
/// message to self five, one that creates its participant six — plus the
/// headers. An estimate, kept above what `merman` 0.8.0-alpha.5 really
/// allocates by a test over many shapes, so a diagram is refused a little
/// early, never drawn a little late: an ordinary diagram's real grid is a
/// quarter of its estimate or less.
#[must_use]
pub fn sequence_cells(
    seq: &merman_core::diagrams::sequence::SequenceDiagramRenderModel,
    nesting: usize,
) -> usize {
    let label = seq
        .messages
        .iter()
        .map(|m| m.message_text().len())
        .chain(seq.notes.iter().map(|n| n.message.len()))
        .max()
        .unwrap_or(0);
    let cols = seq
        .actors
        .values()
        .map(|a| a.description.len().max(label).saturating_add(14))
        .fold(24usize, usize::saturating_add)
        .saturating_add(nesting.saturating_mul(12))
        .saturating_add(seq.title.as_deref().map_or(0, str::len));
    let rows = seq
        .messages
        .len()
        .saturating_add(seq.notes.len())
        .saturating_mul(6)
        .saturating_add(16);
    cols.saturating_mul(rows)
}

/// Render the plan off-thread: art streams back as it is drawn, and so does
/// each refusal, so the reader can be told which blocks a limit kept as
/// source. A block that is simply not drawable sends nothing.
///
/// The thread stops at the first of: the last request, or the receiver gone
/// (the document was closed or replaced — checked between diagrams, and one
/// diagram is bounded). Once [`MAX_DOC_ART_CELLS`] of art has been sent or
/// [`RENDER_BUDGET`] spent, every block still waiting is answered
/// [`Refused::Budget`] without being drawn.
#[must_use]
pub fn spawn(plan: Plan) -> Receiver<Reply> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for (block, why) in plan.refused {
            if tx.send((block, Err(why))).is_err() {
                return;
            }
        }
        let renderer = Renderer::new();
        let started = Instant::now();
        let mut cells = 0usize;
        let mut spent = false;
        for (block, src) in plan.requests {
            spent = spent || started.elapsed() > RENDER_BUDGET;
            let reply = if spent {
                Err(Refused::Budget)
            } else {
                match renderer.render(&src) {
                    Ok(lines) => {
                        cells = cells.saturating_add(art_cells(&lines));
                        if cells > MAX_DOC_ART_CELLS {
                            spent = true;
                            Err(Refused::Budget)
                        } else {
                            Ok(lines)
                        }
                    }
                    Err(Some(why)) => Err(why),
                    Err(None) => continue, // not a drawing: stays source, unremarked
                }
            };
            if tx.send((block, reply)).is_err() {
                return;
            }
        }
    });
    rx
}

/// What a piece of art weighs against [`MAX_DOC_ART_CELLS`].
fn art_cells(lines: &[String]) -> usize {
    lines.iter().map(|l| l.chars().count()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `line(0)`, `line(1)`, … one after another: a generated diagram's body.
    fn each(n: usize, mut line: impl FnMut(usize) -> String) -> String {
        let mut s = String::new();
        for i in 0..n {
            s += &line(i);
        }
        s
    }

    /// What the bounded renderer makes of `src`: drawn, or why not.
    fn verdict(src: &str) -> Result<(), Option<Refused>> {
        Renderer::new().render(src).map(|_| ())
    }

    fn drawn(src: &str) -> bool {
        verdict(src).is_ok()
    }

    fn too_large(src: &str) -> bool {
        verdict(src) == Err(Some(Refused::TooLarge))
    }

    /// The parsed model of `src`, with no limit of ours in the way.
    fn parsed(src: &str) -> merman_core::diagram::ParsedDiagramRender {
        let r = merman::ascii::HeadlessAsciiRenderer::new();
        r.engine
            .parse_diagram_for_render_model_sync(src, r.parse)
            .expect("parses")
            .expect("a known family")
    }

    /// What `merman` counts in `src`: `(items, nesting)`.
    fn counted(src: &str) -> (usize, usize) {
        let c = ModelComplexity::from_render_model(parsed(src).model());
        (c.items, c.nesting_depth)
    }

    /// A class or ER diagram's nodes² × edges, as the limit counts it.
    fn relation_work(src: &str) -> Option<usize> {
        match parsed(src).model() {
            RenderSemanticModel::Class(c) => {
                Some(super::relation_work(c.classes.len(), c.relations.len()))
            }
            RenderSemanticModel::Er(e) => Some(super::relation_work(
                e.entities.len(),
                e.relationships.len(),
            )),
            _ => None,
        }
    }

    fn flow_cost(src: &str) -> FlowCost {
        let parsed = parsed(src);
        let RenderSemanticModel::Flowchart(flow) = parsed.model() else {
            panic!("not a flowchart: {src:?}");
        };
        FlowCost::of(flow)
    }

    /// [`sequence_cells`] for `src`, and the grid `merman` really made of
    /// it with no limit of ours in the way: `(estimate, rows × cols)`.
    fn sequence_grids(src: &str) -> (usize, usize) {
        let r = merman::ascii::HeadlessAsciiRenderer::new().with_resource_profile(
            merman_core::resources::ResourceProfile::UnboundedForTrustedInput,
        );
        let parsed = parsed(src);
        let RenderSemanticModel::Sequence(seq) = parsed.model() else {
            panic!("not a sequence diagram: {src:?}");
        };
        let nesting = ModelComplexity::from_render_model(parsed.model()).nesting_depth;
        let art = r.render_model(parsed.model()).expect("renders");
        let cols = art.lines().map(|l| l.chars().count()).max().unwrap_or(0);
        (sequence_cells(seq, nesting), art.lines().count() * cols)
    }

    fn requests_of(md: &str) -> Vec<Request> {
        plan(&Document::parse(md)).requests
    }

    fn only(requests: Vec<Request>) -> Plan {
        Plan {
            requests,
            refused: Vec::new(),
        }
    }

    #[test]
    fn only_mermaid_blocks_become_requests() {
        let reqs =
            requests_of("para\n\n```mermaid\ngraph TD\n a-->b\n```\n\n```rust\nfn x() {}\n```\n");
        assert_eq!(reqs.len(), 1);
        assert!(reqs[0].1.contains("graph TD"), "{:?}", reqs[0].1);
    }

    #[test]
    fn a_flowchart_renders_and_an_unsupported_family_stays_silent() {
        let flow = vec![(BlockIdx(0), "graph TD\n a[alpha]-->b[beta]\n".to_string())];
        let (block, reply) = spawn(only(flow)).recv().expect("a reply for the flowchart");
        let lines = reply.expect("art");
        assert_eq!(block, BlockIdx(0));
        assert!(lines.len() > 3, "box art has real height: {lines:?}");
        assert!(
            lines.iter().any(|l| l.contains("alpha")),
            "labels survive: {lines:?}"
        );

        // Not a drawing, and not a limit either: nothing is said.
        for src in [
            "pie\n \"A\" : 40\n",
            "graph TD\n a --> --> b\n",
            "not mermaid\n",
        ] {
            assert_eq!(verdict(src), Err(None), "{src:?}");
            let rx = spawn(only(vec![(BlockIdx(1), src.to_string())]));
            assert!(rx.recv().is_err(), "{src:?} sends nothing");
        }
    }

    // --- ordinary diagrams are drawn ----------------------------------
    //
    // The limits bound cost, not what a reader may look at. These are the
    // shapes people draw, at the top of the sizes people draw them in, and
    // every one must come back as art. A pseudo-random generator stands in
    // for the hand that drew them: the same graph every run.

    struct Lcg(u64);
    impl Lcg {
        fn below(&mut self, n: usize) -> usize {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            usize::try_from((self.0 >> 33) % (n as u64)).unwrap_or(0)
        }
    }

    /// `nodes` labelled services in a random tree, plus cross edges up to
    /// `edges` in all.
    fn service_map(dir: &str, nodes: usize, edges: usize) -> String {
        let mut rng = Lcg(7);
        let mut s = format!("flowchart {dir}\n");
        s += &each(nodes, |i| format!(" n{i}[Service {i}]\n"));
        s += &each(nodes - 1, |i| {
            format!(" n{} --> n{}\n", rng.below(i + 1), i + 1)
        });
        s += &each(edges + 1 - nodes, |_| {
            let (a, b) = (rng.below(nodes), rng.below(nodes));
            format!(" n{a} --> n{}\n", if a == b { (b + 1) % nodes } else { b })
        });
        s
    }

    fn conversation(participants: usize, messages: usize, label: usize) -> String {
        let mut rng = Lcg(11);
        let mut s = String::from("sequenceDiagram\n");
        s += &each(participants, |i| format!(" participant P{i}\n"));
        s += &each(messages, |_| {
            let a = rng.below(participants);
            let b = (a + 1 + rng.below(participants - 1)) % participants;
            format!(" P{a}->>P{b}: {}\n", "m".repeat(label))
        });
        s
    }

    #[allow(clippy::too_many_lines)] // a list of fixtures
    fn ordinary() -> Vec<(&'static str, String)> {
        let groups = each(10, |g| {
            format!(
                " subgraph G{g}[Group {g}]\n{} end\n",
                each(15, |i| format!("  g{g}_{i}[svc {g}.{i}]\n"))
            )
        });
        let wiring = each(9, |g| {
            each(15, |i| {
                format!(" g{g}_{i} --> g{}_{}\n", g + 1, (i + 1) % 15)
            })
        });
        vec![
            ("80 nodes, 110 edges", service_map("TD", 80, 110)),
            ("150 nodes, 200 edges", service_map("TD", 150, 200)),
            ("150 nodes, 260 edges", service_map("TD", 150, 260)),
            ("190 nodes, 200 edges", service_map("TD", 190, 200)),
            ("210 nodes, 220 edges", service_map("TD", 210, 220)),
            ("300 nodes, 320 edges", service_map("TD", 300, 320)),
            ("300 nodes, left to right", service_map("LR", 300, 320)),
            (
                "150 nodes in 10 subgraphs",
                format!("flowchart LR\n{groups}{wiring}"),
            ),
            (
                "an 8 × 8 & product",
                format!(
                    "flowchart LR\n {} --> {}\n",
                    (0..8)
                        .map(|i| format!("s{i}"))
                        .collect::<Vec<_>>()
                        .join(" & "),
                    (0..8)
                        .map(|i| format!("t{i}"))
                        .collect::<Vec<_>>()
                        .join(" & ")
                ),
            ),
            (
                "a small graph under 19 KB of comments",
                format!(
                    "graph TD\n a[alpha]-->b[beta]\n{}",
                    each(330, |i| format!(
                        "%% comment line number {i} explaining the design at length\n"
                    ))
                ),
            ),
            ("12 participants, 100 messages", conversation(12, 100, 40)),
            ("15 participants, long labels", conversation(15, 60, 60)),
            ("8 participants, 200 messages", conversation(8, 200, 20)),
            ("5 participants, 300 messages", conversation(5, 300, 10)),
            ("20 participants", conversation(20, 40, 10)),
            (
                "40 classes",
                format!(
                    "classDiagram\n{}{}",
                    each(40, |i| format!(
                        " class C{i}{{\n  +field{i}\n  +method{i}()\n }}\n"
                    )),
                    each(39, |i| format!(" C{} <|-- C{}\n", (i + 1) / 3, i + 1))
                ),
            ),
            (
                "30 entities",
                format!(
                    "erDiagram\n{}{}",
                    each(29, |i| format!(
                        " T{} ||--o{{ T{} : has\n",
                        i.div_ceil(2),
                        i + 1
                    )),
                    each(30, |i| format!(" T{i} {{\n  int id\n  string name\n }}\n"))
                ),
            ),
            (
                "120 tasks",
                format!(
                    "gantt\n dateFormat YYYY-MM-DD\n{}",
                    each(120, |i| format!(
                        " Task {i} :t{i}, 2024-01-{:02}, {}d\n",
                        1 + i % 28,
                        1 + i % 9
                    ))
                ),
            ),
            (
                "a mind map of 136 nodes",
                format!(
                    "mindmap\n  root((root))\n{}",
                    each(8, |i| format!(
                        "    Topic {i}\n{}",
                        each(4, |j| format!(
                            "      Sub {i}.{j}\n{}",
                            each(3, |k| format!("        Leaf {i}.{j}.{k}\n"))
                        ))
                    ))
                ),
            ),
            (
                "25 branches",
                format!(
                    "gitGraph\n commit\n{}",
                    each(25, |i| format!(
                        " branch f{i}\n checkout f{i}\n commit\n commit\n checkout main\n merge f{i}\n"
                    ))
                ),
            ),
        ]
    }

    #[test]
    fn ordinary_diagrams_are_drawn_whatever_their_width() {
        for (name, src) in ordinary() {
            assert_eq!(verdict(&src), Ok(()), "{name}");
        }
        // And there is room above them: a quarter again as many nodes.
        assert_eq!(verdict(&service_map("TD", 380, 410)), Ok(()));
        // Width is not a limit. This one is nearly a thousand columns.
        let wide = Renderer::new()
            .render(&service_map("TD", 300, 320))
            .expect("drawn");
        let cols = wide.iter().map(|l| l.chars().count()).max().unwrap_or(0);
        assert!(cols > 900, "{cols} columns");
    }

    // --- the limits, each on both sides -------------------------------
    //
    // Every fixture here is cheap: the diagram past a limit is refused
    // before the step that would cost anything, and where a broken limit
    // would let it through, it is tens of megabytes at most, never the
    // gigabytes the limits exist for.

    #[test]
    fn the_limits_are_set_on_the_renderer_that_draws() {
        let r = Renderer::new();
        let policy = r.0.resource_policy();
        for (id, value) in [
            (InputResourceLimitId::MaxModelItems, MAX_ITEMS),
            (InputResourceLimitId::MaxModelTextBytes, MAX_TEXT_BYTES),
            (InputResourceLimitId::MaxModelNestingDepth, MAX_NESTING),
        ] {
            assert_eq!(policy.value(id), Some(value), "{id:?}");
        }
        assert_eq!(r.0.ascii.max_grid_cells, MAX_ART_CELLS);
    }

    /// The reported file: 3,000 participants, 49 KB, more than 4 GB of
    /// grid. And the same shape far smaller — 400 participants are 6 KB
    /// and were 244 MB.
    #[test]
    fn a_sequence_diagram_of_thousands_of_participants_stays_source() {
        let chain = |n: usize| {
            let messages = each(n, |i| format!("P{i}->>P{}: m\n", i + 1));
            format!("sequenceDiagram\n{messages}")
        };
        assert!(too_large(&chain(3000)));
        assert!(too_large(&chain(400)));
        assert!(too_large(&chain(160)));
        assert!(drawn(&chain(30)), "an ordinary one is still drawn");
    }

    /// The source limit counts what the parser reads: a comment costs
    /// nothing, so it is not counted; anything else is.
    #[test]
    fn source_at_the_byte_limit_is_drawn_one_byte_more_is_not_and_comments_are_free() {
        let head = "graph TD\n a[alpha]-->b[beta]\n";
        let padded = |pad: usize| format!("{head}{}\n", " ".repeat(pad));
        let at = padded(MAX_SOURCE_BYTES - head.len() - 1);
        assert_eq!(effective_len(&at), MAX_SOURCE_BYTES);
        assert!(drawn(&at));
        let over = padded(MAX_SOURCE_BYTES - head.len());
        assert_eq!(effective_len(&over), MAX_SOURCE_BYTES + 1);
        assert!(too_large(&over));

        // Three times the limit in comments, indented ones included.
        let commented = format!(
            "{head}{}",
            "  %% a comment line explaining the design at some length\n".repeat(3500)
        );
        assert!(commented.len() > 3 * MAX_SOURCE_BYTES);
        assert_eq!(effective_len(&commented), head.len());
        assert!(drawn(&commented));
        // A directive is not a comment: the parser reads it.
        let directive = "%%{init: {'theme': 'dark'}}%%\ngraph TD\n a-->b\n";
        assert_eq!(effective_len(directive), directive.len());
        assert!(drawn(directive));
    }

    #[test]
    fn items_at_the_limit_are_drawn_and_one_more_is_not() {
        let gantt = |tasks: usize| {
            let tasks = each(tasks, |i| format!(" t{i} :t{i}, 2024-01-01, 1d\n"));
            format!("gantt\n dateFormat YYYY-MM-DD\n{tasks}")
        };
        let (at, over) = (gantt(MAX_ITEMS), gantt(MAX_ITEMS + 1));
        assert_eq!(counted(&at).0, MAX_ITEMS);
        assert!(drawn(&at));
        assert_eq!(counted(&over).0, MAX_ITEMS + 1);
        assert!(too_large(&over));
    }

    #[test]
    fn nesting_at_the_limit_is_drawn_and_one_level_more_is_not() {
        let nested = |depth: usize| {
            format!(
                "graph TD\n{} a-->b\n{}",
                each(depth, |i| format!("subgraph s{i}\n")),
                "end\n".repeat(depth)
            )
        };
        let (at, over) = (nested(MAX_NESTING), nested(MAX_NESTING + 1));
        assert_eq!(counted(&at).1, MAX_NESTING);
        assert!(drawn(&at));
        assert_eq!(counted(&over).1, MAX_NESTING + 1);
        assert!(too_large(&over));
    }

    /// Nested `{ }` are bounded from the text: class namespaces 1,700 deep
    /// were 1.3 s of parse before anything could count them.
    #[test]
    fn nested_braces_are_bounded_before_the_parse() {
        let namespaces = |depth: usize| {
            format!(
                "classDiagram\n{}class A\n{}",
                each(depth, |i| format!("namespace N{i} {{\n")),
                "}\n".repeat(depth)
            )
        };
        assert_eq!(brace_depth(&namespaces(MAX_NESTING)), MAX_NESTING);
        assert_eq!(admits(&namespaces(MAX_NESTING)), Ok(()));
        assert_eq!(admits(&namespaces(MAX_NESTING + 1)), Err(Refused::TooLarge));
        assert_eq!(admits(&namespaces(1700)), Err(Refused::TooLarge));
        // Siblings do not nest, braces in a comment are not read, and the
        // shapes that double their braces are two deep.
        assert_eq!(brace_depth(&"class A {\n +x\n}\n".repeat(500)), 1);
        assert_eq!(brace_depth(&"%% {{{{{{{{{{\n".repeat(100)), 0);
        assert_eq!(brace_depth("graph TD\n a{{hex}} --> b{rhombus}\n"), 2);

        // An ER diagram's crow's feet are braces that never close: 300
        // relationships are not 300 levels, whatever comes before the
        // header. Anything else that opens that many is.
        let feet = each(300, |i| format!(" T{i} ||--o{{ T{} : has\n", i + 1));
        for head in [
            "erDiagram\n",
            "\n%% the schema\n  erDiagram\n",
            "---\ntitle: Schema {draft\n---\n%%{init: {'theme': 'dark'}}%%\nerDiagram\n",
        ] {
            let er = format!("{head}{feet}");
            assert_eq!(brace_depth(&er), 0, "{head:?}");
            assert!(drawn(&er), "{head:?}");
        }
        assert_eq!(brace_depth(&format!("classDiagram\n{feet}")), 300);
        assert_eq!(
            brace_depth(&format!("---\nx: erDiagram\n---\nclassDiagram\n{feet}")),
            300,
            "front matter does not name the family"
        );
    }

    /// `&` lists are bounded from the text, because the parser multiplies
    /// them out before anything can be counted: 14 KB was a million edges.
    #[test]
    fn ampersand_lists_are_bounded_before_the_parse() {
        let list = |p: &str, k: usize| {
            (0..k)
                .map(|i| format!("{p}{i}"))
                .collect::<Vec<_>>()
                .join(" & ")
        };
        let product = |k: usize| format!("graph TD\n {} --> {}\n", list("a", k), list("b", k));
        // Two lists of 22 on one line: 42 ampersands, (42 + 1)² = 1,849.
        assert_eq!(fan_out(&product(22)), 1849);
        assert_eq!(admits(&product(22)), Ok(()));
        assert!(drawn(&product(22)));
        assert_eq!(fan_out(&product(23)), 2025);
        assert_eq!(admits(&product(23)), Err(Refused::TooLarge));
        assert!(too_large(&product(23)));

        // The million-edge source is inside the byte limit, and is refused
        // by `admits` — which `render` asks before it parses anything.
        let huge = product(1000);
        assert!(huge.len() < MAX_SOURCE_BYTES);
        assert_eq!(admits(&huge), Err(Refused::TooLarge));

        // The bound is never under the edges the parser really makes,
        // wherever the arrows sit: on the lists' line, on the next, or
        // chaining three lists.
        for k in [2, 5, 11, 14] {
            for (src, names) in [
                (product(k), 2 * k),
                (product(k).replace(" --> ", "\n --> "), 2 * k),
                (
                    format!(
                        "graph TD\n {} --> {}\n --> {}\n",
                        list("a", k),
                        list("b", k),
                        list("c", k)
                    ),
                    3 * k,
                ),
            ] {
                let edges = counted(&src).0 - names;
                assert!(edges >= k * k, "the product is real: {src}");
                assert!(fan_out(&src) >= edges, "{} < {edges}: {src}", fan_out(&src));
            }
        }

        // Ordinary use, and `&` in labels, pass.
        assert!(drawn(
            "graph TD\n a & b --> c & d\n e[R & D] --> f[Q & A]\n"
        ));
    }

    /// The bound in [`fan_out`] leans on one fact about the parser: a list
    /// stops at a line break. If a `merman` bump changes that, this fails
    /// and the bound has to be re-derived.
    #[test]
    fn an_ampersand_list_cannot_continue_on_the_next_line() {
        let r = Renderer::new();
        for src in [
            "graph TD\n a &\n b --> c\n",
            "graph TD\n a\n & b --> c\n",
            "graph TD\n a &\r b --> c\n",
        ] {
            assert!(
                r.0.engine
                    .parse_diagram_for_render_model_sync(src, r.0.parse)
                    .is_err(),
                "{src:?} parsed"
            );
        }
    }

    /// A flowchart's three limits, each alone. Top-down, where reaching
    /// them costs milliseconds; what they exist for is left to right.
    #[test]
    fn a_flowchart_at_each_of_its_limits_is_drawn_and_just_past_it_is_not() {
        // Nodes: that many, and no edges.
        let lonely = |n: usize| format!("graph TD\n{}", each(n, |i| format!(" n{i}\n")));
        assert_eq!(flow_cost(&lonely(MAX_FLOW_NODES)).nodes, MAX_FLOW_NODES);
        assert!(drawn(&lonely(MAX_FLOW_NODES)));
        assert!(too_large(&lonely(MAX_FLOW_NODES + 1)));

        // Work: 400 nodes in two ranks, 400 edges; then one edge more.
        let ranks = |extra: &str| {
            format!(
                "graph TD\n{}{extra}",
                each(200, |i| format!(
                    " a{i}-->b{}\n a{i}-->b{}\n",
                    (i * 7) % 200,
                    (i * 13 + 5) % 200
                ))
            )
        };
        let at = flow_cost(&ranks(""));
        assert_eq!((at.nodes, at.work), (400, MAX_FLOW_WORK));
        assert!(at.hubs < MAX_FLOW_HUBS / 10, "{at:?}");
        assert!(drawn(&ranks("")));
        assert!(flow_cost(&ranks(" a0-->a1\n")).work > MAX_FLOW_WORK);
        assert!(too_large(&ranks(" a0-->a1\n")));

        // Hubs: one node with 199 children is 199² + 199.
        let fan = |dir: &str, n: usize| {
            format!("graph {dir}\n{}", each(n, |i| format!(" root-->n{i}\n")))
        };
        assert_eq!(flow_cost(&fan("TD", 199)).hubs, 39_800);
        assert!(drawn(&fan("TD", 199)));
        assert_eq!(flow_cost(&fan("TD", 200)).hubs, 40_200);
        assert!(too_large(&fan("TD", 200)));
        // The shapes the limits were set by: left to right, 499 children
        // took 2.7 s, 1,500 more than a minute.
        assert!(too_large(&fan("LR", 499)));
        assert!(too_large(&fan("LR", 1500)));
    }

    /// Class and ER diagrams are laid out as graphs too, by a different
    /// layout with a different cost: 300 classes with 1,600 relations were
    /// 1,900 items, inside every limit, and 4 to 6.5 s of one core — more
    /// than the whole document's budget. Their bound is nodes² × edges.
    ///
    /// At the limit with a cheap shape (everything related to one class);
    /// the costly one, relations at random, is only ever refused here.
    #[test]
    fn a_class_or_er_diagram_at_its_work_limit_is_drawn_and_one_relation_more_is_not() {
        let class = |n: usize, e: usize| {
            format!(
                "classDiagram\n{}",
                each(e, |i| format!(" C0 --> C{} : uses\n", 1 + i % (n - 1)))
            )
        };
        let er = |n: usize, e: usize| {
            format!(
                "erDiagram\n{}",
                each(e, |i| format!(" E0 ||--o{{ E{} : has\n", 1 + i % (n - 1)))
            )
        };
        // 200² × 750 is the limit exactly.
        assert_eq!(200 * 200 * 750, MAX_RELATION_WORK);
        for diagram in [class, er] {
            assert_eq!(relation_work(&diagram(200, 750)), Some(MAX_RELATION_WORK));
            assert!(drawn(&diagram(200, 750)));
            assert!(relation_work(&diagram(200, 751)) > Some(MAX_RELATION_WORK));
            assert!(too_large(&diagram(200, 751)));
        }
        // The reported shapes: 300 classes or entities, 1,600 relations at
        // random. And an ordinary large one, a hundred classes with 150
        // relations among them, is a twentieth of the limit.
        let mut rng = Lcg(7);
        let mut pairs = |n: usize, e: usize, line: &dyn Fn(usize, usize) -> String| {
            each(e, |_| line(rng.below(n), rng.below(n)))
        };
        let class_line = |a: usize, b: usize| format!(" C{a} --> C{b} : uses\n");
        let er_line = |a: usize, b: usize| format!(" E{a} ||--o{{ E{b} : has\n");
        assert!(too_large(&format!(
            "classDiagram\n{}",
            pairs(300, 1600, &class_line)
        )));
        assert!(too_large(&format!(
            "erDiagram\n{}",
            pairs(300, 1600, &er_line)
        )));
        let ordinary = format!("classDiagram\n{}", pairs(100, 150, &class_line));
        assert!(relation_work(&ordinary) < Some(MAX_RELATION_WORK / 20));
        assert!(drawn(&ordinary));
    }

    #[test]
    fn a_sequence_diagram_at_the_grid_estimate_is_drawn_and_just_past_it_is_not() {
        // Ten participants and 380 messages; one more character of label
        // moves the estimate by ten columns over 2,296 rows.
        let seq = |label: usize| {
            format!(
                "sequenceDiagram\n{}{}",
                each(10, |i| format!("participant P{i}\n")),
                format!("P0->>P1: {}\n", "m".repeat(label)).repeat(380)
            )
        };
        let (at, over) = (seq(70), seq(71));
        let (estimate, real) = sequence_grids(&at);
        assert!(
            estimate <= MAX_SEQUENCE_CELLS && real <= estimate,
            "{estimate} {real}"
        );
        assert!(drawn(&at));
        let (estimate, _) = sequence_grids(&over);
        assert!(estimate > MAX_SEQUENCE_CELLS, "{estimate}");
        assert!(counted(&over).0 <= MAX_ITEMS, "refused for its grid alone");
        assert!(too_large(&over));
    }

    /// Every feature found to grow a sequence diagram's grid, each alone.
    fn sequence_shapes() -> Vec<(&'static str, String)> {
        let rep = |s: &str, n: usize| s.repeat(n);
        vec![
            ("chain", each(40, |i| format!("P{i}->>P{}: m\n", i + 1))),
            ("tall", rep("A->>B: m\n", 300)),
            (
                "wide",
                each(120, |i| format!("participant P{i}\n")) + "P0->>P1: m\n",
            ),
            ("long label", format!("A->>B: {}\n", rep("x", 3000))),
            (
                "long actor",
                format!("participant A as {}\nA->>B: m\n", rep("n", 2000)),
            ),
            ("title", format!("title {}\nA->>B: m\n", rep("T", 2000))),
            (
                "self",
                format!("A->>A: {}\n{}", rep("s", 200), rep("A->>A: again\n", 100)),
            ),
            (
                "create",
                "A->>B: m\n".to_string()
                    + &each(60, |i| format!("create participant C{i}\nA->>C{i}: hi\n")),
            ),
            ("wrap", format!("A->>B: wrap: {}\n", rep("word ", 400))),
            (
                "breaks",
                format!("A->>B: m\nNote over A,B: {}\n", rep("y<br/>", 200)),
            ),
            (
                "notes",
                "A->>B: m\n".to_string()
                    + &rep(&format!("Note right of B: {}\n", rep("z", 60)), 100),
            ),
            (
                "note left",
                "A->>B: m\n".to_string()
                    + &rep(&format!("Note left of A: {}\n", rep("z", 300)), 20),
            ),
            (
                "activations",
                rep("A->>+B: go\n", 100) + &rep("B-->>-A: back\n", 100),
            ),
            (
                "par",
                each(50, |i| {
                    format!("par p{i}\nA->>B: m\nand q\nB->>A: n\nend\n")
                }),
            ),
            (
                "alt nest",
                rep("alt a\n", 40) + "A->>B: m\n" + &rep("else b\nB->>A: n\nend\n", 40),
            ),
            (
                "loop nest",
                rep("loop l\n", 60) + "A->>B: m\n" + &rep("end\n", 60),
            ),
            (
                "rect nest",
                rep("rect rgb(0,0,0)\n", 50) + "A->>B: m\n" + &rep("end\n", 50),
            ),
            (
                "loop label",
                format!("loop {}\nA->>B: m\nend\n", rep("L", 2000)),
            ),
            (
                "boxes",
                each(50, |i| format!("box G{i}\nparticipant P{i}\nend\n")) + "P0->>P49: m\n",
            ),
            (
                "autonumber",
                "autonumber\n".to_string() + &rep("A->>B: m\n", 200),
            ),
            (
                "spanning",
                each(40, |i| format!("P{i}->>P{}: {}\n", i + 1, rep("w", 80))),
            ),
            (
                "note span",
                each(40, |i| format!("participant P{i}\n"))
                    + &format!("Note over P0,P39: {}\n", rep("k", 2000)),
            ),
        ]
    }

    /// The estimate is only a limit if the real grid stays under it.
    #[test]
    fn the_sequence_estimate_is_never_under_the_real_grid() {
        for (name, body) in sequence_shapes() {
            let (estimate, real) = sequence_grids(&format!("sequenceDiagram\n{body}"));
            assert!(
                real <= estimate,
                "{name}: {real} cells, estimated {estimate}"
            );
        }
    }

    /// The art limit is cells, never a width or a height: a chain of a
    /// hundred nodes under one very long label is a thousand rows of a
    /// thousand columns.
    #[test]
    fn art_at_the_cell_limit_is_kept_and_one_column_more_is_not() {
        let chain = |label: usize| {
            format!(
                "graph TD\n n0[{}]-->n1\n{}",
                "x".repeat(label),
                each(99, |i| format!(" n{}-->n{}\n", i + 1, i + 2))
            )
        };
        let size = |src: &str| {
            Renderer::new().render(src).map(|l| {
                (
                    l.len(),
                    l.iter().map(|l| l.chars().count()).max().unwrap_or(0),
                )
            })
        };
        // 1,005 rows; the box is its label and four cells of frame.
        assert_eq!(size(&chain(991)), Ok((1005, 995)));
        assert_eq!(
            MAX_ART_CELLS / 1005,
            995,
            "one column either side of the limit"
        );
        assert_eq!(size(&chain(992)), Err(Some(Refused::TooLarge)));
    }

    #[test]
    fn a_document_asks_for_a_bounded_amount_of_source_in_document_order() {
        // Each block is 16 KiB of source, comments mostly, so the budget is
        // a whole number of them; a block refused on its own in the middle
        // costs none of it, and each kind of refusal is reported as itself.
        let small = format!("graph TD\n a-->b\n%% {}\n", "c".repeat(16 * 1024 - 20));
        assert_eq!(small.len(), 16 * 1024);
        let fits = MAX_DOC_SOURCE_BYTES / small.len();
        let huge = format!("graph TD\n a-->b\n{}\n", " ".repeat(MAX_SOURCE_BYTES));
        let md = format!(
            "```mermaid\n{huge}```\n\n{}",
            format!("```mermaid\n{small}```\n\n").repeat(fits + 5)
        );
        let plan = plan(&Document::parse(&md));
        assert_eq!(plan.requests.len(), fits);
        assert!(plan.requests.iter().all(|(_, s)| s.len() == small.len()));
        assert!(
            plan.requests.windows(2).all(|w| w[0].0 < w[1].0),
            "the first ones in the document are the ones drawn"
        );
        let first = plan.requests[0].0;
        let last = plan.requests[fits - 1].0;
        let of = |kind: Refused| -> Vec<BlockIdx> {
            let kept = plan.refused.iter().filter(|(_, why)| *why == kind);
            kept.map(|(b, _)| *b).collect()
        };
        let (large, budget) = (of(Refused::TooLarge), of(Refused::Budget));
        assert_eq!(large.len(), 1);
        assert!(large[0] < first, "the oversized block came first");
        assert_eq!(budget.len(), 5);
        assert!(budget.iter().all(|b| *b > last));
    }

    #[test]
    fn a_document_keeps_a_bounded_amount_of_art_and_says_what_it_dropped() {
        // One node with a 996-character label: five rows of 1,000 cells.
        let src = format!("graph TD\n a[{}]\n", "x".repeat(996));
        let each_cells = art_cells(&Renderer::new().render(&src).expect("drawn"));
        assert_eq!(each_cells, 5000);
        let fits = MAX_DOC_ART_CELLS / each_cells;
        let reqs: Vec<Request> = (0..fits + 50)
            .map(|i| (BlockIdx(u32::try_from(i).unwrap()), src.clone()))
            .collect();
        let replies: Vec<Reply> = spawn(only(reqs)).iter().collect();
        assert_eq!(replies.len(), fits + 50, "every block is answered");
        assert!(replies[..fits].iter().all(|(_, r)| r.is_ok()));
        assert!(
            replies[fits..]
                .iter()
                .all(|(_, r)| *r == Err(Refused::Budget)),
            "art past the budget is not sent, and its blocks say why"
        );
    }

    /// What the text alone refuses reaches the reader too, before anything
    /// is drawn; and a diagram refused after its parse is answered in turn.
    #[test]
    fn the_worker_reports_each_refusal_with_its_reason() {
        let fan = each(300, |i| format!(" root-->n{i}\n"));
        let nest = "namespace N {\n".repeat(100);
        let md = format!(
            "```mermaid\ngraph TD\n a-->b\n```\n\n```mermaid\ngraph LR\n{fan}```\n\n\
             ```mermaid\nclassDiagram\n{nest}```\n\n```mermaid\npie\n \"A\": 1\n```\n"
        );
        let plan = plan(&Document::parse(&md));
        assert_eq!(plan.requests.len(), 3, "the brace nest is not even copied");
        assert_eq!(plan.refused.len(), 1);
        let nest = plan.refused[0].0;
        let (first, fan) = (plan.requests[0].0, plan.requests[1].0);
        let replies: Vec<Reply> = spawn(plan).iter().collect();
        let why = |b: BlockIdx| {
            replies
                .iter()
                .find(|(block, _)| *block == b)
                .map(|(_, r)| r.as_ref().map(|_| ()).map_err(|e| *e))
        };
        assert_eq!(why(nest), Some(Err(Refused::TooLarge)));
        assert_eq!(why(fan), Some(Err(Refused::TooLarge)));
        assert_eq!(why(first), Some(Ok(())));
        assert_eq!(replies.len(), 3, "the pie chart is not a refusal");
    }
}
