//! CLI integration tests for the host-binding / privilege-escalation
//! warning emitter (#1358).
//!
//! The compose-file paths these tests exercise do not require a Podman
//! daemon: `config` walks the parsed model without contacting one,
//! `generate quadlet` is purely file-system work, and the warning
//! emit happens before any API call on `up`/`create`/`run`. So the
//! suite can run on a host without Podman the same way the parse /
//! `cli_diagnostics` tests do.
//!
//! This crate root holds the helpers every scenario file inherits through
//! `use super::*`, plus one `mod` line per scenario. The rows themselves live
//! in the files declared below.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use tempfile::TempDir;

fn bin() -> &'static str {
	env!("CARGO_BIN_EXE_podup")
}

fn compose_file(name: &str, contents: &str) -> (TempDir, PathBuf) {
	let dir = tempfile::tempdir().expect("create temp dir");
	let path = dir.path().join(name);
	fs::write(&path, contents).expect("write compose file");
	(dir, path)
}

// The `config` surface: every active mode warns, `--no-warn` does not
// silence it, and a clean compose file stays silent.
#[path = "host_binding_warnings/config_surface.rs"]
mod config_surface;

// The `generate quadlet` surface: the warnings are emitted at generate time
// because Quadlet has no Podman client to surface them through.
#[path = "host_binding_warnings/quadlet_surface.rs"]
mod quadlet_surface;

// The parse-time port-exposure warning, and which commands surface it.
#[path = "host_binding_warnings/port_exposure.rs"]
mod port_exposure;
