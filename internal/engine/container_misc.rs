//! Device, blkio, tmpfs, label-file, and small utility helpers for container creation.

use std::collections::HashMap;
use std::path::Path;

use bollard::models::{DeviceRequest, ResourcesBlkioWeightDevice, ThrottleDevice};
use tracing::warn;

use crate::compose::types::{BlkioConfig, CountOrAll, Service};

// ---------------------------------------------------------------------------
// Device helpers
// ---------------------------------------------------------------------------

pub(crate) fn parse_device(s: &str) -> bollard::models::DeviceMapping {
	let parts: Vec<&str> = s.splitn(3, ':').collect();
	let host = parts.first().copied().unwrap_or("").to_string();
	let cont = parts
		.get(1)
		.copied()
		.map(|c| c.to_string())
		.unwrap_or_else(|| host.clone());
	let perm = parts.get(2).copied().unwrap_or("rwm").to_string();
	bollard::models::DeviceMapping {
		path_on_host: Some(host),
		path_in_container: Some(cont),
		cgroup_permissions: Some(perm),
	}
}

pub(super) fn build_device_requests(service: &Service) -> Vec<DeviceRequest> {
	let mut requests: Vec<DeviceRequest> = Vec::new();

	if let Some(gpus) = &service.gpus {
		requests.push(DeviceRequest {
			driver: Some("".into()),
			count: Some(gpus.to_count()),
			device_ids: None,
			capabilities: Some(vec![vec!["gpu".into()]]),
			options: None,
		});
	}

	if let Some(deploy) = &service.deploy {
		if let Some(resources) = &deploy.resources {
			if let Some(reservations) = &resources.reservations {
				for dev in &reservations.devices {
					if dev.capabilities.is_empty() {
						continue;
					}

					let count = if !dev.device_ids.is_empty() {
						None
					} else {
						Some(
							dev.count
								.as_ref()
								.map(|c: &CountOrAll| c.to_i64())
								.unwrap_or(-1),
						)
					};

					let device_ids = if dev.device_ids.is_empty() {
						None
					} else {
						Some(dev.device_ids.clone())
					};

					requests.push(DeviceRequest {
						driver: dev.driver.clone().or(Some("".into())),
						count,
						device_ids,
						capabilities: Some(vec![dev.capabilities.clone()]),
						options: if dev.options.is_empty() {
							None
						} else {
							Some(dev.options.clone())
						},
					});
				}
			}
		}
	}

	requests
}

// ---------------------------------------------------------------------------
// Blkio
// ---------------------------------------------------------------------------

pub(super) struct BlkioHostConfig {
	pub(super) weight: Option<u16>,
	pub(super) weight_device: Option<Vec<ResourcesBlkioWeightDevice>>,
	pub(super) device_read_bps: Option<Vec<ThrottleDevice>>,
	pub(super) device_write_bps: Option<Vec<ThrottleDevice>>,
	pub(super) device_read_iops: Option<Vec<ThrottleDevice>>,
	pub(super) device_write_iops: Option<Vec<ThrottleDevice>>,
}

pub(super) fn build_blkio_config(service: &Service) -> Option<BlkioHostConfig> {
	let cfg: &BlkioConfig = service.blkio_config.as_ref()?;

	let weight_device = if cfg.weight_device.is_empty() {
		None
	} else {
		Some(
			cfg.weight_device
				.iter()
				.map(|d| ResourcesBlkioWeightDevice {
					path: Some(d.path.clone()),
					weight: Some(d.weight as usize),
				})
				.collect(),
		)
	};

	let to_throttle = |devs: &[crate::compose::types::BlkioRateDevice]| {
		if devs.is_empty() {
			None
		} else {
			Some(
				devs.iter()
					.map(|d| ThrottleDevice {
						path: Some(d.path.clone()),
						rate: Some(d.rate_value()),
					})
					.collect(),
			)
		}
	};

	Some(BlkioHostConfig {
		weight: cfg.weight,
		weight_device,
		device_read_bps: to_throttle(&cfg.device_read_bps),
		device_write_bps: to_throttle(&cfg.device_write_bps),
		device_read_iops: to_throttle(&cfg.device_read_iops),
		device_write_iops: to_throttle(&cfg.device_write_iops),
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
		let d = parse_device("/dev/sda:/dev/xvda:rwm");
		assert_eq!(d.path_on_host.as_deref(), Some("/dev/sda"));
		assert_eq!(d.path_in_container.as_deref(), Some("/dev/xvda"));
		assert_eq!(d.cgroup_permissions.as_deref(), Some("rwm"));
	}

	#[test]
	fn parse_device_default_perm() {
		let d = parse_device("/dev/null:/dev/null");
		assert_eq!(d.cgroup_permissions.as_deref(), Some("rwm"));
	}

	#[test]
	fn parse_device_same_path_both_sides() {
		let d = parse_device("/dev/dri");
		assert_eq!(d.path_on_host.as_deref(), Some("/dev/dri"));
		assert_eq!(d.path_in_container.as_deref(), Some("/dev/dri"));
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
		assert!(blkio.weight_device.is_none());
		assert!(blkio.device_read_bps.is_none());
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
		let devs = blkio.device_read_bps.unwrap();
		assert_eq!(devs.len(), 1);
		assert_eq!(devs[0].path.as_deref(), Some("/dev/sda"));
		assert_eq!(devs[0].rate, Some(1048576));
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
}
