use super::*;

#[test]
fn effective_steps_materialize_files_after_inheritance() {
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(dir.path().join("parent.env"), "API_TOKEN=parent\n").unwrap();
	std::fs::write(dir.path().join("step.env"), "API_TOKEN=step\n").unwrap();
	for kind in ["services", "jobs"] {
		let triggers = if kind == "jobs" {
			"    triggers: {manual: true}\n"
		} else {
			""
		};
		let file = podup::parse_str(&format!(
			"{kind}:\n  web:\n{triggers}    image: alpine\n    env_file: parent.env\n    pre_start:\n      - env_file: step.env\n"
		)).unwrap();
		let containers = prepare_containers(&file, dir.path()).unwrap();
		assert_eq!(
			containers[0].service.environment.to_map()["API_TOKEN"].as_deref(),
			Some("parent")
		);
		assert_eq!(
			containers[1].service.environment.to_map()["API_TOKEN"].as_deref(),
			Some("step")
		);
		assert!(containers[1].service.env_file.to_entries().is_empty());
		let report = audit_containers(&file, &containers, &[]);
		assert!(
			report
				.findings
				.iter()
				.any(|f| f.service == "web/pre_start[1]" && f.check == "secret_in_environment"),
			"{report:#?}"
		);
	}
}

#[test]
fn effective_steps_keep_explicit_environment_over_file_values() {
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(dir.path().join("step.env"), "API_TOKEN=from-file\n").unwrap();
	let file = podup::parse_str(
		"services:\n  web:\n    image: alpine\n    environment:\n      API_TOKEN: explicit\n    pre_start:\n      - env_file: step.env\n"
	).unwrap();
	let containers = prepare_containers(&file, dir.path()).unwrap();
	assert_eq!(
		containers[1].service.environment.to_map()["API_TOKEN"].as_deref(),
		Some("explicit")
	);
}

#[test]
fn effective_steps_propagate_missing_required_env_files() {
	let dir = tempfile::tempdir().unwrap();
	let file = podup::parse_str(
		"services:\n  web:\n    image: alpine\n    pre_start:\n      - env_file: absent.env\n",
	)
	.unwrap();
	assert!(matches!(
		prepare_containers(&file, dir.path()),
		Err(podup::ComposeError::FileNotFound(_))
	));
}

#[cfg(unix)]
#[test]
fn effective_steps_resolve_relative_binds_for_services_and_jobs() {
	for kind in ["services", "jobs"] {
		let triggers = if kind == "jobs" {
			"    triggers: {manual: true}\n"
		} else {
			""
		};
		let file = podup::parse_str(&format!(
			"{kind}:\n  web:\n{triggers}    image: alpine\n    pre_start:\n      - volumes:\n          - ./ssh:/keys:ro\n          - type: bind\n            source: ./ssl\n            target: /certs\n          - type: bind\n            source: ssh\n            target: /plain\n"
		)).unwrap();
		let containers = prepare_containers(&file, std::path::Path::new("/etc")).unwrap();
		let report = audit_containers(&file, &containers, &[]);
		assert_eq!(
			report
				.findings
				.iter()
				.filter(|f| f.service == "web/pre_start[1]" && f.check == "sensitive_bind_mount")
				.count(),
			3,
			"{report:#?}"
		);
	}
}

#[cfg(unix)]
#[test]
fn audit_preserves_ambiguous_bind_paths_before_checks() {
	for kind in ["services", "jobs"] {
		let triggers = if kind == "jobs" {
			"    triggers: {manual: true}\n"
		} else {
			""
		};
		let file = podup::parse_str(&format!(
			"{kind}:\n  web:\n{triggers}    image: alpine\n    volumes:\n      - ./link/../../etc:/host:ro\n      - type: bind\n        source: ./link/../../etc\n        target: /host-long\n    pre_start:\n      - command: init\n"
		)).unwrap();
		let containers = prepare_containers(&file, std::path::Path::new("/srv/app")).unwrap();
		let report = audit_containers(&file, &containers, &[]);
		for name in ["web", "web/pre_start[1]"] {
			let findings: Vec<_> = report
				.findings
				.iter()
				.filter(|f| f.service == name && f.check == "sensitive_bind_mount")
				.collect();
			assert_eq!(findings.len(), 2, "{report:#?}");
			assert!(
				findings.iter().all(|f| f.reason.contains("symbolic link")),
				"{findings:#?}"
			);
		}
	}
}

#[cfg(unix)]
#[test]
fn audit_checks_both_written_and_folded_anchored_bind_paths() {
	let file = podup::parse_str(
		"services:\n  web:\n    image: alpine\n    volumes:\n      - ./../etc:/folded\n      - ./../etc/../srv:/written\n      - ./data:/safe\n      - named:/named\n"
	).unwrap();
	let containers = prepare_containers(&file, std::path::Path::new("/tmp")).unwrap();
	let report = audit_containers(&file, &containers, &[]);
	assert_eq!(
		report
			.findings
			.iter()
			.filter(|f| f.check == "sensitive_bind_mount")
			.count(),
		2,
		"{report:#?}"
	);
}

#[cfg(unix)]
#[test]
fn audit_keeps_sensitive_written_path_even_when_folded_path_is_safe() {
	let file = podup::parse_str(
		"services:\n  web:\n    image: alpine\n    volumes:\n      - ../srv:/short\n      - type: bind\n        source: ../srv\n        target: /long\n"
	).unwrap();
	let containers = prepare_containers(&file, std::path::Path::new("/etc")).unwrap();
	let report = audit_containers(&file, &containers, &[]);
	let findings: Vec<_> = report
		.findings
		.iter()
		.filter(|f| f.check == "sensitive_bind_mount")
		.collect();
	assert_eq!(findings.len(), 2, "{report:#?}");
	assert!(
		findings.iter().all(|f| !f.reason.contains("cannot tell")),
		"{findings:#?}"
	);
}
