//! Translate a parsed compose file into Podman Quadlet unit files.
//!
//! Quadlet is Podman's systemd integration: declarative `.container`,
//! `.network` and `.volume` units placed under
//! `~/.config/containers/systemd/` that a systemd generator turns into
//! services, so systemd owns the lifecycle (boot, restart, dependencies)
//! instead of a long-running `podup` process.
//!
//! This is an additive export path, not a replacement for the runner. It
//! maps the common compose fields and warns — loudly, never silently — for
//! every field that is set but has no Quadlet equivalent yet, so generated
//! units never quietly drop configuration.

mod render;
mod unit;
mod warnings;

use crate::compose::types::ComposeFile;
use unit::{container_unit, network_unit, volume_unit};

/// A single generated unit file: its name and full contents.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct QuadletUnit {
	/// File name, e.g. `web.container` or `db-data.volume`.
	pub filename: String,
	/// Full file contents, ending in a newline.
	pub contents: String,
}

/// The result of a generation run: the units plus any warnings about set but
/// unmapped fields.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct QuadletOutput {
	/// Generated unit files, in a deterministic order.
	pub units: Vec<QuadletUnit>,
	/// Human-readable warnings for compose fields with no Quadlet mapping.
	pub warnings: Vec<String>,
}

impl QuadletOutput {
	/// The first unit file name that two different units share, if any. Distinct
	/// compose keys can sanitize to the same stem (e.g. `web:1` and `web_1` both
	/// become `web_1`); writing them would silently overwrite one unit, dropping
	/// a service/network/volume from the export. Callers surface this as an error
	/// instead of clobbering.
	pub fn duplicate_filename(&self) -> Option<&str> {
		let mut seen = std::collections::HashSet::new();
		self.units
			.iter()
			.find(|u| !seen.insert(u.filename.as_str()))
			.map(|u| u.filename.as_str())
	}
}

/// Translate a compose file into Quadlet units for the given project name.
///
/// Emits one `.container` per service, one `.network` per declared network,
/// and one `.volume` per declared named volume. Replica scaling, build
/// services, and other fields without a Quadlet mapping are reported as
/// warnings rather than silently dropped.
pub fn generate(file: &ComposeFile, project: &str) -> QuadletOutput {
	let mut out = QuadletOutput::default();

	// External networks/volumes are assumed to pre-exist. Emitting a unit would
	// make systemd try to (re-)create them, so skip them here; the container unit
	// references such resources by their existing name instead.
	for (name, cfg) in &file.networks {
		if cfg.as_ref().is_some_and(|c| c.external == Some(true)) {
			continue;
		}
		out.units.push(network_unit(name, project, cfg.as_ref()));
	}
	for (name, cfg) in &file.volumes {
		if cfg.as_ref().is_some_and(|c| c.external == Some(true)) {
			continue;
		}
		out.units.push(volume_unit(name, project, cfg.as_ref()));
	}

	let declared_volumes: Vec<&str> = file
		.volumes
		.iter()
		.filter(|(_, cfg)| cfg.as_ref().is_none_or(|c| c.external != Some(true)))
		.map(|(name, _)| name.as_str())
		.collect();
	let declared_networks: Vec<&str> = file
		.networks
		.iter()
		.filter(|(_, cfg)| cfg.as_ref().is_none_or(|c| c.external != Some(true)))
		.map(|(name, _)| name.as_str())
		.collect();
	for (name, service) in &file.services {
		out.units.push(container_unit(
			name,
			service,
			&declared_volumes,
			&declared_networks,
			&mut out.warnings,
		));
	}

	out
}

#[cfg(test)]
mod tests;
