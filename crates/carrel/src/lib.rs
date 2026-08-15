//! Carrel's terminal frontend.
//!
//! A library as well as a binary so integration tests can drive the reader —
//! a `[[bin]]`-only crate exposes nothing to `tests/`.
//!
//! # The boundary that matters
//!
//! [`action`], [`app`], [`layout`] and [`view`] are **pure state and must never
//! import ratatui**. A GTK frontend reuses them verbatim; only [`keys`],
//! [`theme`] and [`render`] are terminal-specific. `scripts/check-discipline.sh`
//! rule 6 enforces this mechanically, and has been verified to fail on an
//! injected violation.

pub mod action;
pub mod app;
pub mod config;
pub mod diagrams;
pub mod footer;
pub mod grep;
pub mod home;
pub mod images;
pub mod keys;
pub mod layout;
pub mod math_art;
pub mod plain;
pub mod render;
pub mod scan;
pub mod state;
pub mod theme;
pub mod view;
pub mod wiki;
