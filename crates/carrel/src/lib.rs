//! Carrel's terminal frontend.
//!
//! A library as well as a binary so integration tests can drive the reader —
//! a `[[bin]]`-only crate exposes nothing to `tests/`.
//!
//! # The boundary that matters
//!
//! Two layers sit under this one. [`carrel_core`] is the document model;
//! `carrel_state` is what every frontend shares above it — the intents a
//! reader can ask for, and (by the end of the GUI preparation, research.md
//! Q29) persistence and file discovery. Neither may depend on a UI crate,
//! and the compiler holds them to it.
//!
//! Inside this crate, [`action`], [`app`], [`layout`], [`view`] and the rest
//! of the state machine are **terminal-free**: they compile without ratatui
//! or crossterm, so behaviour is tested with no terminal. Only [`keys`]
//! (the key-event dispatcher), [`theme`], [`render`] and the event loop in
//! `main.rs` touch the terminal. Rule 6 of `scripts/check-discipline.sh`
//! lists the terminal-free modules and checks the whole closure — no
//! terminal crate named, and no `crate::` edge from the list to anything off
//! it — which is what the old direct-import grep missed while `app` reached
//! crossterm through `keys`. What the keys are CALLED is data in [`keymap`].
//!
//! These modules are still shaped by the terminal (`App` is sized in cells),
//! which is why they are not in `carrel_state`: a crate boundary blocks
//! dependencies, not shapes.

pub mod action;
pub mod annotation_render;
pub mod annotation_state;
pub mod ansi;
pub mod app;
pub mod breadcrumb;
pub mod cli;
pub mod config;
pub mod diagrams;
pub mod footer;
pub mod fuzzy;
pub mod grep;
pub mod home;
pub mod images;
pub mod keymap;
pub mod keys;
pub mod layout;
pub mod links;
pub mod marginalia;
pub mod math_art;
pub mod menu;
pub mod omarchy;
pub mod peek;
pub mod plain;
pub mod render;
pub mod scan;
pub mod state;
pub mod status;
pub mod stream;
pub mod tags;
pub mod theme;
pub mod view;
pub mod wiki;
