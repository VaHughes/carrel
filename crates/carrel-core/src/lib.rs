//! # carrel-core
//!
//! The document model, search, and layout primitives for [Carrel] — a quiet place
//! to read your markdown.
//!
//! **This crate has no UI dependencies and must never acquire any.** No ratatui,
//! no crossterm, no gtk4, no ANSI. `scripts/check-discipline.sh` enforces that
//! mechanically, and CI runs it. The reasoning is in architecture.md (private notes repo) §0
//! and idea.md §9.3 (notes repo); the short version is that Helix's own architecture
//! document records its "frontend-agnostic" view layer becoming terminal-tied,
//! and this crate is the guard against repeating that.
//!
//! ## The one invariant
//!
//! > There is exactly one authoritative coordinate space: a byte offset into a
//! > flattened, unwrapped display text. Screen row, wrap column, and highlight
//! > rectangle are *derived* functions of `(document, width)` — recomputed on
//! > resize, never stored. A search hit recorded at width 80 is bit-for-bit the
//! > same value at width 40.
//!
//! Everything else follows. Search state cannot be invalidated by reflow because
//! no search state is ever expressed in display coordinates.
//!
//! ## Quick start
//!
//! ```
//! use carrel_core::{Document, search};
//!
//! // The source hard-wraps the phrase; the reader shows one paragraph.
//! let doc = Document::parse("the quick\nbrown fox");
//! let hits = search(&doc, "quick brown", true);
//! assert_eq!(hits.len(), 1);
//! ```
//!
//! [Carrel]: https://github.com/VaHughes/carrel

pub mod diff;
pub mod document;
pub mod highlight;
pub mod layout;
pub mod math;
pub mod position;
pub mod search;

pub use diff::{looks_like_diff, to_markdown};
pub use document::{
    AlertKind, Document, Inline, LinkId, Marker, Node, NodeKind, Prefix, Prov, ProvKind, Style,
};
pub use highlight::{Token, TokenKind};
pub use layout::{
    CHUNK_BYTES, CONTINUATION_COLS, Row, RowKind, WidthFn, chunk_count, cluster_at_col,
    cluster_width, cols_for_doc_range, display_width, wrap, wrap_chunk, wrap_range,
};
pub use math::{MathClass, MathExpr, MatrixDelim};
pub use position::{Affinity, BlockIdx, DocByte, NodeId, SrcByte};
/// Re-exported so a frontend can hold what [`content_pattern`] returns
/// without its own `regex` dependency line.
pub use regex::Regex;
pub use search::{Matches, content_pattern, search};
