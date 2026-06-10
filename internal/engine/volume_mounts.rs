//! Volume bind-string and Mount-API helpers.
//!
//! [`build_binds`] converts `volumes:` entries to bollard bind strings.
//! [`build_mounts`] converts entries that require the Mount API (tmpfs,
//! volumes with subpath/labels/driver_config).

use std::path::Path;

use bollard::models::{
	Mount, MountBindOptions, MountTmpfsOptions, MountType, MountVolumeOptions,
	MountVolumeOptionsDriverConfig,
};

use crate::compose::types::{BindOptions, Service, VolumeMount, VolumeOptions, VolumeType};

pub(crate) fn build_binds(service: &Service, base_dir: &Path) -> Vec<String> {
	let mut out = Vec::new();
	for v in &service.volumes {
		match v {
			VolumeMount::Short(s) => out.push(s.clone()),
			VolumeMount::Long {
				volume_type,
				source,
				target,
				read_only,
				bind,
				volume,
				..
			} => {
				if matches!(volume_type, VolumeType::Tmpfs) {
					continue;
				}
				if needs_mount_api(volume) {
					continue;
				}
				let src = source.as_deref().unwrap_or("");

				if matches!(volume_type, VolumeType::Bind) {
					if let Some(b) = bind {
						if b.create_host_path.unwrap_or(false) && !src.is_empty() {
							let abs = if Path::new(src).is_absolute() {
								std::path::PathBuf::from(src)
							} else {
								base_dir.join(src)
							};
							if let Err(e) = std::fs::create_dir_all(&abs) {
								tracing::warn!(
									"create_host_path: failed to create {}: {e}",
									abs.display()
								);
							}
						}
					}
				}

				let mut opts: Vec<String> = Vec::new();
				if read_only.unwrap_or(false) {
					opts.push("ro".into());
				} else {
					opts.push("rw".into());
				}
				if let Some(b) = bind {
					extend_bind_opts(&mut opts, b);
				}
				if let Some(vol) = volume {
					extend_volume_opts(&mut opts, vol);
				}
				out.push(format!("{src}:{target}:{}", opts.join(",")));
			}
		}
	}
	out
}

fn needs_mount_api(volume: &Option<VolumeOptions>) -> bool {
	volume
		.as_ref()
		.is_some_and(|v| v.subpath.is_some() || !v.labels.is_empty() || v.driver_config.is_some())
}

pub(crate) fn build_mounts(service: &Service) -> Vec<Mount> {
	let mut out = Vec::new();
	for v in &service.volumes {
		if let VolumeMount::Long {
			volume_type,
			source,
			target,
			read_only,
			bind,
			volume,
			tmpfs,
			consistency,
		} = v
		{
			if matches!(volume_type, VolumeType::Tmpfs) {
				let tmpfs_options = tmpfs.as_ref().map(|t| MountTmpfsOptions {
					size_bytes: t.size.map(|s| s as i64),
					mode: t.mode.map(|m| m as i64),
					options: None,
				});
				out.push(Mount {
					target: Some(target.clone()),
					source: source.clone(),
					typ: Some(MountType::TMPFS),
					read_only: *read_only,
					consistency: consistency.clone(),
					tmpfs_options,
					..Default::default()
				});
				continue;
			}
			if !needs_mount_api(volume) {
				continue;
			}
			let mount_type = match volume_type {
				VolumeType::Bind => MountType::BIND,
				VolumeType::Volume => MountType::VOLUME,
				VolumeType::Npipe => MountType::NPIPE,
				VolumeType::Cluster => MountType::CLUSTER,
				VolumeType::Tmpfs => unreachable!(),
			};
			let bind_options = bind.as_ref().map(|b| MountBindOptions {
				propagation: b.propagation.as_deref().and_then(|p| p.parse().ok()),
				..Default::default()
			});
			let volume_options = volume.as_ref().map(|v| {
				let labels = if v.labels.is_empty() {
					None
				} else {
					Some(v.labels.to_map())
				};
				let driver_config =
					v.driver_config
						.as_ref()
						.map(|dc| MountVolumeOptionsDriverConfig {
							name: dc.name.clone(),
							options: if dc.options.is_empty() {
								None
							} else {
								Some(dc.options.clone())
							},
						});
				MountVolumeOptions {
					no_copy: v.nocopy,
					labels,
					driver_config,
					subpath: v.subpath.clone(),
				}
			});
			out.push(Mount {
				target: Some(target.clone()),
				source: source.clone(),
				typ: Some(mount_type),
				read_only: *read_only,
				consistency: consistency.clone(),
				bind_options,
				volume_options,
				..Default::default()
			});
		}
	}
	out
}

fn extend_bind_opts(opts: &mut Vec<String>, b: &BindOptions) {
	if let Some(p) = &b.propagation {
		opts.push(p.clone());
	}
	if let Some(s) = &b.selinux {
		opts.push(s.clone());
	}
}

fn extend_volume_opts(opts: &mut Vec<String>, v: &VolumeOptions) {
	if v.nocopy.unwrap_or(false) {
		opts.push("nocopy".into());
	}
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::{build_binds, build_mounts};
	use crate::compose::types::{BindOptions, Service, VolumeMount, VolumeOptions, VolumeType};
	use bollard::models::MountType;
	use std::path::Path;

	fn svc_with_volumes(vols: Vec<VolumeMount>) -> Service {
		Service {
			volumes: vols,
			..Default::default()
		}
	}

	#[test]
	fn short_form_passthrough() {
		let svc = svc_with_volumes(vec![VolumeMount::Short("./data:/app/data".into())]);
		let binds = build_binds(&svc, Path::new("/base"));
		assert_eq!(binds, vec!["./data:/app/data"]);
	}

	#[test]
	fn long_form_bind_read_only() {
		let svc = svc_with_volumes(vec![VolumeMount::Long {
			volume_type: VolumeType::Bind,
			source: Some("/host/path".into()),
			target: "/container/path".into(),
			read_only: Some(true),
			bind: None,
			volume: None,
			tmpfs: None,
			consistency: None,
		}]);
		let binds = build_binds(&svc, Path::new("/base"));
		assert_eq!(binds.len(), 1);
		assert!(binds[0].contains("ro"));
		assert!(binds[0].contains("/host/path:/container/path"));
	}

	#[test]
	fn long_form_bind_with_propagation() {
		let svc = svc_with_volumes(vec![VolumeMount::Long {
			volume_type: VolumeType::Bind,
			source: Some("/host".into()),
			target: "/cont".into(),
			read_only: Some(false),
			bind: Some(BindOptions {
				propagation: Some("rshared".into()),
				create_host_path: None,
				selinux: None,
			}),
			volume: None,
			tmpfs: None,
			consistency: None,
		}]);
		let binds = build_binds(&svc, Path::new("/base"));
		assert!(binds[0].contains("rshared"));
	}

	#[test]
	fn long_form_volume_nocopy() {
		let svc = svc_with_volumes(vec![VolumeMount::Long {
			volume_type: VolumeType::Volume,
			source: Some("myvolume".into()),
			target: "/data".into(),
			read_only: None,
			bind: None,
			volume: Some(VolumeOptions {
				nocopy: Some(true),
				..Default::default()
			}),
			tmpfs: None,
			consistency: None,
		}]);
		let binds = build_binds(&svc, Path::new("/base"));
		assert!(binds[0].contains("nocopy"));
	}

	#[test]
	fn tmpfs_type_excluded_from_binds() {
		let svc = svc_with_volumes(vec![VolumeMount::Long {
			volume_type: VolumeType::Tmpfs,
			source: None,
			target: "/run".into(),
			read_only: None,
			bind: None,
			volume: None,
			tmpfs: None,
			consistency: None,
		}]);
		let binds = build_binds(&svc, Path::new("/base"));
		assert!(binds.is_empty());
	}

	#[test]
	fn build_mounts_tmpfs_long_form() {
		use crate::compose::types::TmpfsOptions;
		let svc = svc_with_volumes(vec![VolumeMount::Long {
			volume_type: VolumeType::Tmpfs,
			source: None,
			target: "/tmp/cache".into(),
			read_only: None,
			bind: None,
			volume: None,
			tmpfs: Some(TmpfsOptions {
				size: Some(65536),
				mode: Some(0o700),
			}),
			consistency: None,
		}]);
		let mounts = build_mounts(&svc);
		assert_eq!(mounts.len(), 1);
		let m = &mounts[0];
		assert_eq!(m.target.as_deref(), Some("/tmp/cache"));
		let opts = m.tmpfs_options.as_ref().unwrap();
		assert_eq!(opts.size_bytes, Some(65536));
		assert_eq!(opts.mode, Some(0o700));
	}

	#[test]
	fn build_mounts_non_tmpfs_skipped_without_mount_api() {
		let svc = svc_with_volumes(vec![VolumeMount::Long {
			volume_type: VolumeType::Bind,
			source: Some("/host".into()),
			target: "/cont".into(),
			read_only: None,
			bind: None,
			volume: None,
			tmpfs: None,
			consistency: None,
		}]);
		let mounts = build_mounts(&svc);
		assert!(
			mounts.is_empty(),
			"plain bind goes through bind string, not Mount API"
		);
	}

	#[test]
	fn build_binds_volume_with_labels_excluded_from_binds() {
		use crate::compose::types::Labels;
		use indexmap::IndexMap;
		let mut map = IndexMap::new();
		map.insert("com.example.key".to_string(), "val".to_string());
		let svc = svc_with_volumes(vec![VolumeMount::Long {
			volume_type: VolumeType::Volume,
			source: Some("myvol".into()),
			target: "/data".into(),
			read_only: None,
			bind: None,
			volume: Some(VolumeOptions {
				labels: Labels::Map(map),
				..Default::default()
			}),
			tmpfs: None,
			consistency: None,
		}]);
		let binds = build_binds(&svc, Path::new("/base"));
		assert!(
			binds.is_empty(),
			"volume with labels must use Mount API, not bind string"
		);
	}

	#[test]
	fn build_binds_create_host_path_creates_directory() {
		let dir = tempfile::tempdir().unwrap();
		let rel = "subdir/nested";
		let svc = svc_with_volumes(vec![VolumeMount::Long {
			volume_type: VolumeType::Bind,
			source: Some(rel.into()),
			target: "/cont".into(),
			read_only: None,
			bind: Some(BindOptions {
				propagation: None,
				create_host_path: Some(true),
				selinux: None,
			}),
			volume: None,
			tmpfs: None,
			consistency: None,
		}]);
		build_binds(&svc, dir.path());
		assert!(
			dir.path().join(rel).exists(),
			"create_host_path should have created the directory"
		);
	}

	#[test]
	fn build_mounts_volume_with_labels_uses_mount_api() {
		use crate::compose::types::Labels;
		use indexmap::IndexMap;
		let mut map = IndexMap::new();
		map.insert("k".to_string(), "v".to_string());
		let svc = svc_with_volumes(vec![VolumeMount::Long {
			volume_type: VolumeType::Volume,
			source: Some("myvol".into()),
			target: "/data".into(),
			read_only: Some(false),
			bind: None,
			volume: Some(VolumeOptions {
				labels: Labels::Map(map),
				..Default::default()
			}),
			tmpfs: None,
			consistency: None,
		}]);
		let mounts = build_mounts(&svc);
		assert_eq!(mounts.len(), 1);
		assert_eq!(mounts[0].typ, Some(MountType::VOLUME));
		let vopts = mounts[0].volume_options.as_ref().unwrap();
		assert!(vopts.labels.is_some());
	}

	#[test]
	fn build_mounts_volume_with_driver_config() {
		use crate::compose::types::DriverConfig;
		let svc = svc_with_volumes(vec![VolumeMount::Long {
			volume_type: VolumeType::Volume,
			source: Some("myvol".into()),
			target: "/data".into(),
			read_only: None,
			bind: None,
			volume: Some(VolumeOptions {
				driver_config: Some(DriverConfig {
					name: Some("local".into()),
					options: Default::default(),
				}),
				..Default::default()
			}),
			tmpfs: None,
			consistency: None,
		}]);
		let mounts = build_mounts(&svc);
		assert_eq!(mounts.len(), 1);
		let vopts = mounts[0].volume_options.as_ref().unwrap();
		let dc = vopts.driver_config.as_ref().unwrap();
		assert_eq!(dc.name.as_deref(), Some("local"));
	}
}
