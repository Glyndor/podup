//! Volume mount helpers.
//!
//! [`build_mounts_all`] converts all `volumes:` entries, secret bind-strings,
//! and config bind-strings into OCI `Mount` entries and `NamedVolume` entries
//! for the SpecGenerator. Named volumes go in `volumes`; everything else
//! (bind, tmpfs, npipe, cluster) goes in `mounts`.

use std::path::Path;

use crate::compose::types::{BindOptions, Service, VolumeMount, VolumeOptions, VolumeType};
use crate::libpod::types::container::{Mount, NamedVolume};

/// Build all OCI mounts and named volume attachments for a container.
///
/// Returns `(mounts, named_volumes)`. Named volumes must go into
/// `SpecGenerator.volumes`; bind/tmpfs/npipe mounts go into
/// `SpecGenerator.mounts`.
pub(crate) fn build_mounts_all(
	service: &Service,
	base_dir: &Path,
	secret_binds: &[String],
	config_binds: &[String],
) -> (Vec<Mount>, Vec<NamedVolume>) {
	let mut mounts = Vec::new();
	let mut named = Vec::new();

	for v in &service.volumes {
		match v {
			VolumeMount::Short(s) => {
				if let Some((m, n)) = parse_volume_string(s) {
					match n {
						Some(nv) => named.push(nv),
						None => mounts.push(m.unwrap()),
					}
				}
			}
			VolumeMount::Long {
				volume_type,
				source,
				target,
				read_only,
				bind,
				volume,
				tmpfs,
				..
			} => match volume_type {
				VolumeType::Tmpfs => {
					let mut opts: Vec<String> = Vec::new();
					if let Some(t) = tmpfs {
						if let Some(size) = t.size {
							opts.push(format!("size={size}"));
						}
						if let Some(mode) = t.mode {
							opts.push(format!("mode={mode:o}"));
						}
					}
					if read_only.unwrap_or(false) {
						opts.push("ro".into());
					}
					mounts.push(Mount {
						mount_type: "tmpfs".into(),
						source: None,
						destination: target.clone(),
						options: opts,
					});
				}
				VolumeType::Bind => {
					let src = source.as_deref().unwrap_or("");

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

					let mut opts = access_opts(*read_only);
					extend_bind_opts_str(&mut opts, bind.as_ref());
					mounts.push(Mount {
						mount_type: "bind".into(),
						source: Some(src.to_string()),
						destination: target.clone(),
						options: opts,
					});
				}
				VolumeType::Volume => {
					let mut opts = access_opts(*read_only);
					extend_volume_opts_str(&mut opts, volume.as_ref());
					named.push(NamedVolume {
						name: source.clone().unwrap_or_default(),
						dest: target.clone(),
						options: opts,
					});
				}
				VolumeType::Npipe => {
					mounts.push(Mount {
						mount_type: "npipe".into(),
						source: source.clone(),
						destination: target.clone(),
						options: vec![],
					});
				}
				VolumeType::Cluster => {
					mounts.push(Mount {
						mount_type: "cluster".into(),
						source: source.clone(),
						destination: target.clone(),
						options: vec![],
					});
				}
			},
		}
	}

	// Top-level `tmpfs:` shorthand — equivalent to volumes with type=tmpfs.
	for path in service.tmpfs.to_list() {
		mounts.push(Mount {
			mount_type: "tmpfs".into(),
			source: None,
			destination: path,
			options: vec![],
		});
	}

	// Materialised secrets and configs are passed as pre-built bind strings.
	for bind in secret_binds.iter().chain(config_binds.iter()) {
		if let Some(m) = parse_bind_string(bind) {
			mounts.push(m);
		}
	}

	(mounts, named)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse a short-form volume string `"src:dst"` or `"src:dst:opts"`.
///
/// Returns `Some((mount, named))` where exactly one of the two is `Some`.
/// Named volumes go to `SpecGenerator.volumes`; bind mounts go to `mounts`.
fn parse_volume_string(s: &str) -> Option<(Option<Mount>, Option<NamedVolume>)> {
	let parts: Vec<&str> = s.splitn(3, ':').collect();
	let (src, dst, opts_str) = match parts.len() {
		1 => (parts[0], parts[0], ""),
		2 => (parts[0], parts[1], ""),
		_ => (parts[0], parts[1], parts[2]),
	};
	let opts: Vec<String> = opts_str
		.split(',')
		.map(|o| o.trim().to_string())
		.filter(|o| !o.is_empty())
		.collect();
	if src.starts_with('/') || src.starts_with('.') || src.starts_with('~') {
		Some((
			Some(Mount {
				mount_type: "bind".into(),
				source: if src.is_empty() {
					None
				} else {
					Some(src.to_string())
				},
				destination: dst.to_string(),
				options: opts,
			}),
			None,
		))
	} else {
		Some((
			None,
			Some(NamedVolume {
				name: src.to_string(),
				dest: dst.to_string(),
				options: opts,
			}),
		))
	}
}

/// Parse a pre-built bind string (secret/config) — always produces a bind Mount.
fn parse_bind_string(s: &str) -> Option<Mount> {
	let parts: Vec<&str> = s.splitn(3, ':').collect();
	let (src, dst, opts_str) = match parts.len() {
		1 => (parts[0], parts[0], ""),
		2 => (parts[0], parts[1], ""),
		_ => (parts[0], parts[1], parts[2]),
	};
	let opts: Vec<String> = opts_str
		.split(',')
		.map(|o| o.trim().to_string())
		.filter(|o| !o.is_empty())
		.collect();
	Some(Mount {
		mount_type: "bind".into(),
		source: if src.is_empty() {
			None
		} else {
			Some(src.to_string())
		},
		destination: dst.to_string(),
		options: opts,
	})
}

fn access_opts(read_only: Option<bool>) -> Vec<String> {
	if read_only.unwrap_or(false) {
		vec!["ro".into()]
	} else {
		vec!["rw".into()]
	}
}

fn extend_bind_opts_str(opts: &mut Vec<String>, b: Option<&BindOptions>) {
	let Some(b) = b else { return };
	if let Some(p) = &b.propagation {
		opts.push(p.clone());
	}
	if let Some(s) = &b.selinux {
		opts.push(s.clone());
	}
}

fn extend_volume_opts_str(opts: &mut Vec<String>, v: Option<&VolumeOptions>) {
	let Some(v) = v else { return };
	if v.nocopy.unwrap_or(false) {
		opts.push("nocopy".into());
	}
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::build_mounts_all;
	use crate::compose::types::{BindOptions, Service, VolumeMount, VolumeOptions, VolumeType};
	use std::path::Path;

	fn svc_with_volumes(vols: Vec<VolumeMount>) -> Service {
		Service {
			volumes: vols,
			..Default::default()
		}
	}

	#[test]
	fn short_form_bind_passthrough() {
		let svc = svc_with_volumes(vec![VolumeMount::Short("./data:/app/data".into())]);
		let (mounts, named) = build_mounts_all(&svc, Path::new("/base"), &[], &[]);
		assert_eq!(mounts.len(), 1);
		assert!(named.is_empty());
		assert_eq!(mounts[0].mount_type, "bind");
		assert_eq!(mounts[0].destination, "/app/data");
	}

	#[test]
	fn short_form_named_volume() {
		let svc = svc_with_volumes(vec![VolumeMount::Short("myvolume:/data".into())]);
		let (mounts, named) = build_mounts_all(&svc, Path::new("/base"), &[], &[]);
		assert!(mounts.is_empty());
		assert_eq!(named.len(), 1);
		assert_eq!(named[0].name, "myvolume");
		assert_eq!(named[0].dest, "/data");
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
		let (mounts, _) = build_mounts_all(&svc, Path::new("/base"), &[], &[]);
		assert_eq!(mounts.len(), 1);
		assert_eq!(mounts[0].mount_type, "bind");
		assert!(mounts[0].options.contains(&"ro".to_string()));
		assert_eq!(mounts[0].destination, "/container/path");
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
		let (mounts, _) = build_mounts_all(&svc, Path::new("/base"), &[], &[]);
		assert!(mounts[0].options.contains(&"rshared".to_string()));
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
		let (mounts, named) = build_mounts_all(&svc, Path::new("/base"), &[], &[]);
		assert!(mounts.is_empty());
		assert_eq!(named.len(), 1);
		assert_eq!(named[0].name, "myvolume");
		assert!(named[0].options.contains(&"nocopy".to_string()));
	}

	#[test]
	fn tmpfs_type_becomes_tmpfs_mount() {
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
		let (mounts, _) = build_mounts_all(&svc, Path::new("/base"), &[], &[]);
		assert_eq!(mounts.len(), 1);
		assert_eq!(mounts[0].mount_type, "tmpfs");
		assert_eq!(mounts[0].destination, "/tmp/cache");
		assert!(mounts[0].options.iter().any(|o| o.starts_with("size=")));
		assert!(mounts[0].options.iter().any(|o| o.starts_with("mode=")));
	}

	#[test]
	fn secret_binds_appended() {
		let svc = svc_with_volumes(vec![]);
		let secret = "/run/secrets/mydb:/run/secrets/mydb:ro".to_string();
		let (mounts, _) = build_mounts_all(&svc, Path::new("/base"), &[secret], &[]);
		assert_eq!(mounts.len(), 1);
		assert_eq!(mounts[0].destination, "/run/secrets/mydb");
	}

	#[test]
	fn create_host_path_creates_directory() {
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
		build_mounts_all(&svc, dir.path(), &[], &[]);
		assert!(dir.path().join(rel).exists());
	}

	#[test]
	fn top_level_tmpfs_shorthand() {
		use crate::compose::types::StringOrList;
		let svc = Service {
			tmpfs: StringOrList::List(vec!["/tmp".into(), "/run".into()]),
			..Default::default()
		};
		let (mounts, _) = build_mounts_all(&svc, Path::new("/base"), &[], &[]);
		assert_eq!(mounts.len(), 2);
		assert_eq!(mounts[0].mount_type, "tmpfs");
		assert_eq!(mounts[0].destination, "/tmp");
		assert_eq!(mounts[1].destination, "/run");
	}

	#[test]
	fn top_level_tmpfs_single_string() {
		use crate::compose::types::StringOrList;
		let svc = Service {
			tmpfs: StringOrList::Single("/tmp".into()),
			..Default::default()
		};
		let (mounts, _) = build_mounts_all(&svc, Path::new("/base"), &[], &[]);
		assert_eq!(mounts.len(), 1);
		assert_eq!(mounts[0].mount_type, "tmpfs");
		assert_eq!(mounts[0].destination, "/tmp");
	}
}
