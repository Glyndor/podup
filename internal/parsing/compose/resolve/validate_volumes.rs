//! Top-level volume checks.

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};

/// An `external: true` volume is used as it is: the engine never creates it, so
/// a `driver`, `driver_opts` or `labels` next to it would be dropped without a
/// word. Docker Compose refuses the file, naming the first of them, and so does
/// this. `name:` is allowed: it says which existing volume to use.
pub(super) fn validate_volumes(file: &ComposeFile) -> Result<()> {
	for (name, cfg) in &file.volumes {
		let Some(cfg) = cfg else { continue };
		if cfg.external != Some(true) {
			continue;
		}
		let conflict = if cfg.driver.is_some() {
			Some("driver")
		} else if !cfg.driver_opts.is_empty() {
			Some("driver_opts")
		} else if !cfg.labels.is_empty() {
			Some("labels")
		} else {
			None
		};
		if let Some(field) = conflict {
			return Err(ComposeError::Unsupported(format!(
				"volume '{}' is external but also sets '{field}'; an external volume is \
				 used as it is and its attributes cannot be applied to it",
				crate::error::sanitize_name(name)
			)));
		}
	}
	Ok(())
}

#[cfg(test)]
#[path = "validate_volumes_tests.rs"]
mod tests;
