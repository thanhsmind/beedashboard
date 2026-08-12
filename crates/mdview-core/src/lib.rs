//! mdview-core — domain + application + adapters for the mdview markdown server.
//!
//! Dependency rule (PRD §7.4): this crate never depends on Axum/Tauri. Adapters
//! (SQLite, notify) live here behind ports; the HTTP/MCP/CLI wiring is in the
//! `mdview` binary crate.

pub mod ansi;
pub mod bee;
pub mod config;
pub mod daemon;
pub mod doc_links;
pub mod domain;
pub mod engine;
pub mod error;
pub mod fuzzy;
pub mod indexer;
pub mod link_resolver;
pub mod notify_store;
pub mod paths_boundary;
pub mod process;
pub mod render;
pub mod repository;
pub mod short_link;
pub mod transcript;

pub use config::Config;
pub use engine::Engine;
pub use error::{Error, Result};
pub use repository::SqliteStore;
