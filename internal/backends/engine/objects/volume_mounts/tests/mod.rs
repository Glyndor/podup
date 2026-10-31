use super::{bind_mounts, build_mounts_all, ensure_bind_source, mount_targets, BindSource};
use crate::compose::types::{BindOptions, Service, VolumeMount, VolumeOptions, VolumeType};
use std::path::Path;

fn svc_with_volumes(vols: Vec<VolumeMount>) -> Service {
	Service {
		volumes: vols,
		..Default::default()
	}
}

mod bind_mounts;
mod create_host_path;
mod ensure_bind_source;
mod hostile_short_form;
mod long_form;
mod short_form;
mod volume_type;
