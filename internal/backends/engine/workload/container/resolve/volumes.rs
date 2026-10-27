use std::path::Path;

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};

/// Resolve a named-volume reference to the volume name `create_volumes`
/// produced: a custom `name:`, the raw name for an external volume, or the
/// `{project}_{name}` form.
///
/// An empty reference is an anonymous volume (an image `VOLUME` directive or a
/// colon-less short-form mount); it carries no name, so Podman assigns one and
/// it is returned unchanged. A non-empty reference that is *not* declared under
/// the top-level `volumes:` map is rejected, matching the compose-spec: leaving
/// it unprefixed would make Podman auto-create a bare, unlabelled global volume
/// that escapes project namespacing and is never reclaimed by `down -v`.
pub(in crate::backends::engine::workload::container) fn resolve_volume_name(
	reference: &str,
	project: &str,
	file: &ComposeFile,
) -> Result<String> {
	if reference.is_empty() {
		return Ok(String::new());
	}
	match file.volumes.get(reference) {
		Some(cfg) => Ok(
			if let Some(name) = cfg.as_ref().and_then(|c| c.name.as_deref()) {
				name.to_string()
			} else if cfg.as_ref().and_then(|c| c.external).unwrap_or(false) {
				reference.to_string()
			} else {
				default_volume_name(reference, project)
			},
		),
		None => Err(ComposeError::Unsupported(format!(
			"service refers to undefined volume \"{reference}\"; declare it under the \
			 top-level `volumes:` key, or use a bind mount or anonymous volume instead"
		))),
	}
}

/// The project-prefixed name a named-volume reference resolves to when its
/// top-level definition does not override it with `name:` or `external: true`.
///
/// The one place the rule is written down, so a caller that needs to know
/// whether a resolved name was overridden (the config hash) does not have to
/// repeat the `{project}_{reference}` shape and risk drifting from it.
pub(in crate::backends::engine::workload::container) fn default_volume_name(
	reference: &str,
	project: &str,
) -> String {
	format!("{project}_{reference}")
}

/// Resolve a bind-mount source path: expand a leading `~`, then make a relative
/// path absolute against the project base directory. Absolute paths (including
/// staged secret/config files) are returned unchanged.
pub(crate) fn resolve_bind_source(src: &str, base_dir: &Path) -> String {
	if src.is_empty() {
		return src.to_string();
	}
	let expanded = if let Some(rest) = src.strip_prefix("~/") {
		// Join with the platform separator rather than hardcoding `/`, and look
		// up the home directory in a platform-correct way (USERPROFILE on
		// native Windows, where HOME is usually unset).
		match home_dir() {
			Some(home) => home.join(rest).to_string_lossy().into_owned(),
			None => src.to_string(),
		}
	} else if src == "~" {
		home_dir()
			.map(|h| h.to_string_lossy().into_owned())
			.unwrap_or_else(|| src.to_string())
	} else {
		src.to_string()
	};
	if Path::new(&expanded).is_absolute() {
		expanded
	} else {
		base_dir.join(&expanded).to_string_lossy().into_owned()
	}
}

/// The current user's home directory. Prefers `HOME` (set on Unix and most
/// shells), falling back to `USERPROFILE` for native Windows where `HOME` is
/// usually absent. Empty values are treated as unset.
fn home_dir() -> Option<std::path::PathBuf> {
	std::env::var_os("HOME")
		.or_else(|| std::env::var_os("USERPROFILE"))
		.filter(|v| !v.is_empty())
		.map(std::path::PathBuf::from)
}
