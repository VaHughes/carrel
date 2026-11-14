//! # carrel-state
//!
//! The part of [Carrel] above the document model that every frontend shares.
//! The terminal reader is one frontend; a GTK4 one is designed for and not
//! written. Both link this crate and [`carrel_core`] in-process — the shape
//! research.md Q29 found holding up wherever two frontends ship (Ghostty,
//! Transmission's GTK and macOS apps, Fractal).
//!
//! **No UI dependency, and no terminal geometry in the public API.** Nothing
//! here names a cell, a row on screen, a pixel or a colour. Positions are
//! byte offsets into [`carrel_core::Document::text`] or into the source, the
//! one coordinate space the core defines.
//!
//! - [`intent`]: what a reader can ask for, the frontend-neutral half of the
//!   terminal's `Action`.
//! - [`config`] and [`state`]: what carrel remembers, in the files every
//!   frontend reads — preferences, and reading positions, bookmarks and
//!   collapsed sections, saved in coordinates that outlive a release
//!   ([`state::Place`]). [`marginalia`]: highlights and notes.
//! - [`scan`], [`grep`], [`tags`], [`links`], [`wiki`], [`fuzzy`]: finding
//!   documents — the library walk, content search, the tag scan, link and
//!   wikilink resolution, the filter's fuzzy match.
//! - [`clip`]: what may go on the clipboard — the screen's text, without
//!   the controls a paste into a shell would act on.
//!
//! **No API-stability promise.** This crate is versioned in lockstep with
//! `carrel` and `carrel-core`, released from one tag, and each depends on the
//! others at exactly one version. It is published so that `cargo install
//! carrel` can build, not as a library for other projects.
//!
//! [Carrel]: https://github.com/VaHughes/carrel

pub mod clip;
pub mod config;
pub mod fuzzy;
pub mod grep;
pub mod intent;
pub mod links;
pub mod marginalia;
pub mod scan;
pub mod state;
pub mod tags;
pub mod wiki;
