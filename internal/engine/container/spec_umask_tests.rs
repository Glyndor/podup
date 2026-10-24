//! Wire-level initial process mask values from the create-spec builder.

use super::{build_spec_generator, NamespaceInputs, SecurityInputs, SpecInputs};
use crate::compose::types::Service;

fn create_body(umask: Option<&str>) -> serde_json::Value {
	let service = Service {
		umask: umask.map(str::to_owned),
		..Default::default()
	};
	crate::libpod::validate::pre_validate_spec("web", &service).unwrap();
	let inputs = SpecInputs {
		in_pod: false,
		container_name: "umask-web-1".into(),
		image: "alpine:latest".into(),
		portmappings: Vec::new(),
		expose: Default::default(),
		networks: Default::default(),
		netns: None,
		labels: Default::default(),
		annotations: Default::default(),
		sysctl: Default::default(),
		resource_limits: None,
		ulimits: Vec::new(),
		mounts: Vec::new(),
		named_volumes: Vec::new(),
		volumes_from: Vec::new(),
		native_secrets: Vec::new(),
		devices: Vec::new(),
		device_cgroup_rule: Vec::new(),
		security: SecurityInputs {
			selinux_opts: Vec::new(),
			apparmor_profile: None,
			seccomp_profile_path: None,
			no_new_privileges: None,
			mask: Vec::new(),
			unmask: Vec::new(),
		},
		namespaces: NamespaceInputs {
			userns: None,
			pidns: None,
			ipcns: None,
			utsns: None,
			cgroupns: None,
		},
		restart: (None, None),
		stop_signal_timeout: (None, None),
		command: None,
		entrypoint: None,
		env: Default::default(),
		links: Vec::new(),
		image_platform: (None, None),
		storage_opts: Default::default(),
	};
	let spec = build_spec_generator("umask", &service, None, None, None, inputs).unwrap();
	serde_json::to_value(spec).unwrap()
}

#[test]
fn umask_spec_formats_octal() {
	assert_eq!(create_body(Some("22"))["umask"], "0022");
	assert_eq!(create_body(Some("7777"))["umask"], "7777");
}

#[test]
fn umask_spec_sends_explicit_zero() {
	assert_eq!(create_body(Some("0"))["umask"], "0000");
}

#[test]
fn umask_spec_omits_unset() {
	assert!(create_body(None).get("umask").is_none());
}
