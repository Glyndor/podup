//! How `run` turns its flags into overrides of the service a one-off copies.

use crate::compose::types::{Command, Service};
use crate::error::{ComposeError, Result};

/// Split a `--entrypoint` value into words with the rules Compose uses for a
/// string `command:` or `entrypoint:` (see [`split_words`](crate::compose::types::command::split_words)).
/// `Ok(vec![])` for an empty string.
pub(super) fn split_words(text: &str) -> Result<Vec<String>> {
	crate::compose::types::command::split_words(text)
		.map_err(|why| ComposeError::Unsupported(format!("--entrypoint {text:?}: {why}")))
}

/// Apply `run`'s `--entrypoint` and trailing command to the one-off's copy of
/// the service, as `docker compose run` does: the entrypoint is split into words,
/// an empty one clears the image entrypoint, and an entrypoint given with no
/// command also drops the command the service declared, which belonged to the
/// entrypoint it replaced.
pub(super) fn apply_entrypoint_and_command(
	service: &mut Service,
	entrypoint: Option<String>,
	command: Vec<String>,
) -> Result<()> {
	if let Some(ep) = entrypoint {
		let mut words = split_words(&ep)?;
		if words.is_empty() {
			// `[""]` is how libpod is told to drop the image's entrypoint.
			words.push(String::new());
		}
		service.entrypoint = Some(Command::Exec(words));
		if command.is_empty() {
			service.command = Some(Command::Exec(Vec::new()));
		}
	}
	if !command.is_empty() {
		service.command = Some(Command::Exec(command));
	}
	Ok(())
}

/// A one-off runs once: neither the service's `restart:` nor its
/// `deploy.restart_policy` may bring it back.
pub(super) fn run_once(service: &mut Service) {
	service.restart = None;
	if let Some(deploy) = service.deploy.as_mut() {
		deploy.restart_policy = None;
	}
}

#[cfg(test)]
#[path = "overrides_tests.rs"]
mod tests;
