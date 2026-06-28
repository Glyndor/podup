//! `volumes` — list the named volumes declared by a compose project.
//!
//! Mirrors `docker compose volumes [SERVICE...]`: with no services it lists
//! every top-level `volumes:` entry; with services it lists only the named
//! volumes those services mount. Anonymous/bind mounts are not listed (they
//! have no top-level name), matching docker compose.

use std::collections::BTreeSet;

use crate::compose::types::{ComposeFile, VolumeMount};
use crate::error::Result;

use super::super::Engine;

/// Options for [`Engine::list_volumes`], mirroring `docker compose volumes`.
#[derive(Default)]
pub struct VolumesOptions {
	/// Print only volume names, `-q/--quiet`.
	pub quiet: bool,
	/// Emit a JSON array instead of the table, `--format json`.
	pub json: bool,
}

impl Engine {
	/// List the project's named volumes (`docker compose volumes`). When
	/// `services` is non-empty, only volumes mounted by those services are shown.
	pub async fn list_volumes(
		&self,
		file: &ComposeFile,
		services: &[String],
		opts: VolumesOptions,
	) -> Result<()> {
		// Reject an unknown service name (docker compose errors with "no such
		// service") instead of silently filtering it out and printing nothing.
		for s in services {
			if !file.services.contains_key(s) {
				return Err(crate::error::ComposeError::ServiceNotFound(s.clone()));
			}
		}
		let keys = self.selected_volume_keys(file, services);

		// (declared key, resolved on-host name, driver, external)
		let rows: Vec<(String, String, String, bool)> = keys
			.iter()
			.map(|key| {
				let cfg = file.volumes.get(key.as_str()).and_then(|c| c.as_ref());
				let external = cfg.and_then(|c| c.external).unwrap_or(false);
				let name = match cfg.and_then(|c| c.name.as_deref()) {
					Some(n) => n.to_string(),
					None if external => key.to_string(),
					None => format!("{}_{}", self.project, key),
				};
				let driver = cfg
					.and_then(|c| c.driver.clone())
					.unwrap_or_else(|| "local".into());
				(key.to_string(), name, driver, external)
			})
			.collect();

		if opts.quiet {
			for (_, name, _, _) in &rows {
				println!("{name}");
			}
			return Ok(());
		}
		if opts.json {
			let arr: Vec<_> = rows
				.iter()
				.map(|(_, name, driver, external)| {
					serde_json::json!({ "Name": name, "Driver": driver, "External": external })
				})
				.collect();
			println!("{}", serde_json::to_string_pretty(&arr).unwrap_or_default());
			return Ok(());
		}

		crate::ui::print_bold_header(&format!("{:<40} {:<12}", "NAME", "DRIVER"));
		for (_, name, driver, _) in &rows {
			println!("{name:<40} {driver:<12}");
		}
		Ok(())
	}

	/// The top-level volume keys to list: all of them, or just those mounted by
	/// `services` (in declaration order), deduplicated.
	fn selected_volume_keys(&self, file: &ComposeFile, services: &[String]) -> Vec<String> {
		if services.is_empty() {
			return file.volumes.keys().cloned().collect();
		}
		let used: BTreeSet<String> = services
			.iter()
			.filter_map(|s| file.services.get(s))
			.flat_map(|svc| svc.volumes.iter().filter_map(mount_source_name))
			.filter(|src| file.volumes.contains_key(src))
			.collect();
		file.volumes
			.keys()
			.filter(|k| used.contains(k.as_str()))
			.cloned()
			.collect()
	}
}

/// The source (named-volume) component of a mount, if any. Bind mounts and
/// anonymous volumes (no source) return `None`.
fn mount_source_name(m: &VolumeMount) -> Option<String> {
	match m {
		VolumeMount::Short(s) => {
			let parts: Vec<&str> = s.splitn(3, ':').collect();
			// `src:target[:opts]` — a leading `.`/`/`/`~` is a bind path, not a name.
			if parts.len() >= 2 && !parts[0].starts_with(['.', '/', '~']) {
				Some(parts[0].to_string())
			} else {
				None
			}
		}
		VolumeMount::Long { source, .. } => source.clone(),
	}
}

#[cfg(test)]
mod tests {
	use super::mount_source_name;
	use crate::compose::types::VolumeMount;

	#[test]
	fn named_volume_short_form_has_source() {
		assert_eq!(
			mount_source_name(&VolumeMount::Short("data:/var/lib".into())),
			Some("data".to_string())
		);
	}

	#[test]
	fn bind_and_anonymous_have_no_source() {
		assert_eq!(
			mount_source_name(&VolumeMount::Short("./host:/c".into())),
			None
		);
		assert_eq!(
			mount_source_name(&VolumeMount::Short("/abs:/c".into())),
			None
		);
		assert_eq!(mount_source_name(&VolumeMount::Short("/data".into())), None);
	}
}
