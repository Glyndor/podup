//! Container-spec field builders: device mappings, block-I/O throttling,
//! label-file labels, and Swarm-only deploy-field warnings.

// libc FFI (stat, for device major/minor) is needed here; each block carries a
// soundness comment.
#![allow(unsafe_code)]

use std::collections::HashMap;
use std::path::Path;

use tracing::warn;

use crate::compose::types::{BlkioConfig, Service};
use crate::libpod::types::container::{
	LinuxBlockIO, LinuxDevice, LinuxThrottleDevice, LinuxWeightDevice,
};

// ---------------------------------------------------------------------------
// Device helpers
// ---------------------------------------------------------------------------

/// Parse a compose `devices:` entry (`host:container:permissions`) into a
/// `LinuxDevice`. The container path defaults to the host path when the
/// `:container` segment is absent; major/minor/type are derived by `stat`ing the
/// host node. Trailing permissions, if present, are ignored here.
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

/// Linux device number encoding uses 64-bit `dev_t`; the formula is Linux-kernel specific.
#[cfg(target_os = "linux")]
fn device_major_minor(path: &str) -> (i64, i64, String) {
	use std::ffi::CString;
	let Ok(c_path) = CString::new(path) else {
		return (0, 0, "c".to_string());
	};
	// SAFETY: `libc::stat` is a plain C struct of integers; an all-zero bit
	// pattern is a valid initial value that `libc::stat()` fully overwrites.
	let mut st: libc::stat = unsafe { std::mem::zeroed() };
	// SAFETY: `c_path` is a valid NUL-terminated C string that outlives the
	// call, and `&mut st` points to a live, correctly-sized `stat`. The return
	// value is checked before any field of `st` is read.
	if unsafe { libc::stat(c_path.as_ptr(), &mut st) } != 0 {
		return (0, 0, "c".to_string());
	}
	let rdev = st.st_rdev as u64;
	let major = (((rdev >> 8) & 0xfff) | ((rdev >> 32) & !0xfff)) as i64;
	let minor = ((rdev & 0xff) | ((rdev >> 12) & !0xff)) as i64;
	let dev_type = if st.st_mode & libc::S_IFMT == libc::S_IFBLK {
		"b"
	} else {
		"c"
	};
	(major, minor, dev_type.to_string())
}

/// Non-Linux Unix (macOS): Podman runs via a VM; host device paths don't translate to Linux device numbers.
#[cfg(all(unix, not(target_os = "linux")))]
fn device_major_minor(_path: &str) -> (i64, i64, String) {
	(0, 0, "c".to_string())
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

	let weight_device = cfg
		.weight_device
		.iter()
		.map(|d| {
			let (major, minor, _) = device_major_minor(&d.path);
			LinuxWeightDevice {
				major,
				minor,
				weight: Some(d.weight),
			}
		})
		.collect();

	let throttle = |devs: &[crate::compose::types::BlkioRateDevice]| -> Vec<LinuxThrottleDevice> {
		devs.iter()
			.map(|d| {
				let (major, minor, _) = device_major_minor(&d.path);
				LinuxThrottleDevice {
					major,
					minor,
					rate: d.rate_value() as u64,
				}
			})
			.collect()
	};

	Some(LinuxBlockIO {
		weight: cfg.weight,
		weight_device,
		throttle_read_bps_device: throttle(&cfg.device_read_bps),
		throttle_write_bps_device: throttle(&cfg.device_write_bps),
		throttle_read_iops_device: throttle(&cfg.device_read_iops),
		throttle_write_iops_device: throttle(&cfg.device_write_iops),
	})
}

// ---------------------------------------------------------------------------
// Label helpers
// ---------------------------------------------------------------------------

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
		let content = match crate::filesystem::read_to_string_capped(&full) {
			Ok(c) => c,
			Err(e) => {
				warn!("label_file: cannot read {}: {e}", full.display());
				continue;
			}
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

/// Resolve the user-facing labels for a container.
///
/// Merges `service.labels` with any labels sourced from `label_file`, with
/// `service.labels` taking precedence. Per the Compose Specification,
/// `deploy.labels` are set on the service only and are deliberately NOT applied
/// to containers, matching docker-compose v2 behaviour.
pub(super) fn resolve_container_labels(
	service: &Service,
	label_file_labels: HashMap<String, String>,
) -> HashMap<String, String> {
	let mut labels = service.labels.to_map();
	for (k, v) in label_file_labels {
		labels.entry(k).or_insert(v);
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
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::*;
	use crate::compose::types::Service;

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
	fn build_blkio_config_with_rate_device() {
		use crate::compose::types::{BlkioConfig, BlkioRateDevice};
		let mut svc = default_service();
		svc.blkio_config = Some(BlkioConfig {
			device_read_bps: vec![BlkioRateDevice {
				path: "/dev/sda".into(),
				rate: serde_yaml::Value::Number(serde_yaml::Number::from(1048576u64)),
			}],
			..Default::default()
		});
		let blkio = build_blkio_config(&svc).unwrap();
		assert_eq!(blkio.throttle_read_bps_device.len(), 1);
		assert_eq!(blkio.throttle_read_bps_device[0].rate, 1048576);
		assert!(blkio.throttle_write_bps_device.is_empty());
	}

	#[test]
	fn build_blkio_config_maps_weight_device() {
		use crate::compose::types::{BlkioConfig, BlkioWeightDevice};
		let mut svc = default_service();
		svc.blkio_config = Some(BlkioConfig {
			weight: Some(300),
			weight_device: vec![BlkioWeightDevice {
				// A non-existent path stats to (0, 0); the weight still propagates.
				path: "/dev/does-not-exist".into(),
				weight: 800,
			}],
			..Default::default()
		});
		let blkio = build_blkio_config(&svc).unwrap();
		assert_eq!(blkio.weight, Some(300));
		assert_eq!(blkio.weight_device.len(), 1);
		assert_eq!(blkio.weight_device[0].weight, Some(800));
	}

	#[test]
	fn build_blkio_config_maps_all_four_throttle_kinds() {
		use crate::compose::types::{BlkioConfig, BlkioRateDevice};
		let dev = |rate: u64| BlkioRateDevice {
			path: "/dev/sda".into(),
			rate: serde_yaml::Value::Number(serde_yaml::Number::from(rate)),
		};
		let mut svc = default_service();
		svc.blkio_config = Some(BlkioConfig {
			device_read_bps: vec![dev(1)],
			device_write_bps: vec![dev(2)],
			device_read_iops: vec![dev(3)],
			device_write_iops: vec![dev(4)],
			..Default::default()
		});
		let blkio = build_blkio_config(&svc).unwrap();
		assert_eq!(blkio.throttle_read_bps_device[0].rate, 1);
		assert_eq!(blkio.throttle_write_bps_device[0].rate, 2);
		assert_eq!(blkio.throttle_read_iops_device[0].rate, 3);
		assert_eq!(blkio.throttle_write_iops_device[0].rate, 4);
	}

	// --- build_label_file_labels ---

	#[test]
	fn label_file_parses_keys_skips_comments_and_blanks() {
		use crate::compose::types::primitives::StringOrList;
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("labels.env");
		std::fs::write(
			&path,
			"# a comment\n\ncom.example.team=blue\nbare-key\n  com.example.tier = gold \n",
		)
		.unwrap();

		let mut svc = default_service();
		svc.label_file = StringOrList::Single("labels.env".to_string());
		let labels = build_label_file_labels(&svc, dir.path());

		assert_eq!(
			labels.get("com.example.team").map(String::as_str),
			Some("blue")
		);
		// A bare key with no `=` keeps an empty value.
		assert_eq!(labels.get("bare-key").map(String::as_str), Some(""));
		// The whole line is trimmed first, then the key side is trimmed again; the
		// value keeps its leading space after `=` but loses the line's trailing space.
		assert_eq!(
			labels.get("com.example.tier").map(String::as_str),
			Some(" gold")
		);
		// Comment and blank lines contribute nothing.
		assert_eq!(labels.len(), 3);
	}

	#[test]
	fn label_file_missing_file_is_skipped() {
		use crate::compose::types::primitives::StringOrList;
		let dir = tempfile::tempdir().unwrap();
		let mut svc = default_service();
		svc.label_file = StringOrList::Single("absent.env".to_string());
		// A missing label file warns and yields no labels rather than erroring.
		assert!(build_label_file_labels(&svc, dir.path()).is_empty());
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

	// --- container label resolution ---

	#[test]
	fn resolve_container_labels_keeps_service_labels() {
		use crate::compose::types::primitives::Labels;
		use indexmap::IndexMap;
		let mut svc = default_service();
		let mut map = IndexMap::new();
		map.insert("com.example.team".to_string(), "blue".to_string());
		svc.labels = Labels::Map(map);

		let labels = resolve_container_labels(&svc, HashMap::new());
		assert_eq!(
			labels.get("com.example.team").map(String::as_str),
			Some("blue")
		);
	}

	#[test]
	fn resolve_container_labels_does_not_apply_deploy_labels() {
		use crate::compose::types::primitives::Labels;
		use crate::compose::types::DeployConfig;
		use indexmap::IndexMap;
		let mut svc = default_service();
		let mut svc_map = IndexMap::new();
		svc_map.insert("com.example.service".to_string(), "on".to_string());
		svc.labels = Labels::Map(svc_map);
		let mut deploy_map = IndexMap::new();
		deploy_map.insert("com.example.deploy".to_string(), "swarm".to_string());
		svc.deploy = Some(DeployConfig {
			labels: Labels::Map(deploy_map),
			..Default::default()
		});

		let labels = resolve_container_labels(&svc, HashMap::new());
		// Per the Compose Specification, deploy.labels are NOT applied to the container.
		assert!(!labels.contains_key("com.example.deploy"));
		// Service labels still apply.
		assert_eq!(
			labels.get("com.example.service").map(String::as_str),
			Some("on")
		);
	}

	#[test]
	fn resolve_container_labels_service_overrides_label_file() {
		use crate::compose::types::primitives::Labels;
		use indexmap::IndexMap;
		let mut svc = default_service();
		let mut map = IndexMap::new();
		map.insert("shared".to_string(), "from-service".to_string());
		svc.labels = Labels::Map(map);
		let mut file_labels = HashMap::new();
		file_labels.insert("shared".to_string(), "from-file".to_string());
		file_labels.insert("only-file".to_string(), "yes".to_string());

		let labels = resolve_container_labels(&svc, file_labels);
		assert_eq!(
			labels.get("shared").map(String::as_str),
			Some("from-service")
		);
		assert_eq!(labels.get("only-file").map(String::as_str), Some("yes"));
	}
}
