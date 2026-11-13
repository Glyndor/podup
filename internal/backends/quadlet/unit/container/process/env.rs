//! The environment a container runs with: `environment:` and `env_file:`.

use std::path::Path;

use crate::compose::types::Service;

use super::super::{abs_against, sorted_pairs, Section};

/// Emit one `Environment=` line per compose environment entry, sorted by name.
pub(in super::super) fn emit_environment(service: &Service, container: &mut Section) {
	for (key, val) in sorted_pairs(service.environment.to_map()) {
		match val {
			Some(v) => container.add("Environment", format!("{key}={v}")),
			None => container.add("Environment", key),
		}
	}
}

/// Emit one `EnvironmentFile=` line per `env_file:` entry, reporting a file
/// Podman would read differently from `up`.
pub(in super::super) fn emit_env_files(
	name: &str,
	service: &Service,
	base_dir: &Path,
	container: &mut Section,
	warnings: &mut Vec<String>,
) {
	// `EnvironmentFile=` is resolved by podman-systemd.unit(5) against the unit
	// file's own directory, not the compose file's. Units are installed to
	// `~/.config/containers/systemd`, so `env_file: .env` would render to a unit
	// looking for `~/.config/containers/systemd/.env`, and `--env-file` on a
	// missing path is fatal, so the container never starts. Resolve against the
	// compose base directory, the same way the build context is.
	for entry in service.env_file.to_entries() {
		let path = abs_against(base_dir, entry.path());
		let keys = super::env_file_check::keys_read_differently(&entry, Path::new(&path), base_dir);
		if !keys.is_empty() {
			warnings.push(format!(
				"{name}: env_file {:?} uses dotenv syntax (quotes, `export`, inline comments \
				 or `${{VAR}}`) that `podman --env-file` reads as written, so the container \
				 gets other values for: {}",
				entry.path(),
				keys.join(", ")
			));
		}
		container.add("EnvironmentFile", path);
	}
}
