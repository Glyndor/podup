//! Integration tests that exercise the engine against a real Podman daemon.
//!
//! All tests skip gracefully when Podman is not reachable, so they are safe to
//! run on a machine without it. Set `PODUP_REQUIRE_PODMAN=1` where Podman is
//! guaranteed — the nested-virt lane does — and an unreachable Podman becomes a
//! hard failure rather than a suite that reports `ok` having run nothing.
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
	let connected =
		match podup::podman::connect_from_env().or_else(|_| podup::podman::connect(None)) {
			Ok(client) => client.ping().await.is_ok().then_some(client),
			Err(_) => None,
		};
	// Skipping is the right default: these tests must not fail on a developer
	// machine without Podman. But a silent skip reports `ok` for a test that
	// executed nothing, and libtest counts it as passed — so an environment
	// where Podman never came up looks identical to a clean run. Somewhere that
	// Podman is guaranteed (the nested-virt lane), set PODUP_REQUIRE_PODMAN and
	// the skip becomes a hard failure instead of a green lie.
	assert!(
		!(connected.is_none() && std::env::var_os("PODUP_REQUIRE_PODMAN").is_some()),
		"PODUP_REQUIRE_PODMAN is set but Podman is unreachable — refusing to report this suite as passing without running it"
	);
	connected
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

#[path = "engine_integration/build_resources.rs"]
mod build_resources;
#[path = "engine_integration/commands_networking.rs"]
mod commands_networking;
#[path = "engine_integration/cp_flags.rs"]
mod cp_flags;
#[path = "engine_integration/exec_flags.rs"]
mod exec_flags;
#[path = "engine_integration/health_targeting.rs"]
mod health_targeting;
#[path = "engine_integration/lifecycle.rs"]
mod lifecycle;
#[path = "engine_integration/niche.rs"]
mod niche;
#[path = "engine_integration/resources_health.rs"]
mod resources_health;
#[path = "engine_integration/run_flags.rs"]
mod run_flags;

#[cfg(feature = "test-helpers")]
#[path = "engine_integration/watch.rs"]
mod watch_tests;

#[path = "engine_integration/cli_commands.rs"]
mod cli_commands;
#[path = "engine_integration/cli_flags.rs"]
mod cli_flags;
#[path = "engine_integration/cli_lifecycle.rs"]
mod cli_lifecycle;
#[path = "engine_integration/create_ls.rs"]
mod create_ls;
#[path = "engine_integration/lifecycle_output.rs"]
mod lifecycle_output;
#[path = "engine_integration/scale.rs"]
mod scale;
#[path = "engine_integration/stats_flags.rs"]
mod stats_flags;
