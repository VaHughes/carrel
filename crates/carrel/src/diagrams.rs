//! Mermaid → Unicode box art, off-thread — wave F, Q24.
//!
//! Same shape as the image pipeline: requests collected from the document,
//! rendered on a background thread, results drained per frame, and art
//! arrival is just another reflow through the anchor machinery. Rendering is
//! best-effort decoration: an unsupported family or a parse error simply
//! sends nothing, and the block stays syntax-highlighted source.
//!
//! NO RATATUI — `scripts/check-discipline.sh` rule 6.

use std::sync::mpsc::{self, Receiver};

use carrel_core::{BlockIdx, Document, NodeKind};

/// One mermaid block worth rendering: `(block, source)`.
pub type Request = (BlockIdx, String);

/// Every ` ```mermaid ` block's source, in layout order.
#[must_use]
pub fn requests(doc: &Document) -> Vec<Request> {
    (0..doc.block_count())
        .map(|i| BlockIdx(u32::try_from(i).unwrap_or(u32::MAX)))
        .filter_map(|b| {
            let node = doc.node_for_block(b);
            match &node.kind {
                NodeKind::CodeBlock { lang: Some(l) } if &**l == "mermaid" => {
                    let src = doc.text[node.doc.start as usize..node.doc.end as usize].to_string();
                    Some((b, src))
                }
                _ => None,
            }
        })
        .collect()
}

/// Render each request off-thread; successes stream back as art lines.
#[must_use]
pub fn spawn(reqs: Vec<Request>) -> Receiver<(BlockIdx, Vec<String>)> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let renderer = merman::ascii::HeadlessAsciiRenderer::new();
        for (block, src) in reqs {
            let Ok(Some(art)) = renderer.render_ascii_sync(&src) else {
                continue; // unsupported family or parse error: keep the source
            };
            let lines: Vec<String> = art.lines().map(str::to_string).collect();
            if lines.is_empty() {
                continue;
            }
            if tx.send((block, lines)).is_err() {
                return;
            }
        }
    });
    rx
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
}
