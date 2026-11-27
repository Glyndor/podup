//! The environment a container runs with: `environment:` and `env_file:`.

use std::path::Path;

use crate::compose::types::Service;
use crate::error::sanitize_name;

use super::super::{abs_against, quote_podman_arg_value, sorted_pairs, Section};
use super::env_values::fits_env_file;

/// Emit the lines for the compose environment entries the env file of the
/// service does not carry, sorted by name.
///
/// An entry with a value goes to that file (see [`super::env_values`]), because
/// Quadlet puts every `Environment=KEY=VALUE` on the `podman run` command line.
/// Only an entry the file cannot give back as written stays an `Environment=`
/// line, and is reported.
///
/// A bare `KEY`, or a null in a mapping, is "unset and removed from the service
/// container environment" in the Compose Specification. `Environment=KEY`
/// alone (`podman run --env KEY`) takes the value from the environment of the
/// process that starts the container, and leaves the image's own value of
/// `KEY` when there is none. Measured on Podman 6.2.0 (2026-11-23): `--env
/// container` on the Fedora image gives `container=podman`, `--unsetenv
/// container` removes it, both together remove it when the variable is not
/// defined and keep the defined value when it is. Quadlet has no `UnsetEnv=`
/// key (the generator rejects it as unsupported), so the removal goes through
/// `PodmanArgs=--unsetenv=`, next to the bare `Environment=` line.
pub(in super::super) fn emit_environment(
	name: &str,
	service: &Service,
	container: &mut Section,
	warnings: &mut Vec<String>,
) {
	for (key, val) in sorted_pairs(service.environment.to_map()) {
		match val {
			Some(v) if fits_env_file(&key, &v) => {}
			Some(v) => {
				warnings.push(format!(
					"{}: environment variable {} does not fit an env file, so its value is on \
					 the podman command line",
					sanitize_name(name),
					sanitize_name(&key)
				));
				container.add("Environment", format!("{key}={v}"));
			}
			None => {
				container.add("Environment", key.clone());
				container.add(
					"PodmanArgs",
					format!("--unsetenv={}", quote_podman_arg_value(&key)),
				);
			}
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
	let entries = service.env_file.to_entries();
	// What each file gives the container under `podman --env-file`, next to
	// what it gives it under `up`, which resolves the files, the service's own
	// `environment:` and the project environment together.
	let reads = super::env_file_check::reads(&entries, &service.environment.to_map(), base_dir);
	for (entry, read) in entries.iter().zip(reads) {
		let path = abs_against(base_dir, entry.path());
		if !read.is_empty() {
			warnings.push(format!(
				"{name}: env_file {:?} uses dotenv syntax (quotes, `export`, inline comments \
				 or `${{VAR}}`) that `podman --env-file` reads as written, so the container \
				 gets other values for: {}",
				entry.path(),
				read.detail()
			));
		}
		container.add("EnvironmentFile", path);
	}
}
