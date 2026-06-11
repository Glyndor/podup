//! `podup` — docker-compose → Podman translator library.
//!
//! Provides parsing, variable substitution, topological ordering, and an
//! async engine that drives container lifecycle via Podman's native libpod
//! REST API over a Unix socket or Windows named pipe.

pub mod compose;
pub(crate) mod dotenv;
pub(crate) mod engine;
pub mod env_file;
pub(crate) mod error;
pub(crate) mod libpod;
pub mod podman;
pub mod ports;
pub mod quadlet;
pub mod size;
pub mod substitute;
pub mod update;

pub use compose::{
	parse_file, parse_file_with_env_files, parse_files_with_env_files, parse_str, parse_str_raw,
	resolve_order,
};
pub use engine::{Engine, ProjectLock, RunOptions};
pub use error::{ComposeError, Result};
pub use libpod::Client;
