use podup::compose::types::{ComposeFile, Service};

use super::{finding, Finding};

/// `read_only` not set to `true`, the container's rootfs is writable.
/// Compose's default is `false`; an absent key and an explicit `false` both
/// keep the filesystem writable, so the check fires unless the service
/// opted into read-only.
pub fn check_writable_root(name: &str, service: &Service, _file: &ComposeFile) -> Vec<Finding> {
	if service.read_only != Some(true) {
		vec![finding(
			name,
			"writable_root",
			"read_only is not true: the container's root filesystem is writable",
		)]
	} else {
		Vec::new()
	}
}
