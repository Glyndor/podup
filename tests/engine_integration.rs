//! Integration tests that exercise the engine against a real Podman daemon.
//!
//! All tests skip gracefully when Podman is not reachable. In CI the
//! `podman` input to the rust-ci reusable starts the socket and sets
//! `PODMAN_SOCKET` before the coverage gate runs.
//!
//! The test bodies are split across the `engine_integration/` submodules to
//! keep each file under the source line limit. Shared helpers live here at the
//! crate root so the submodules can reach them via `use super::*;`.
use std::fs;

use podup::{parse_files_with_env_files, parse_str, Client, Engine};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn podman() -> Option<Client> {
	let client = podup::podman::connect_from_env()
		.or_else(|_| podup::podman::connect(None))
		.ok()?;
	client.ping().await.ok()?;
	Some(client)
}

/// Unique project name per test run + per test to avoid parallel conflicts.
fn proj(tag: &str) -> String {
	format!("t{}-{}", std::process::id(), tag)
}

/// Path to the built `podup` binary, for the CLI tests.
fn bin() -> &'static str {
	env!("CARGO_BIN_EXE_podup")
}

// ---------------------------------------------------------------------------
// Test groups (see engine_integration/*.rs)
// ---------------------------------------------------------------------------

#[path = "engine_integration/group_a1.rs"]
mod group_a1;
#[path = "engine_integration/group_a2.rs"]
mod group_a2;
#[path = "engine_integration/group_a3.rs"]
mod group_a3;
#[path = "engine_integration/group_a4.rs"]
mod group_a4;
#[path = "engine_integration/group_b1.rs"]
mod group_b1;

#[cfg(feature = "test-helpers")]
#[path = "engine_integration/watch.rs"]
mod watch_tests;

#[path = "engine_integration/cli1.rs"]
mod cli1;
#[path = "engine_integration/cli2.rs"]
mod cli2;
#[path = "engine_integration/scale.rs"]
mod scale;
