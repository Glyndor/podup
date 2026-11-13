//! The filesystems a container mounts: `volumes:` and `tmpfs:`.

use std::path::Path;

use crate::compose::types::Service;

use super::super::{render_tmpfs_mount, render_volume, ExternalNames, Section};

/// Emit a `Volume=` line per `volumes:` entry, or `Tmpfs=` for a long-form
/// `type: tmpfs` mount.
pub(in super::super) fn emit_volumes(
	service: &Service,
	project: &str,
	declared_volumes: &[&str],
	external_volumes: &ExternalNames<'_>,
	base_dir: &Path,
	container: &mut Section,
) {
	for vol in &service.volumes {
		// A long-form `type: tmpfs` mount maps to `Tmpfs=`, not `Volume=`
		// (which would persist it as a volume rather than an in-memory fs).
		if let Some(t) = render_tmpfs_mount(vol) {
			container.add("Tmpfs", t);
		} else {
			container.add(
				"Volume",
				render_volume(vol, project, declared_volumes, external_volumes, base_dir),
			);
		}
	}
}

/// Emit a `Tmpfs=` line per `tmpfs:` entry.
pub(in super::super) fn emit_tmpfs(service: &Service, container: &mut Section) {
	for t in service.tmpfs.to_list() {
		// `uid=`/`gid=` that name the service's `user:` are Podman's `U`; any other
		// was refused by `validate_for_quadlet`, and is written as it came.
		let t = crate::libpod::validate::translate_entry(&t, service.user.as_deref()).unwrap_or(t);
		container.add("Tmpfs", t);
	}
}

/// The host directories the unit has to create before the container starts:
/// the bind sources `up` creates (`create_host_path`, on by default, off only
/// with an explicit `false` on a long-form bind). Without them `podman run`
/// refuses a missing source (`statfs ...: no such file or directory`) and the
/// unit never starts, where `up` would have made the directory.
fn bind_dirs_to_create(service: &Service, base_dir: &Path) -> Vec<String> {
	use crate::compose::types::{VolumeMount, VolumeType};
	let names_a_path = |src: &str| src.starts_with(['.', '/', '~']);
	let mut dirs = Vec::new();
	for vol in &service.volumes {
		let source = match vol {
			VolumeMount::Short(s) => s
				.split_once(':')
				.map(|(src, _)| src)
				.filter(|src| names_a_path(src)),
			VolumeMount::Long {
				volume_type: VolumeType::Bind,
				source,
				bind,
				..
			} => source
				.as_deref()
				.filter(|src| !src.is_empty())
				.filter(|_| bind.as_ref().and_then(|b| b.create_host_path) != Some(false)),
			_ => None,
		};
		if let Some(src) = source {
			let abs = crate::engine::container::resolve_bind_source(src, base_dir);
			if !dirs.contains(&abs) {
				dirs.push(abs);
			}
		}
	}
	dirs
}

/// `ExecStartPre=` creating each directory of [`bind_dirs_to_create`]. Only a
/// path made of plain characters (letters, digits, `/ . _ - + @ ,` and spaces)
/// is written: it goes on a systemd command line, and a path from a compose
/// file is not trusted to be safe there. Any other path is left out and
/// reported, so the operator creates it before the unit starts.
pub(in super::super) fn emit_bind_dirs(
	name: &str,
	service: &Service,
	base_dir: &Path,
	svc: &mut Section,
	warnings: &mut Vec<String>,
) {
	let plain = |c: char| {
		c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | '+' | '@' | ',' | ' ')
	};
	for dir in bind_dirs_to_create(service, base_dir) {
		if dir.chars().all(plain) {
			svc.add("ExecStartPre", format!("mkdir -p -- \"{dir}\""));
		} else {
			warnings.push(format!(
				"{name}: bind source {dir:?} is not created by the unit (it has characters \
				 a systemd command line cannot carry safely); create it before starting it"
			));
		}
	}
}
