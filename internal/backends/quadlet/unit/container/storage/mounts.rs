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
		container.add("Tmpfs", t);
	}
}
