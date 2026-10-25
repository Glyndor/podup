//! `deploy.resources.reservations.devices`, the GPU reservation.
//!
//! Split out of `tests/compose/parsing/parse/coverage/resources_secrets/mod.rs`; the imports
//! are inherited via `use super::*`.

use super::*;

// ---------------------------------------------------------------------------
// deploy.resources.reservations.devices (GPU reservation)
// ---------------------------------------------------------------------------

#[test]
fn deploy_gpu_device_reservation() {
	let yaml = r#"
services:
  app:
    image: alpine
    deploy:
      resources:
        reservations:
          devices:
            - capabilities: [gpu]
              count: 1
            - capabilities: [gpu]
              count: all
              device_ids: ["GPU-0", "GPU-1"]
"#;
	let svc = &parse_str(yaml).unwrap().services["app"];
	let deploy = svc.deploy.as_ref().unwrap();
	let devs = &deploy
		.resources
		.as_ref()
		.unwrap()
		.reservations
		.as_ref()
		.unwrap()
		.devices;
	assert_eq!(devs.len(), 2);
	assert!(devs[0].capabilities.contains(&"gpu".to_string()));
	assert_eq!(devs[1].device_ids.len(), 2);
}
