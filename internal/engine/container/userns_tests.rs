//! I assert user namespace fields in the spec builder and engine request.

use super::{build_spec_generator, NamespaceInputs, SecurityInputs, SpecInputs};
use crate::compose::types::Service;
use crate::libpod::types::container::Namespace;

fn create_body(mode: Option<&str>) -> serde_json::Value {
	let service = Service {
		userns_mode: mode.map(str::to_string),
		..Default::default()
	};
	crate::libpod::validate::pre_validate_spec("web", &service, &[]).unwrap();
	let inputs = SpecInputs {
		in_pod: false,
		container_name: "userns-web-1".into(),
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
			userns: mode.map(Namespace::parse),
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
	let spec = build_spec_generator("userns", &service, None, None, None, inputs).unwrap();
	serde_json::to_value(spec).unwrap()
}

#[cfg(unix)]
async fn engine_create_body(mode: Option<&str>, in_pod: bool) -> serde_json::Value {
	use crate::engine::{fake_podman, Engine};
	let fake = fake_podman::start(|method, target| {
		if method == "POST" && target.ends_with("/containers/create") {
			(201, r#"{"Id":"userns","Warnings":[]}"#.into())
		} else if method == "DELETE" {
			(404, r#"{"message":"no such container"}"#.into())
		} else {
			(200, "[]".into())
		}
	});
	let engine = Engine::new(fake.client(), "userns".into());
	let mut file = crate::compose::parse_str(&format!(
		"x-podman-pod: {in_pod}\nservices:\n  web:\n    image: alpine:latest\n"
	))
	.unwrap();
	file.services.get_mut("web").unwrap().userns_mode = mode.map(str::to_string);
	engine
		.create_and_start("userns-web-1", "web", &file.services["web"], &file, false)
		.await
		.unwrap();
	let requests = fake.requests.lock().unwrap();
	let index = requests
		.iter()
		.position(|request| request.ends_with("/containers/create"))
		.expect("container create must be sent");
	let bodies = fake.bodies.lock().unwrap();
	serde_json::from_slice(&bodies[index]).unwrap()
}

fn assert_namespace(body: &serde_json::Value, mode: &str, value: Option<&str>) {
	assert_eq!(body["userns"]["nsmode"], mode);
	assert_eq!(
		body["userns"].get("value"),
		value.map(serde_json::Value::from).as_ref()
	);
	assert_eq!(
		body["userns"].as_object().unwrap().len(),
		if value.is_some() { 2 } else { 1 }
	);
}

fn assert_auto_mappings(body: &serde_json::Value, size: u32) {
	let mappings = &body["idmappings"];
	assert_eq!(mappings["HostUIDMapping"], false);
	assert_eq!(mappings["HostGIDMapping"], false);
	assert_eq!(mappings["AutoUserNs"], true);
	assert_eq!(mappings["AutoUserNsOpts"]["Size"], size);
	assert_eq!(mappings["AutoUserNsOpts"].as_object().unwrap().len(), 1);
	assert_eq!(mappings.as_object().unwrap().len(), 4);
}

#[test]
fn auto_sends_storage_allocation_flags() {
	let body = create_body(Some("auto"));
	assert_namespace(&body, "auto", None);
	assert_auto_mappings(&body, 0);
}

#[test]
fn auto_size_sends_namespace_options_and_storage_size() {
	let body = create_body(Some("auto:size=65536"));
	assert_namespace(&body, "auto", Some("size=65536"));
	assert_auto_mappings(&body, 65536);
}

#[test]
fn keep_id_sends_the_plain_mode() {
	let body = create_body(Some("keep-id"));
	assert_namespace(&body, "keep-id", None);
	assert!(body.get("idmappings").is_none());
}

#[test]
fn keep_id_options_are_a_separate_value() {
	let body = create_body(Some("keep-id:uid=1000,gid=1000"));
	assert_namespace(&body, "keep-id", Some("uid=1000,gid=1000"));
	assert!(body.get("idmappings").is_none());
}

#[test]
fn nomap_uses_the_libpod_mode_name() {
	let body = create_body(Some("nomap"));
	assert_namespace(&body, "no-map", None);
	assert!(body.get("idmappings").is_none());
}

#[test]
fn host_keeps_the_plain_mode() {
	let body = create_body(Some("host"));
	assert_namespace(&body, "host", None);
	assert!(body.get("idmappings").is_none());
}

#[test]
fn private_does_not_invent_explicit_mappings() {
	let body = create_body(Some("private"));
	assert_namespace(&body, "private", None);
	assert!(body.get("idmappings").is_none());
}

#[test]
fn ns_path_uses_the_libpod_path_mode() {
	let body = create_body(Some("ns:/run/user/1000/userns"));
	assert_namespace(&body, "path", Some("/run/user/1000/userns"));
	assert!(body.get("idmappings").is_none());
}

#[test]
fn container_keeps_the_target_id() {
	let body = create_body(Some("container:abc123"));
	assert_namespace(&body, "container", Some("abc123"));
	assert!(body.get("idmappings").is_none());
}

#[test]
fn absent_userns_omits_both_fields() {
	let body = create_body(None);
	assert!(body.get("userns").is_none());
	assert!(body.get("idmappings").is_none());
}

#[test]
fn invalid_auto_options_cannot_silently_use_the_default_size() {
	for mode in [
		"auto:size=-1",
		"auto:size=+1",
		"auto:size=4294967296",
		"auto:size=",
		"auto:size=no",
		"auto:other=1",
		"auto:size=1,other=2",
	] {
		assert!(Namespace::parse(mode).id_mappings().is_err(), "{mode}");
	}
}

#[test]
fn auto_size_accepts_uint32_boundaries_and_last_option_wins() {
	for (mode, expected) in [
		("auto:size=0", 0),
		("auto:size=4294967295", u32::MAX),
		("auto:size=1,size=2048", 2048),
	] {
		let mappings = Namespace::parse(mode).id_mappings().unwrap().unwrap();
		assert_eq!(
			serde_json::to_value(mappings).unwrap()["AutoUserNsOpts"]["Size"],
			expected
		);
	}
}

#[cfg(unix)]
#[tokio::test]
async fn engine_transmits_userns_and_preserves_pod_inheritance() {
	for mode in [
		None,
		Some("auto"),
		Some("auto:size=65536"),
		Some("keep-id"),
		Some("keep-id:uid=1000,gid=1000"),
		Some("nomap"),
		Some("host"),
		Some("private"),
		Some("ns:/path"),
		Some("container:abc123"),
	] {
		let body = engine_create_body(mode, false).await;
		let expected = create_body(mode);
		assert_eq!(body.get("userns"), expected.get("userns"), "{mode:?}");
		assert_eq!(
			body.get("idmappings"),
			expected.get("idmappings"),
			"{mode:?}"
		);
	}
	let member = engine_create_body(Some("auto:size=65536"), true).await;
	assert_eq!(member["pod"], "userns");
	assert!(member.get("userns").is_none());
	assert!(member.get("idmappings").is_none());
}
