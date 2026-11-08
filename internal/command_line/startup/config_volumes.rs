//! Named volumes in the output of `config`.
//!
//! A volume declared as `data: {}` or `data:` has no keys, so the pruning of
//! empty values dropped it from the output, and a service that mounts `data`
//! then referred to a volume the printed file never declared: the output could
//! not be read back. Docker Compose prints the name the volume will have, which
//! is also what keeps the entry from being empty, so this does the same.

use podup::compose::types::ComposeFile;

/// Stamp each declared volume with the name `up` creates for it:
/// `<project>_<key>`, unless the file names it or marks it `external: true`,
/// which keep what they say.
pub(super) fn inject_resolved_volume_names(file: &mut ComposeFile, project: &str) {
	for (key, slot) in file.volumes.iter_mut() {
		let cfg = slot.get_or_insert_with(Default::default);
		if cfg.name.is_none() && !cfg.external.unwrap_or(false) {
			cfg.name = Some(format!("{project}_{key}"));
		}
	}
}

#[cfg(test)]
#[path = "config_volumes_tests.rs"]
mod tests;
