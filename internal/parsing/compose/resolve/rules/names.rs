//! The keys of the top-level `networks`, `volumes`, `secrets` and `configs`
//! must be plain ASCII.
//!
//! The compose specification limits those keys to ASCII letters, digits, `.`,
//! `_` and `-`. Anything outside ASCII lets a name hold a right-to-left
//! override, a zero-width character or a letter from another script that looks
//! like a Latin one, so two names read the same or one reads as another. Other
//! ASCII punctuation is left to the layers that build file names and unit
//! names, which refuse it where it matters.

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};

/// Reject a top-level key with a character outside ASCII or a control
/// character. The message quotes the key with `{:?}`, which prints an
/// invisible or reordering character as an escape, so the error cannot carry
/// the character it complains about.
pub(in crate::parsing::compose::resolve) fn validate_resource_names(
	file: &ComposeFile,
) -> Result<()> {
	let groups: [(&str, Vec<&String>); 4] = [
		("network", file.networks.keys().collect()),
		("volume", file.volumes.keys().collect()),
		("secret", file.secrets.keys().collect()),
		("config", file.configs.keys().collect()),
	];
	for (kind, names) in groups {
		for name in names {
			if !is_plain(name) {
				return Err(ComposeError::Invalid(format!(
					"{kind} name {name:?} is invalid: use only ASCII characters"
				)));
			}
		}
	}
	Ok(())
}

fn is_plain(name: &str) -> bool {
	!name.is_empty() && name.chars().all(|c| c.is_ascii() && !c.is_ascii_control())
}
