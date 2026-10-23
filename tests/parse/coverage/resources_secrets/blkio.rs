//! The `blkio_config` block: weight, weight_device and the per-device rates.
//!
//! Split out of `tests/parse/coverage/resources_secrets/mod.rs`; the imports
//! are inherited via `use super::*`.

use super::*;

// ---------------------------------------------------------------------------
// blkio_config
// ---------------------------------------------------------------------------

#[test]
fn blkio_weight() {
	let yaml = r#"
services:
  app:
    image: alpine
    blkio_config:
      weight: 300
"#;
	let svc = &parse_str(yaml).unwrap().services["app"];
	let bc = svc.blkio_config.as_ref().unwrap();
	assert_eq!(bc.weight, Some(300));
}

#[test]
fn blkio_weight_device() {
	let yaml = r#"
services:
  app:
    image: alpine
    blkio_config:
      weight_device:
        - path: /dev/sda
          weight: 400
"#;
	let svc = &parse_str(yaml).unwrap().services["app"];
	let bc = svc.blkio_config.as_ref().unwrap();
	assert_eq!(bc.weight_device.len(), 1);
	assert_eq!(bc.weight_device[0].path, "/dev/sda");
	assert_eq!(bc.weight_device[0].weight, 400);
}

#[test]
fn blkio_device_read_bps() {
	let yaml = r#"
services:
  app:
    image: alpine
    blkio_config:
      device_read_bps:
        - path: /dev/sda
          rate: "12mb"
      device_write_bps:
        - path: /dev/sda
          rate: "1024k"
"#;
	let svc = &parse_str(yaml).unwrap().services["app"];
	let bc = svc.blkio_config.as_ref().unwrap();
	assert_eq!(bc.device_read_bps.len(), 1);
	assert_eq!(bc.device_read_bps[0].path, "/dev/sda");
	assert_eq!(bc.device_read_bps[0].rate_value(), 12 * 1024 * 1024);
	assert_eq!(bc.device_write_bps.len(), 1);
	assert_eq!(bc.device_write_bps[0].rate_value(), 1024 * 1024);
}

#[test]
fn blkio_device_read_write_iops() {
	let yaml = r#"
services:
  app:
    image: alpine
    blkio_config:
      device_read_iops:
        - path: /dev/sda
          rate: 100
      device_write_iops:
        - path: /dev/sda
          rate: 200
"#;
	let svc = &parse_str(yaml).unwrap().services["app"];
	let bc = svc.blkio_config.as_ref().unwrap();
	assert_eq!(bc.device_read_iops.len(), 1);
	assert_eq!(bc.device_write_iops.len(), 1);
	assert_eq!(bc.device_read_iops[0].rate_value(), 100);
	assert_eq!(bc.device_write_iops[0].rate_value(), 200);
}
