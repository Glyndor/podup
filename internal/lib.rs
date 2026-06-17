//! `podup` — docker-compose → Podman translator library.
//!
//! Provides parsing, variable substitution, topological ordering, and an
//! async engine that drives container lifecycle via Podman's native libpod
//! REST API over a Unix socket or Windows named pipe.

// `unsafe` is denied crate-wide; the few modules that need libc FFI opt back in
// locally with `#![allow(unsafe_code)]` and a soundness comment per block, so a
// new `unsafe` block elsewhere fails the build.
#![deny(unsafe_code)]

pub mod compose;
pub(crate) mod dotenv;
pub(crate) mod engine;
pub mod env_file;
pub(crate) mod error;
pub(crate) mod filesystem;
pub(crate) mod libpod;
pub mod podman;
pub mod ports;
pub mod quadlet;
pub mod size;
pub mod substitute;
#[cfg(feature = "update")]
pub mod update;

pub use compose::{
	collect_diagnostics, parse_file, parse_file_with_env_files, parse_files_with_env_files,
	parse_str, parse_str_raw, resolve_levels, resolve_order,
};
pub use engine::{
	is_safe_project_name, list_projects, BuildOptions, Engine, ExecOptions, ImagesOptions,
	LsOptions, ProjectLock, PsOptions, PushOptions, RunOptions,
};
pub use error::{ComposeError, Result};
pub use libpod::Client;

/// Internal parsers exposed only under `test-helpers` for fuzzing and tests.
///
/// These are not part of the public API (the feature is off by default, so the
/// published crate does not expose them); they let the fuzz harness reach the
/// crate-private dotenv parser and the libpod stream framer.
#[cfg(feature = "test-helpers")]
pub mod fuzz_api {
	pub use crate::dotenv::parse as dotenv_parse;
	pub use crate::libpod::types::stream::{parse_frame, take_json_line};
}
