use super::*;

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
