//! Device, blkio, tmpfs, label-file, and small utility helpers for container creation.

use std::collections::HashMap;
use std::path::Path;

use tracing::warn;

use crate::compose::types::{BlkioConfig, Service};
use crate::libpod::types::container::{LinuxBlockIO, LinuxDevice};

// ---------------------------------------------------------------------------
// Device helpers
// ---------------------------------------------------------------------------

pub(crate) fn parse_device(s: &str) -> LinuxDevice {
	let parts: Vec<&str> = s.splitn(3, ':').collect();
	let host = parts.first().copied().unwrap_or("").to_string();
	let cont = parts
		.get(1)
		.copied()
		.map(|c| c.to_string())
		.unwrap_or_else(|| host.clone());

	let (major, minor, device_type) = device_major_minor(&host);

	LinuxDevice {
		path: cont,
		device_type,
		major,
		minor,
		file_mode: None,
		uid: None,
		gid: None,
	}
}

#[cfg(unix)]
fn device_major_minor(path: &str) -> (i64, i64, String) {
	use std::ffi::CString;
	let Ok(c_path) = CString::new(path) else {
		return (0, 0, "c".to_string());
	};
	let mut st: libc::stat = unsafe { std::mem::zeroed() };
	if unsafe { libc::stat(c_path.as_ptr(), &mut st) } != 0 {
		return (0, 0, "c".to_string());
	}
	let rdev = st.st_rdev;
	let major = (((rdev >> 8) & 0xfff) | ((rdev >> 32) & !0xfff)) as i64;
	let minor = ((rdev & 0xff) | ((rdev >> 12) & !0xff)) as i64;
	let dev_type = if st.st_mode & libc::S_IFMT == libc::S_IFBLK {
		"b"
	} else {
		"c"
	};
	(major, minor, dev_type.to_string())
}

#[cfg(not(unix))]
fn device_major_minor(_path: &str) -> (i64, i64, String) {
	(0, 0, "c".to_string())
}

// ---------------------------------------------------------------------------
// Blkio
// ---------------------------------------------------------------------------

pub(super) fn build_blkio_config(service: &Service) -> Option<LinuxBlockIO> {
	let cfg: &BlkioConfig = service.blkio_config.as_ref()?;

	Some(LinuxBlockIO {
		weight: cfg.weight,
		weight_device: vec![],
		throttle_read_bps_device: vec![],
		throttle_write_bps_device: vec![],
		throttle_read_iops_device: vec![],
		throttle_write_iops_device: vec![],
	})
}

// ---------------------------------------------------------------------------
// Tmpfs / label helpers
// ---------------------------------------------------------------------------

pub(crate) fn tmpfs_options_to_string(
	opts: Option<&crate::compose::types::TmpfsOptions>,
) -> String {
	let opts = match opts {
		Some(o) => o,
		None => return String::new(),
	};
	let mut parts: Vec<String> = Vec::new();
	if let Some(size) = opts.size {
		parts.push(format!("size={size}"));
	}
	if let Some(mode) = opts.mode {
		parts.push(format!("mode={mode:o}"));
	}
	parts.join(",")
}

pub(super) fn build_label_file_labels(
	service: &Service,
	base_dir: &Path,
) -> HashMap<String, String> {
	let mut labels = HashMap::new();
	for path in service.label_file.to_list() {
		let full = if std::path::Path::new(&path).is_absolute() {
			std::path::PathBuf::from(&path)
		} else {
			base_dir.join(&path)
		};
		let Ok(content) = std::fs::read_to_string(&full) else {
			warn!("label_file: cannot read {}", full.display());
			continue;
		};
		for line in content.lines() {
			let trimmed = line.trim();
			if trimmed.is_empty() || trimmed.starts_with('#') {
				continue;
			}
			let mut parts = trimmed.splitn(2, '=');
			let key = parts.next().unwrap_or("").trim().to_string();
			let val = parts.next().unwrap_or("").to_string();
			if !key.is_empty() {
				labels.insert(key, val);
			}
		}
	}
	labels
}

// ---------------------------------------------------------------------------
// Swarm-only deploy field diagnostics
// ---------------------------------------------------------------------------

pub(super) fn warn_swarm_only_deploy(service_name: &str, service: &Service) {
	let Some(deploy) = &service.deploy else {
		return;
	};

	if let Some(mode) = &deploy.mode {
		warn!(
			"service \"{service_name}\": deploy.mode=\"{mode}\" is a Docker Swarm field \
			and has no effect on single-host Podman"
		);
	}
	if deploy.placement.is_some() {
		warn!(
			"service \"{service_name}\": deploy.placement is a Docker Swarm field \
			and has no effect on single-host Podman"
		);
	}
	if deploy.update_config.is_some() {
		warn!(
			"service \"{service_name}\": deploy.update_config is a Docker Swarm field \
			and has no effect on single-host Podman"
		);
	}
	if deploy.rollback_config.is_some() {
		warn!(
			"service \"{service_name}\": deploy.rollback_config is a Docker Swarm field \
			and has no effect on single-host Podman"
		);
	}
	if let Some(mode) = &deploy.endpoint_mode {
		warn!(
			"service \"{service_name}\": deploy.endpoint_mode=\"{mode}\" is a Docker Swarm field \
			and has no effect on single-host Podman"
		);
	}
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

pub(crate) fn opt_vec<T>(v: Vec<T>) -> Option<Vec<T>> {
	if v.is_empty() {
		None
	} else {
		Some(v)
	}
}

pub(crate) fn opt_map<K, V>(m: HashMap<K, V>) -> Option<HashMap<K, V>> {
	if m.is_empty() {
		None
	} else {
		Some(m)
	}
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::*;
	use crate::compose::types::{Service, TmpfsOptions};

	fn default_service() -> Service {
		Service::default()
	}

	// --- device parsing ---

	#[test]
	fn parse_device_host_container_perm() {
		let d = parse_device("/dev/null:/dev/zero:rwm");
		assert_eq!(d.path, "/dev/zero");
	}

	#[test]
	fn parse_device_same_path_both_sides() {
		let d = parse_device("/dev/null");
		assert_eq!(d.path, "/dev/null");
	}

	#[test]
	fn parse_device_two_part() {
		let d = parse_device("/dev/null:/dev/xvda");
		assert_eq!(d.path, "/dev/xvda");
	}

	// --- tmpfs ---

	#[test]
	fn tmpfs_options_empty() {
		assert!(tmpfs_options_to_string(None).is_empty());
	}

	#[test]
	fn tmpfs_options_size_only() {
		let opts = TmpfsOptions {
			size: Some(67108864),
			mode: None,
		};
		assert_eq!(tmpfs_options_to_string(Some(&opts)), "size=67108864");
	}

	#[test]
	fn tmpfs_options_mode_only() {
		let opts = TmpfsOptions {
			size: None,
			mode: Some(0o1755),
		};
		assert_eq!(tmpfs_options_to_string(Some(&opts)), "mode=1755");
	}

	#[test]
	fn tmpfs_options_size_and_mode() {
		let opts = TmpfsOptions {
			size: Some(1024),
			mode: Some(0o755),
		};
		let s = tmpfs_options_to_string(Some(&opts));
		assert!(s.contains("size=1024"));
		assert!(s.contains("mode=755"));
	}

	// --- blkio ---

	#[test]
	fn build_blkio_config_empty_no_blkio() {
		assert!(build_blkio_config(&default_service()).is_none());
	}

	#[test]
	fn build_blkio_config_weight_only() {
		use crate::compose::types::BlkioConfig;
		let mut svc = default_service();
		svc.blkio_config = Some(BlkioConfig {
			weight: Some(500),
			..Default::default()
		});
		let blkio = build_blkio_config(&svc).unwrap();
		assert_eq!(blkio.weight, Some(500));
		assert!(blkio.weight_device.is_empty());
	}

	#[test]
	fn build_blkio_config_with_rate_device_global_weight_only() {
		use crate::compose::types::{BlkioConfig, BlkioRateDevice};
		let mut svc = default_service();
		svc.blkio_config = Some(BlkioConfig {
			device_read_bps: vec![BlkioRateDevice {
				path: "/dev/sda".into(),
				rate: serde_yaml::Value::Number(serde_yaml::Number::from(1048576u64)),
			}],
			..Default::default()
		});
		// per-device throttle not translated yet, but struct is returned
		let blkio = build_blkio_config(&svc).unwrap();
		assert!(blkio.throttle_read_bps_device.is_empty());
	}

	// --- opt_vec / opt_map ---

	#[test]
	fn opt_vec_empty() {
		assert!(opt_vec::<String>(vec![]).is_none());
	}

	#[test]
	fn opt_vec_nonempty() {
		assert!(opt_vec(vec!["x"]).is_some());
	}

	#[test]
	fn opt_map_empty() {
		assert!(opt_map::<String, String>(Default::default()).is_none());
	}

	#[test]
	fn opt_map_nonempty() {
		let mut m = HashMap::new();
		m.insert("k".to_string(), "v".to_string());
		assert!(opt_map(m).is_some());
	}

	// --- warn_swarm_only_deploy ---

	#[test]
	fn warn_swarm_only_deploy_no_deploy_is_noop() {
		let svc = default_service();
		warn_swarm_only_deploy("web", &svc);
	}

	#[test]
	fn warn_swarm_only_deploy_no_swarm_fields_is_noop() {
		use crate::compose::types::DeployConfig;
		let mut svc = default_service();
		svc.deploy = Some(DeployConfig {
			replicas: Some(2),
			..Default::default()
		});
		warn_swarm_only_deploy("web", &svc);
	}

	#[test]
	fn warn_swarm_only_deploy_all_swarm_fields_no_panic() {
		use crate::compose::types::{DeployConfig, DeployPlacement, DeployUpdateConfig};
		let mut svc = default_service();
		svc.deploy = Some(DeployConfig {
			mode: Some("global".to_string()),
			placement: Some(DeployPlacement {
				constraints: vec!["node.role == manager".to_string()],
				..Default::default()
			}),
			update_config: Some(DeployUpdateConfig {
				parallelism: Some(1),
				..Default::default()
			}),
			rollback_config: Some(DeployUpdateConfig::default()),
			endpoint_mode: Some("dnsrr".to_string()),
			..Default::default()
		});
		warn_swarm_only_deploy("web", &svc);
	}
}
