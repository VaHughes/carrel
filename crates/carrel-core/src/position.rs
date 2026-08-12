//! The position type system.
//!
//! See architecture.md (private notes repo) §1. There are four coordinate spaces; only two of
//! them live in this crate, and only one of them is ever persisted.
//!
//! | # | Space  | Type              | Indexed by                | Owner        |
//! |---|--------|-------------------|---------------------------|--------------|
//! | 1 | Source | [`SrcByte`]       | bytes of `Document::source` | `carrel-core` |
//! | 2 | Doc    | [`DocByte`]       | bytes of `Document::text`   | `carrel-core` |
//! | 3 | Layout | `BlockIdx`, rows, cols | one width            | `carrel` (TUI) |
//! | 4 | Screen | terminal cells    | one frame                 | `carrel` (TUI) |
//!
//! **Space 2 is the only space anything persistent lives in.** Spaces 3 and 4 are
//! regenerated every frame and own nothing.
//!
//! # The rule
//!
//! Every persisted position in this system is a `u32` **byte** offset. No `char`
//! indices exist anywhere. There is no `char_to_byte` function; if one appears in
//! this codebase, it is a bug.
//!
//! Emacs proves the cost of the alternative: the entire `src/marker.c` char↔byte
//! apparatus — a bracketing macro, a one-element global cache invalidated by any
//! buffer modification, and an O(distance) byte-by-byte fallback scan — exists
//! *only* because its public position type is a character index while storage is
//! bytes. It collapses to `return charpos;` when the content is all-ASCII.
//!
//! The rule may be broken in exactly three transient places, each bounded by one
//! row or one block: grapheme iteration during line breaking, display-column
//! accumulation, and highlight boundary snapping. Nothing produced there is stored.

use std::fmt;

/// Byte offset into [`Document::source`](crate::Document::source) — space 1.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct SrcByte(pub u32);

/// Byte offset into [`Document::text`](crate::Document::text) — space 2.
///
/// **The canonical position type.** Every persisted position is one of these.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct DocByte(pub u32);

/// Index into [`Document::nodes`](crate::Document::nodes).
///
/// Stable for the lifetime of a `Document`.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub u32);

/// Index into [`Document::layout_order`](crate::Document::layout_order).
///
/// This is what layout iterates, and what the TUI's per-block height and
/// prefix-sum arrays are indexed by. It is **not** a `NodeId`.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockIdx(pub u32);

macro_rules! newtype_debug {
    ($($t:ident),*) => {$(
        impl fmt::Debug for $t {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!(stringify!($t), "({})"), self.0)
            }
        }
        impl $t {
            /// The raw index, for slicing.
            #[must_use]
            pub const fn get(self) -> usize { self.0 as usize }
        }
    )*};
}
newtype_debug!(SrcByte, DocByte, NodeId, BlockIdx);

/// Which side of an exact soft-wrap boundary a position belongs to.
///
/// This is **wrap affinity**, not edit bias. Edit bias — which side of an
/// *insertion* a position lands on — is what Emacs's `insertion_type`,
/// `CodeMirror`'s `assoc`, and Monaco's `TrackedRangeStickiness` express, and
/// `carrel-core` does not need it: the document is immutable between loads.
///
/// Wrap affinity is the only bias in the design. Get it wrong and the viewport
/// jitters one row per resize.
///
/// Note that a *range* uses opposite affinities at its two ends — start biases
/// [`Right`](Affinity::Right), end biases [`Left`](Affinity::Left). In the paint
/// loop this falls out of two half-open comparisons rather than being consulted
/// as a field. See architecture.md (private notes repo) §3.3.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum Affinity {
    /// Attach to the character before: an offset exactly on a wrap point
    /// resolves to the **end of the previous** visual row.
    Left,
    /// Attach to the character after: an offset exactly on a wrap point
    /// resolves to the **start of the next** visual row. Default for anchors.
    #[default]
    Right,
}
