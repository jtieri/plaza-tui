//! The Plaza TUI application.
//!
//! This library hosts the terminal UI and the application state machine; the
//! `plaza-tui` binary is a thin shell that parses arguments, sets up logging, and
//! calls [`app::run`]. Domain logic lives in the sibling crates: [`plaza_api`] for
//! the backend and [`plaza_audio`] for playback.

pub mod app;
pub mod config;
pub mod theme;
pub mod tui;
