use super::*;

#[test]
fn warns_on_windows_only_cpu_fields() {
	let msgs = diagnostics_for(
		"services:\n  web:\n    image: nginx\n    cpu_count: 2\n    cpu_percent: 50\n",
	);
	assert!(msgs.iter().any(|m| m.contains("cpu_count")));
	assert!(msgs.iter().any(|m| m.contains("cpu_percent")));
}

#[test]
fn warns_on_env_file_format() {
	let msgs = diagnostics_for(
			"services:\n  web:\n    image: nginx\n    env_file:\n      - path: ./a.env\n        format: raw\n",
		);
	assert!(
		msgs.iter()
			.any(|m| m.contains("env_file format") && m.contains("dotenv")),
		"got: {msgs:?}"
	);
}

#[test]
fn clean_file_produces_no_diagnostics() {
	let msgs = diagnostics_for("services:\n  web:\n    image: nginx\n    cpu_shares: 512\n");
	assert!(msgs.is_empty(), "unexpected diagnostics: {msgs:?}");
}

#[test]
fn attach_is_honored_no_warning() {
	// `attach: false` is honored (it suppresses the service's `up` log streaming,
	// matching Compose), so it must NOT produce an "ignored field" diagnostic.
	let msgs = diagnostics_for("services:\n  web:\n    image: nginx\n    attach: false\n");
	assert!(
		!msgs.iter().any(|m| m.contains("attach")),
		"attach should not be flagged as ignored: {msgs:?}"
	);
}

#[test]
fn warns_on_per_mount_driver_config() {
	let msgs = diagnostics_for(
			"services:\n  web:\n    image: nginx\n    volumes:\n      - type: volume\n        source: data\n        target: /data\n        volume:\n          driver_config:\n            name: local\nvolumes:\n  data:\n",
		);
	assert!(
		msgs.iter().any(|m| m.contains("per-mount driver_config")),
		"got: {msgs:?}"
	);
}

#[test]
fn warns_on_credential_spec_not_honored() {
	let msgs = diagnostics_for(
		"services:\n  web:\n    image: nginx\n    credential_spec:\n      config: my-spec\n",
	);
	assert!(
		msgs.iter()
			.any(|m| m.contains("credential_spec") && m.contains("not honored")),
		"got: {msgs:?}"
	);
	// The recognized key must not also produce a generic "unknown key" warning.
	assert!(
		!msgs
			.iter()
			.any(|m| m.contains("unknown key 'credential_spec'")),
		"got: {msgs:?}"
	);
}

#[test]
fn warns_on_service_isolation_not_honored() {
	let msgs = diagnostics_for("services:\n  web:\n    image: nginx\n    isolation: hyperv\n");
	assert!(
		msgs.iter()
			.any(|m| m.contains("isolation") && m.contains("not honored")),
		"got: {msgs:?}"
	);
	assert!(!msgs.iter().any(|m| m.contains("unknown key 'isolation'")));
}

#[test]
fn warns_on_provider_not_honored() {
	let msgs = diagnostics_for("services:\n  db:\n    provider:\n      type: awesomecloud\n");
	assert!(
		msgs.iter()
			.any(|m| m.contains("provider") && m.contains("not honored")),
		"got: {msgs:?}"
	);
	assert!(!msgs.iter().any(|m| m.contains("unknown key 'provider'")));
}

#[test]
fn warns_on_use_api_socket_not_honored() {
	let msgs = diagnostics_for("services:\n  web:\n    image: nginx\n    use_api_socket: true\n");
	assert!(
		msgs.iter()
			.any(|m| m.contains("use_api_socket") && m.contains("not honored")),
		"got: {msgs:?}"
	);
	assert!(!msgs
		.iter()
		.any(|m| m.contains("unknown key 'use_api_socket'")));
}

#[test]
fn warns_on_restart_policy_delay_and_window() {
	let msgs = diagnostics_for(
			"services:\n  web:\n    image: nginx\n    deploy:\n      restart_policy:\n        condition: on-failure\n        delay: 5s\n        window: 120s\n",
		);
	assert!(
		msgs.iter().any(|m| m.contains("restart_policy.delay")),
		"got: {msgs:?}"
	);
	assert!(
		msgs.iter().any(|m| m.contains("restart_policy.window")),
		"got: {msgs:?}"
	);
}

#[test]
fn warns_on_top_level_models_not_honored() {
	let msgs = diagnostics_for(
		"services:\n  web:\n    image: nginx\nmodels:\n  llm:\n    model: ai/model\n",
	);
	assert!(
		msgs.iter()
			.any(|m| m.contains("model 'llm'") && m.contains("not honored")),
		"got: {msgs:?}"
	);
	// `models` is now a recognized top-level element, not an unknown key.
	assert!(
		!msgs
			.iter()
			.any(|m| m.contains("unknown top-level key 'models'")),
		"got: {msgs:?}"
	);
}
