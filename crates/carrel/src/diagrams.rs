//! Mermaid → Unicode box art, off-thread — wave F, Q24.
//!
//! Same shape as the image pipeline: requests collected from the document,
//! rendered on a background thread, results drained per frame, and art
//! arrival is just another reflow through the anchor machinery. Rendering is
//! best-effort decoration: an unsupported family, a parse error or a diagram
//! over a limit below simply sends nothing, and the block stays
//! syntax-highlighted source — the same quiet fallback math takes past its
//! own guard.
//!
//! # A diagram is bounded before it is drawn
//!
//! The renderer's cost is not a function of the source's size. A sequence
//! diagram allocates a grid of participants × messages: 3,000 one-line
//! messages between 3,000 participants are 49 KB of source and more than
//! 4 GB of grid, and the allocation failure aborts the process with the
//! terminal still in the alternate screen. A flowchart node with 200
//! children is 2.5 KB and a third of a second of layout; with 1,500, over a
//! minute. A thread cannot be stopped from outside, so everything is decided
//! from what the PARSE produced, before any layout runs:
//!
//! | limit | value | what it bounds |
//! |---|---|---|
//! | [`MAX_SOURCE_BYTES`] | 16 KiB | the parse itself |
//! | [`MAX_FAN_OUT`] | 400 | what `a & b --> c & d` multiplies out to |
//! | [`MAX_ITEMS`] | 400 | entities + relationships, every family |
//! | [`MAX_NESTING`] | 64 | nested blocks, subgraphs, composite states |
//! | [`MAX_GRID_CELLS`] | 250,000 | the grid of the families that budget one |
//! | [`MAX_SEQUENCE_CELLS`] | 1,000,000 | a sequence diagram's grid, estimated |
//! | [`MAX_ART_ROWS`] × [`MAX_ART_COLS`] | 2,000 × 500 | the art that comes back |
//!
//! and per document: [`MAX_DOC_SOURCE_BYTES`] of source requested,
//! [`MAX_DOC_ART_CELLS`] of art kept, [`RENDER_BUDGET`] of rendering.
//!
//! Measured with a release build and `merman` 0.8.0-alpha.5, as the whole
//! process's CPU time and peak RSS reading a document of one diagram (13 MB
//! and 10 ms for a two-node flowchart), the worst shapes found that pass or
//! reach every limit: a node with 199 children, 0.23 s and 30 MB; class
//! namespaces nested to the source limit, 0.17 s and 26 MB, all parse; a
//! sequence diagram at the estimate's edge, 20 ms and 23 MB. Every other
//! family at the item limit is under 30 ms and 20 MB. **The numbers belong
//! to that version: re-measure before a `merman` bump**, as its pin already
//! demands.
//!
//! NO RATATUI — `scripts/check-discipline.sh` rule 6.

use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use carrel_core::{BlockIdx, Document, NodeKind};
use merman_core::diagram::RenderSemanticModel;
use merman_core::resources::{InputResourceLimitId, InputResourcePolicy, ModelComplexity};

/// One mermaid block worth rendering: `(block, source)`.
pub type Request = (BlockIdx, String);

/// The longest source handed to the parser. Real diagrams are a few hundred
/// bytes to a few kilobytes, and [`MAX_ITEMS`] of anything fit in this with
/// room to spare. The parser is not linear in it everywhere: class
/// namespaces nested as deep as 32 KiB allows took 1.3 s to parse.
pub const MAX_SOURCE_BYTES: usize = 16 * 1024;

/// What the source's `&` lists may multiply out to, by [`fan_out`]'s bound.
///
/// `a & b --> c & d` is four edges, and the parser builds every one before
/// anything can count them: 14 KB of two 1,000-name lists is a million
/// edges, 1.5 s and 1.8 GB of parse. So this one is read off the text.
pub const MAX_FAN_OUT: usize = 400;

/// Entities plus relationships, as `merman` counts them for every family:
/// nodes and edges, participants and messages, tasks, commits. Layout is
/// superlinear in these for more than one family, and a diagram past a few
/// hundred is not readable as box art anyway.
pub const MAX_ITEMS: usize = 400;

/// Text the parsed model may hold. The model repeats a label several times
/// over, so this sits well above [`MAX_SOURCE_BYTES`]; it is a ceiling on
/// what the parse may retain, not the bound on label widths — the grid
/// limits are.
const MAX_TEXT_BYTES: usize = 1024 * 1024;

/// Nested blocks, subgraphs and composite states. Each level costs columns
/// on both sides, so nothing legible is this deep.
pub const MAX_NESTING: usize = 64;

/// The grid the flowchart, class, ER, state and chart renderers may
/// allocate; they check it themselves, before allocating. `merman`'s own
/// interactive default, written down here so that a bump cannot move it.
pub const MAX_GRID_CELLS: usize = 250_000;

/// The grid a sequence diagram may allocate, by [`sequence_cells`]'s
/// estimate. Its renderer budgets nothing, so this is the limit that stands
/// between a small file and an out-of-memory abort. At about 45 bytes a
/// cell, a diagram at the limit is at most some 45 MB while it is drawn.
pub const MAX_SEQUENCE_CELLS: usize = 1_000_000;

/// The tallest and widest art kept. Taller scrolls for pages past a diagram
/// nobody can follow; wider is cut off by every terminal there is.
pub const MAX_ART_ROWS: usize = 2_000;
/// See [`MAX_ART_ROWS`].
pub const MAX_ART_COLS: usize = 500;

/// How much mermaid source one document may ask to have drawn, in document
/// order. Later blocks stay source.
pub const MAX_DOC_SOURCE_BYTES: usize = 1024 * 1024;

/// How many cells of art one document may keep — a few megabytes of
/// strings. Ten thousand diagrams, each inside its own limits, are
/// otherwise gigabytes.
pub const MAX_DOC_ART_CELLS: usize = 4_000_000;

/// How long one document's diagrams may take to draw, all told. The limits
/// above keep any one diagram well under a second; this is for the document
/// made of thousands of them, whose reader would otherwise have a core busy
/// for as long as the document is open.
pub const RENDER_BUDGET: Duration = Duration::from_secs(5);

/// Every ` ```mermaid ` block's source, in layout order — those inside
/// [`MAX_SOURCE_BYTES`] each, until [`MAX_DOC_SOURCE_BYTES`] is spent.
#[must_use]
pub fn requests(doc: &Document) -> Vec<Request> {
    let mut spent = 0usize;
    (0..doc.block_count())
        .map(|i| BlockIdx(u32::try_from(i).unwrap_or(u32::MAX)))
        .filter_map(|b| {
            let node = doc.node_for_block(b);
            match &node.kind {
                NodeKind::CodeBlock { lang: Some(l) } if &**l == "mermaid" => {
                    let src = &doc.text[node.doc.start as usize..node.doc.end as usize];
                    // Asked here as well as in `Renderer::render`, so an
                    // oversized block is never even copied.
                    if !admits(src) || spent.saturating_add(src.len()) > MAX_DOC_SOURCE_BYTES {
                        return None;
                    }
                    spent += src.len();
                    Some((b, src.to_string()))
                }
                _ => None,
            }
        })
        .collect()
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
            (InputResourceLimitId::MaxSourceBytes, MAX_SOURCE_BYTES),
            (InputResourceLimitId::MaxModelItems, MAX_ITEMS),
            (InputResourceLimitId::MaxModelTextBytes, MAX_TEXT_BYTES),
            (InputResourceLimitId::MaxModelNestingDepth, MAX_NESTING),
        ] {
            // Refused only for a zero, which none of these is; a test reads
            // the limits back off the policy.
            let _ = policy.apply_limit(id, value);
        }
        let ascii =
            merman::ascii::AsciiRenderOptions::default().with_max_grid_cells(MAX_GRID_CELLS);
        Self(
            merman::ascii::HeadlessAsciiRenderer::new()
                .with_resource_policy(policy)
                .with_ascii_options(ascii),
        )
    }

    /// The art for one diagram, or `None` when it stays source: a family
    /// the renderer does not draw, a parse error, or any limit of this
    /// module — each decided before the step it protects.
    #[must_use]
    pub fn render(&self, src: &str) -> Option<Vec<String>> {
        if !admits(src) {
            return None;
        }
        let parsed = self
            .0
            .engine
            .parse_diagram_for_render_model_sync(src, self.0.parse)
            .ok()??;
        let model = parsed.model();
        // The renderer checks its policy again on the way in; asked here
        // first so the sequence estimate below never walks an oversized
        // model.
        self.0.resource_policy().check_render_model(model).ok()?;
        if let RenderSemanticModel::Sequence(seq) = model
            && sequence_cells(seq, ModelComplexity::from_render_model(model).nesting_depth)
                > MAX_SEQUENCE_CELLS
        {
            return None;
        }
        let art = self.0.render_model(model).ok()?;
        let mut lines = Vec::new();
        for line in art.lines() {
            if lines.len() == MAX_ART_ROWS || line.chars().nth(MAX_ART_COLS).is_some() {
                return None;
            }
            lines.push(line.to_string());
        }
        (!lines.is_empty()).then_some(lines)
    }
}

/// May this source be parsed at all? The two limits that are read off the
/// text, because the parse is what they bound.
#[must_use]
pub fn admits(src: &str) -> bool {
    src.len() <= MAX_SOURCE_BYTES && fan_out(src) <= MAX_FAN_OUT
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
/// with dozens of them its drawing and nothing else.
#[must_use]
pub fn fan_out(src: &str) -> usize {
    src.lines()
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
/// quarter of its estimate, which puts the limit where the other families'
/// [`MAX_GRID_CELLS`] is.
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

/// Render each request off-thread; successes stream back as art lines.
///
/// The thread stops at the first of: the last request, the receiver gone
/// (the document was closed or replaced — checked between diagrams, and one
/// diagram is bounded), [`MAX_DOC_ART_CELLS`] of art sent, or
/// [`RENDER_BUDGET`] spent.
#[must_use]
pub fn spawn(reqs: Vec<Request>) -> Receiver<(BlockIdx, Vec<String>)> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let renderer = Renderer::new();
        let started = Instant::now();
        let mut cells = 0usize;
        for (block, src) in reqs {
            if started.elapsed() > RENDER_BUDGET {
                return;
            }
            let Some(lines) = renderer.render(&src) else {
                continue; // stays source
            };
            cells = cells.saturating_add(art_cells(&lines));
            if cells > MAX_DOC_ART_CELLS || tx.send((block, lines)).is_err() {
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

    #[test]
    fn only_mermaid_blocks_become_requests() {
        let d = Document::parse(
            "para\n\n```mermaid\ngraph TD\n a-->b\n```\n\n```rust\nfn x() {}\n```\n",
        );
        let reqs = requests(&d);
        assert_eq!(reqs.len(), 1);
        assert!(reqs[0].1.contains("graph TD"), "{:?}", reqs[0].1);
    }

    #[test]
    fn a_flowchart_renders_and_an_unsupported_family_stays_silent() {
        let flow = vec![(BlockIdx(0), "graph TD\n a[alpha]-->b[beta]\n".to_string())];
        let rx = spawn(flow);
        let (block, lines) = rx.recv().expect("art for the flowchart");
        assert_eq!(block, BlockIdx(0));
        assert!(lines.len() > 3, "box art has real height: {lines:?}");
        assert!(
            lines.iter().any(|l| l.contains("alpha")),
            "labels survive: {lines:?}"
        );

        let pie = vec![(BlockIdx(1), "pie\n \"A\" : 40\n".to_string())];
        let rx = spawn(pie);
        assert!(rx.recv().is_err(), "unsupported family sends nothing");
    }

    // --- the limits, each on both sides -------------------------------
    //
    // Every fixture here is small: the diagram past a limit is refused
    // before the step that would cost anything, and where a broken limit
    // would let it through, it is a few megabytes, never the gigabytes the
    // limits exist for.

    /// `line(0)`, `line(1)`, … one after another: a generated diagram's body.
    fn each(n: usize, line: impl Fn(usize) -> String) -> String {
        let mut s = String::new();
        for i in 0..n {
            s += &line(i);
        }
        s
    }

    fn drawn(src: &str) -> bool {
        Renderer::new().render(src).is_some()
    }

    /// What `merman` counts in `src`: `(items, nesting)`.
    fn counted(src: &str) -> (usize, usize) {
        let r = Renderer::new();
        let parsed =
            r.0.engine
                .parse_diagram_for_render_model_sync(src, r.0.parse)
                .expect("parses")
                .expect("a known family");
        let c = ModelComplexity::from_render_model(parsed.model());
        (c.items, c.nesting_depth)
    }

    /// [`sequence_cells`] for `src`, and the grid `merman` really made of
    /// it with no limit of ours in the way: `(estimate, rows × cols)`.
    fn sequence_grids(src: &str) -> (usize, usize) {
        let r = merman::ascii::HeadlessAsciiRenderer::new();
        let parsed = r
            .engine
            .parse_diagram_for_render_model_sync(src, r.parse)
            .expect("parses")
            .expect("a known family");
        let RenderSemanticModel::Sequence(seq) = parsed.model() else {
            panic!("not a sequence diagram: {src:?}");
        };
        let nesting = ModelComplexity::from_render_model(parsed.model()).nesting_depth;
        let art = r.render_model(parsed.model()).expect("renders");
        let cols = art.lines().map(|l| l.chars().count()).max().unwrap_or(0);
        (sequence_cells(seq, nesting), art.lines().count() * cols)
    }

    #[test]
    fn the_limits_are_set_on_the_renderer_that_draws() {
        let r = Renderer::new();
        let policy = r.0.resource_policy();
        for (id, value) in [
            (InputResourceLimitId::MaxSourceBytes, MAX_SOURCE_BYTES),
            (InputResourceLimitId::MaxModelItems, MAX_ITEMS),
            (InputResourceLimitId::MaxModelTextBytes, MAX_TEXT_BYTES),
            (InputResourceLimitId::MaxModelNestingDepth, MAX_NESTING),
        ] {
            assert_eq!(policy.value(id), Some(value), "{id:?}");
        }
        assert_eq!(r.0.ascii.max_grid_cells, MAX_GRID_CELLS);
    }

    /// The reported file: 3,000 participants, 49 KB, more than 4 GB of
    /// grid. And the same shape small enough to pass the source limit —
    /// 400 participants are 6 KB and were 244 MB.
    #[test]
    fn a_sequence_diagram_of_thousands_of_participants_stays_source() {
        let chain = |n: usize| {
            let messages = each(n, |i| format!("P{i}->>P{}: m\n", i + 1));
            format!("sequenceDiagram\n{messages}")
        };
        assert!(!drawn(&chain(3000)));
        assert!(!drawn(&chain(400)));
        assert!(!drawn(&chain(100)));
        assert!(drawn(&chain(12)), "an ordinary one is still drawn");

        let doc = Document::parse(&format!("```mermaid\n{}```\n", chain(3000)));
        assert!(requests(&doc).is_empty(), "and is never even copied");
    }

    #[test]
    fn a_source_at_the_byte_limit_is_drawn_and_one_byte_more_is_not() {
        let head = "graph TD\n a[alpha]-->b[beta]\n%% ";
        let at = format!("{head}{}\n", "c".repeat(MAX_SOURCE_BYTES - head.len() - 1));
        assert_eq!(at.len(), MAX_SOURCE_BYTES);
        assert!(drawn(&at));
        let over = format!("{head}{}\n", "c".repeat(MAX_SOURCE_BYTES - head.len()));
        assert_eq!(over.len(), MAX_SOURCE_BYTES + 1);
        assert!(!drawn(&over));
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
        assert!(!drawn(&over));

        // The shape the limit was set by: one node with 199 children took a
        // third of a second, 1,500 more than a minute.
        let fan = |n: usize| format!("graph LR\n{}", each(n, |i| format!(" root-->n{i}\n")));
        assert!(counted(&fan(200)).0 > MAX_ITEMS);
        assert!(!drawn(&fan(200)));
        assert!(!drawn(&fan(1500)));
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
        assert!(!drawn(&over));
    }

    /// `&` lists are bounded from the text, because the parser multiplies
    /// them out before anything can be counted: 14 KB was a million edges.
    #[test]
    fn ampersand_lists_are_bounded_before_the_parse() {
        let product = |k: usize| {
            let list = |p: &str| {
                (0..k)
                    .map(|i| format!("{p}{i}"))
                    .collect::<Vec<_>>()
                    .join(" & ")
            };
            format!("graph TD\n {} --> {}\n", list("a"), list("b"))
        };
        // Two lists of ten on one line: 18 ampersands, (18 + 1)² = 361.
        assert_eq!(fan_out(&product(10)), 361);
        assert!(admits(&product(10)));
        assert!(drawn(&product(10)));
        assert_eq!(fan_out(&product(11)), 441);
        assert!(!admits(&product(11)));
        assert!(!drawn(&product(11)));

        // The million-edge source is inside the byte limit, and is refused
        // by `admits` — which `render` asks before it parses anything.
        let huge = product(1000);
        assert!(huge.len() < MAX_SOURCE_BYTES);
        assert!(!admits(&huge));

        // The bound is never under the edges the parser really makes,
        // wherever the arrows sit: on the lists' line, on the next, or
        // chaining three lists.
        for k in [2, 5, 11, 14] {
            let list = |p: &str| {
                (0..k)
                    .map(|i| format!("{p}{i}"))
                    .collect::<Vec<_>>()
                    .join(" & ")
            };
            for (src, names) in [
                (product(k), 2 * k),
                (product(k).replace(" --> ", "\n --> "), 2 * k),
                (
                    format!(
                        "graph TD\n {} --> {}\n --> {}\n",
                        list("a"),
                        list("b"),
                        list("c")
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
        let (at, over) = (seq(27), seq(28));
        let (estimate, real) = sequence_grids(&at);
        assert!(
            estimate <= MAX_SEQUENCE_CELLS && real <= estimate,
            "{estimate} {real}"
        );
        assert!(drawn(&at));
        let (estimate, _) = sequence_grids(&over);
        assert!(estimate > MAX_SEQUENCE_CELLS, "{estimate}");
        assert!(counted(&over).0 <= MAX_ITEMS, "refused for its grid alone");
        assert!(!drawn(&over));
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

    #[test]
    fn art_at_the_size_limit_is_kept_and_a_row_or_a_column_more_is_not() {
        let tall = |members: usize| {
            format!(
                "classDiagram\n class C {{\n{} }}\n",
                "  a\n".repeat(members)
            )
        };
        let rows = |src: &str| Renderer::new().render(src).map(|l| l.len());
        // The box is four rows of frame and title around its members.
        assert_eq!(rows(&tall(MAX_ART_ROWS - 4)), Some(MAX_ART_ROWS));
        assert_eq!(rows(&tall(MAX_ART_ROWS - 3)), None);

        let wide = |label: usize| format!("graph TD\n a[{}]\n", "x".repeat(label));
        let cols = |src: &str| {
            Renderer::new()
                .render(src)
                .map(|l| l.iter().map(|l| l.chars().count()).max().unwrap_or(0))
        };
        assert_eq!(cols(&wide(MAX_ART_COLS - 4)), Some(MAX_ART_COLS));
        assert_eq!(cols(&wide(MAX_ART_COLS - 3)), None);
    }

    #[test]
    fn a_document_asks_for_a_bounded_amount_of_source_in_document_order() {
        // Each block is 1 KiB of source, so the budget is a whole number
        // of them, and an oversized one in the middle costs nothing.
        let small = format!("graph TD\n a-->b\n%% {}\n", "c".repeat(1024 - 20));
        assert_eq!(small.len(), 1024);
        let fits = MAX_DOC_SOURCE_BYTES / small.len();
        let huge = format!("graph TD\n a-->b\n%% {}\n", "c".repeat(MAX_SOURCE_BYTES));
        let md = format!(
            "```mermaid\n{huge}```\n\n{}",
            format!("```mermaid\n{small}```\n\n").repeat(fits + 5)
        );
        let doc = Document::parse(&md);
        let reqs = requests(&doc);
        assert_eq!(reqs.len(), fits);
        assert!(reqs.iter().all(|(_, s)| s.len() == small.len()));
        assert!(
            reqs.windows(2).all(|w| w[0].0 < w[1].0),
            "the first ones in the document are the ones drawn"
        );
    }

    #[test]
    fn a_document_keeps_a_bounded_amount_of_art() {
        // One node with a 396-character label: five rows of 400 cells.
        let src = format!("graph TD\n a[{}]\n", "x".repeat(396));
        let each = art_cells(&Renderer::new().render(&src).expect("drawn"));
        assert_eq!(each, 2000);
        let fits = MAX_DOC_ART_CELLS / each;
        let reqs: Vec<Request> = (0..fits + 50)
            .map(|i| (BlockIdx(u32::try_from(i).unwrap()), src.clone()))
            .collect();
        let got: Vec<BlockIdx> = spawn(reqs).iter().map(|(b, _)| b).collect();
        assert_eq!(got.len(), fits, "art past the budget is not sent");
        assert_eq!(
            got.last(),
            Some(&BlockIdx(u32::try_from(fits - 1).unwrap()))
        );
    }
}
