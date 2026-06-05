//! `podup` — docker-compose → Podman translator library.
//!
//! Provides parsing, variable substitution, topological ordering, and an
//! async engine that drives container lifecycle via Podman's Docker-compatible
//! REST API (bollard).

pub mod compose;
pub mod engine;
pub mod env_file;
pub mod error;
pub mod podman;
pub mod ports;
pub mod size;
pub mod substitute;

pub use compose::{parse_file, parse_str, parse_str_raw, resolve_order};
pub use engine::Engine;
pub use error::{ComposeError, Result};
