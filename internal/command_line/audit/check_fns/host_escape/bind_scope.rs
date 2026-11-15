//! The `bind_outside_project` check: a bind source written relative to the
//! project, or to the home, that lands outside the project directory.

use std::path::Path;

use podup::compose::types::{VolumeMount, VolumeType};

use super::super::super::check_sensitive_bind::{
	has_windows_drive_prefix, is_short_bind_source, split_short_volume_spec,
};
use super::super::super::HostContext;
use super::host_paths::{canonical_form, fold, resolve};
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
///   deepest part of it that exists on this host resolves outside.
///
/// The host filesystem decides whenever it can answer: the canonical form of
/// the path, or of the deepest part of it that exists, is compared with the
/// canonical form of the project directory. A link that points back inside
/// the project is not an escape, and the lexical form, which cannot see the
/// link, is not asked. Only when neither the path nor the project directory
/// can be resolved is the path judged by its lexical form alone.
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
		let inside = folded.starts_with(&base);
		let (real_base, real) = match (real_base.as_ref(), canonical_form(&resolved)) {
			(Some(real_base), Some(real)) => (real_base, real),
			// The host filesystem cannot answer: the lexical form settles it.
			_ => {
				if !inside {
					out.push(outside_finding(name, src, &folded, &base));
				}
				continue;
			}
		};
		if real.starts_with(real_base) {
			continue;
		}
		// The canonical form is outside. The lexical form says where the
		// escape came from: a link inside the project, or a plain climb.
		if inside {
			out.push(link_finding(name, src, &real, &base));
		} else {
			out.push(outside_finding(name, src, &real, &base));
		}
	}
	out
}

/// The finding for a bind that lands outside the project directory, naming
/// the path it lands on.
fn outside_finding(name: &str, src: &str, path: &Path, base: &Path) -> Finding {
	finding(
		name,
		"bind_outside_project",
		&format!(
			"volumes: {src} resolves to {}, outside the project directory {}",
			path.display(),
			base.display()
		),
	)
}

/// The finding for a bind that folds inside the project but lands outside of
/// it, because a symbolic link on the way moved it.
fn link_finding(name: &str, src: &str, path: &Path, base: &Path) -> Finding {
	finding(
		name,
		"bind_outside_project",
		&format!(
			"volumes: {src} goes through a symbolic link to {}, outside the \
			 project directory {}",
			path.display(),
			base.display()
		),
	)
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
