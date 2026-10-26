//! The "tear down by label" predicate and its tests.
//!
//! `down -p NAME` with no compose file present is the label-only teardown:
//! matching `docker compose -p NAME down` against a running project whose
//! compose file is absent.

use crate::cli::Commands;

/// Whether a `down` invocation should tear the project down purely by its
/// `podup.project` label: the command is `down`, an explicit project name was
/// given (`-p` / `COMPOSE_PROJECT_NAME`), and no compose file resolves on disk.
/// Matches `docker compose -p NAME down` against a running project whose compose
/// file is absent. Pure so it is unit-testable without a Podman daemon.
pub(crate) fn down_by_label_path(
	command: &Commands,
	project: Option<&str>,
	compose_present: bool,
) -> bool {
	matches!(command, Commands::Down { .. }) && project.is_some() && !compose_present
}

#[cfg(test)]
#[path = "down_by_label_tests.rs"]
mod down_by_label_tests;
