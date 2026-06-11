//! `podup` — docker-compose → Podman translator library.
//!
//! Provides parsing, variable substitution, topological ordering, and an
//! async engine that drives container lifecycle via Podman's native libpod
//! REST API over a Unix socket or Windows named pipe.

pub mod compose;
pub(crate) mod engine;
pub mod env_file;
pub(crate) mod error;
pub(crate) mod libpod;
pub mod podman;
pub mod ports;
pub mod size;
pub mod substitute;

pub use compose::{parse_file, parse_str, parse_str_raw, resolve_order};
pub use engine::{Engine, RunOptions};
pub use error::{ComposeError, Result};
pub use libpod::Client;
