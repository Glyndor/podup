//! The `bind_outside_project` check: a bind source written relative to the
//! project, or to the home, that lands outside the project directory.

use std::path::{Path, PathBuf};

use podup::compose::types::{VolumeMount, VolumeType};

use super::super::check_sensitive_bind::{
	has_windows_drive_prefix, is_short_bind_source, split_short_volume_spec,
};
use super::super::HostContext;
use super::host_paths::{fold, resolve};
use super::{finding, Finding};

/// A bind whose host path is written relative (`./`, `../`, a bare long-form
/// source) or from the home (`~`), and which resolves outside the project
/// directory. A relative path reads as "something of this project"; `../..`
/// or a `~` path that lands elsewhere hands the container host files the
/// reader of the compose file would not expect. An absolute source is
/// explicit about where it points and is left to `sensitive_bind_mount`.
///
/// Two ways out are reported, one finding per bind:
///
/// - Lexically: the path with `.` and `..` folded is not under the project
///   directory.
/// - Through a symbolic link: the path folds to somewhere inside, but the
///   deepest part of it that exists on this host resolves outside. This is
///   the one check that reads the host filesystem, because a link is only
///   visible there. A path that does not exist yet, or a project directory
///   that cannot be resolved, is judged lexically only.
pub fn check_bind_outside_project(name: &str, host: &HostContext<'_>) -> Vec<Finding> {
	let base = fold(host.base_dir);
	let real_base = std::fs::canonicalize(host.base_dir).ok();
	let mut out = Vec::new();
	for mount in &host.written.volumes {
		let Some(src) = relative_bind_source(mount) else {
			continue;
		};
		let resolved = resolve(src, host.base_dir);
		let folded = fold(&resolved);
		if !folded.starts_with(&base) {
			out.push(finding(
				name,
				"bind_outside_project",
				&format!(
					"volumes: {src} resolves to {}, outside the project directory {}",
					folded.display(),
					base.display()
				),
			));
			continue;
		}
		let Some(real_base) = &real_base else {
			continue;
		};
		if let Some(real) = deepest_existing(&resolved) {
			if !real.starts_with(real_base) {
				out.push(finding(
					name,
					"bind_outside_project",
					&format!(
						"volumes: {src} goes through a symbolic link to {}, outside the \
						 project directory {}",
						real.display(),
						base.display()
					),
				));
			}
		}
	}
	out
}

/// The host source of a bind written relative to the project or to the home,
/// or `None` for an absolute source, a named or anonymous volume, and every
/// other mount type.
fn relative_bind_source(mount: &VolumeMount) -> Option<&str> {
	match mount {
		VolumeMount::Short(spec) => {
			let (src, _dst, _opts) = split_short_volume_spec(spec);
			// A spec with no separator is an anonymous volume, not a bind.
			let is_bind = src.len() < spec.len() && is_short_bind_source(src);
			(is_bind && (src.starts_with('.') || src.starts_with('~'))).then_some(src)
		}
		VolumeMount::Long {
			volume_type: VolumeType::Bind,
			source: Some(src),
			..
		} => {
			// A long-form bind names a host path, so a bare `data` is relative.
			let absolute = src.starts_with(['/', '\\'])
				|| has_windows_drive_prefix(src)
				|| Path::new(src).is_absolute();
			(!src.is_empty() && !absolute).then_some(src.as_str())
		}
		_ => None,
	}
}

/// `path` with every symbolic link resolved, or, when it does not exist, its
/// deepest ancestor that does. `None` when not even the root resolves.
fn deepest_existing(path: &Path) -> Option<PathBuf> {
	let mut candidate = path.to_path_buf();
	loop {
		if let Ok(real) = std::fs::canonicalize(&candidate) {
			return Some(real);
		}
		if !candidate.pop() {
			return None;
		}
	}
}
