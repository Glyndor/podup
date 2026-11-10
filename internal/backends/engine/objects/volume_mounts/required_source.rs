//! Bind sources the file asks podup not to create.
//!
//! A long-form bind with `create_host_path: false` mounts a directory that
//! must already exist. When it does not, the service fails here, naming the
//! service and the path, instead of reaching the daemon and coming back as a
//! raw `statfs` error.

use std::path::Path;

use crate::compose::types::{Service, VolumeMount, VolumeType};
use crate::error::{ComposeError, Result};

/// Fail when a bind mount with `create_host_path: false` points at a host
/// path that does not exist. The path is resolved exactly like the mount
/// source (`~` expanded, relative paths anchored to the project directory);
/// a dangling symlink counts as existing, as it does when the directory is
/// created.
pub(crate) fn ensure_required_bind_sources(
	service_name: &str,
	service: &Service,
	base_dir: &Path,
) -> Result<()> {
	for v in &service.volumes {
		let VolumeMount::Long {
			volume_type: VolumeType::Bind,
			source: Some(src),
			bind: Some(bind),
			..
		} = v
		else {
			continue;
		};
		if bind.create_host_path != Some(false) || src.is_empty() {
			continue;
		}
		let abs = crate::engine::container::resolve_bind_source(src, base_dir);
		// `./x` joined to the project directory reads as `<dir>/./x`; print it clean.
		let shown: std::path::PathBuf = Path::new(&abs).components().collect();
		let shown = shown.display();
		match std::fs::symlink_metadata(&abs) {
			Ok(_) => {}
			Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
				return Err(ComposeError::Invalid(format!(
					"service \"{service_name}\": bind source path does not exist: {shown} \
					 (create_host_path is false)"
				)));
			}
			Err(e) => {
				return Err(ComposeError::Invalid(format!(
					"service \"{service_name}\": cannot inspect bind source path {shown}: {e}"
				)));
			}
		}
	}
	Ok(())
}
