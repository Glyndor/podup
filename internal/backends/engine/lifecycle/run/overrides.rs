//! How `run` turns its flags into overrides of the service a one-off copies.

use crate::compose::types::{Command, Service};
use crate::error::{ComposeError, Result};

/// Split a `--entrypoint` value into words the way Compose does with
/// `shellwords.Parse`: whitespace separates words, single quotes keep everything,
/// double quotes keep everything except a backslash escape, and a backslash
/// outside quotes escapes the next character. `Ok(vec![])` for an empty string.
pub(super) fn split_words(text: &str) -> Result<Vec<String>> {
	let mut words = Vec::new();
	let mut word = String::new();
	let mut in_word = false;
	let mut chars = text.chars();
	while let Some(c) = chars.next() {
		match c {
			c if c.is_whitespace() => {
				if in_word {
					words.push(std::mem::take(&mut word));
					in_word = false;
				}
			}
			'\'' => {
				in_word = true;
				loop {
					match chars.next() {
						Some('\'') => break,
						Some(q) => word.push(q),
						None => return Err(unterminated(text)),
					}
				}
			}
			'"' => {
				in_word = true;
				loop {
					match chars.next() {
						Some('"') => break,
						Some('\\') => match chars.next() {
							Some(e @ ('"' | '\\' | '$' | '`')) => word.push(e),
							Some(e) => {
								word.push('\\');
								word.push(e);
							}
							None => return Err(unterminated(text)),
						},
						Some(q) => word.push(q),
						None => return Err(unterminated(text)),
					}
				}
			}
			'\\' => {
				in_word = true;
				match chars.next() {
					Some(e) => word.push(e),
					None => return Err(unterminated(text)),
				}
			}
			c => {
				in_word = true;
				word.push(c);
			}
		}
	}
	if in_word {
		words.push(word);
	}
	Ok(words)
}

fn unterminated(text: &str) -> ComposeError {
	ComposeError::Unsupported(format!(
		"--entrypoint {text:?}: unterminated quote or trailing backslash"
	))
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
