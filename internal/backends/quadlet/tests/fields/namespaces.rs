//! Namespace flags, donor identities and the dependencies sharing requires.

use crate::parse_str;
use crate::quadlet::tests::unit_named;
use crate::quadlet::{generate_at, validate_for_quadlet};

fn export(yaml: &str) -> crate::quadlet::QuadletOutput {
	generate_at(
		&parse_str(yaml).unwrap(),
		"p",
		std::path::Path::new("/srv/app"),
	)
}

#[test]
fn namespace_flags_are_emitted_inside_and_outside_pods() {
	for pod in [false, true] {
		for ipc in ["private", "shareable", "host"] {
			let out = export(&format!(
				"x-podman-pod: {pod}\nservices:\n  s:\n    image: app\n    ipc: {ipc}\n    pid: host\n    uts: private\n    cgroup: private\n"
			));
			let unit = &unit_named(&out, "p-s.container").contents;
			for (flag, value) in [
				("ipc", ipc),
				("pid", "host"),
				("uts", "private"),
				("cgroupns", "private"),
			] {
				assert!(
					unit.contains(&format!("PodmanArgs=--{flag}=\"{value}\"")),
					"{unit}"
				);
			}
			assert!(!unit.contains("--cgroups="), "{unit}");
		}
	}
}

#[test]
fn service_namespaces_use_exported_container_names_and_required_units() {
	for field in ["ipc", "pid"] {
		for explicit in ["", "    container_name: donor-custom\n"] {
			for optional in ["", "    depends_on:\n      db:\n        condition: service_started\n        required: false\n"] {
				let out = export(&format!(
					"services:\n  db:\n    image: db\n{explicit}  s:\n    image: app\n    {field}: service:db\n{optional}"
				));
				let unit = &unit_named(&out, "p-s.container").contents;
				let donor = if explicit.is_empty() {
					"p-db"
				} else {
					"donor-custom"
				};
				assert!(
					unit.contains(&format!("--{field}=\"container:{donor}\"")),
					"{unit}"
				);
				for key in ["After", "Requires"] {
					assert_eq!(
						unit.lines()
							.filter(|l| *l == format!("{key}=p-db.service"))
							.count(),
						1,
						"{unit}"
					);
				}
				assert!(!unit.contains("Wants=p-db.service"), "{unit}");
			}
		}
	}
}

#[test]
fn multiple_namespace_references_do_not_duplicate_dependencies() {
	let out = export("services:\n  db:\n    image: db\n  s:\n    image: app\n    ipc: service:db\n    pid: service:db\n    depends_on:\n      db:\n        condition: service_started\n        required: false\n");
	let unit = &unit_named(&out, "p-s.container").contents;
	for key in ["After", "Requires"] {
		assert_eq!(
			unit.lines()
				.filter(|l| *l == format!("{key}=p-db.service"))
				.count(),
			1,
			"{unit}"
		);
	}
}

#[test]
fn external_container_namespaces_stay_literal_without_unit_dependencies() {
	let out = export("services:\n  s:\n    image: app\n    ipc: container:external\n    pid: container:external\n    uts: container:external\n");
	let unit = &unit_named(&out, "p-s.container").contents;
	for flag in ["ipc", "pid", "uts"] {
		assert!(
			unit.contains(&format!("--{flag}=\"container:external\"")),
			"{unit}"
		);
	}
	assert!(
		!unit.contains("After=") && !unit.contains("Requires="),
		"{unit}"
	);
}

#[test]
fn missing_namespace_services_are_rejected_before_export() {
	for field in ["ipc", "pid"] {
		let file = parse_str(&format!(
			"services:\n  s:\n    image: app\n    {field}: service:missing\n"
		))
		.unwrap();
		let error = validate_for_quadlet(&file).unwrap_err().to_string();
		assert!(
			error.contains(field) && error.contains("missing"),
			"{error}"
		);
	}
}

#[test]
fn hostname_cannot_override_a_shared_uts_namespace() {
	for mode in ["host", "container:external"] {
		let file = parse_str(&format!(
			"services:\n  s:\n    image: app\n    uts: {mode}\n    hostname: own-name\n"
		))
		.unwrap();
		let error = validate_for_quadlet(&file).unwrap_err().to_string();
		assert!(
			error.contains("uts") && error.contains("hostname"),
			"{error}"
		);
	}
	let file =
		parse_str("services:\n  s:\n    image: app\n    uts: private\n    hostname: own-name\n")
			.unwrap();
	validate_for_quadlet(&file).unwrap();
}

#[test]
fn namespace_warnings_describe_isolation_instead_of_unsupported_fields() {
	for field in ["ipc", "pid", "uts", "cgroup"] {
		for mode in ["host", "container:external", "service:db"] {
			// Only IPC/PID support Compose service references.
			if mode.starts_with("service:") && !["ipc", "pid"].contains(&field) {
				continue;
			}
			let out = export(&format!(
				"services:\n  db:\n    image: db\n  s:\n    image: app\n    {field}: {mode}\n"
			));
			let warnings = out.warnings.join("\n");
			assert!(
				warnings.contains(field) && warnings.contains("isolation"),
				"{warnings}"
			);
			assert!(!warnings.contains("skipped"), "{warnings}");
		}
		let out = export(&format!(
			"services:\n  s:\n    image: app\n    {field}: private\n"
		));
		assert!(out.warnings.is_empty(), "{:?}", out.warnings);
	}
}

#[test]
fn namespace_values_use_the_podman_argument_quoting_path() {
	let out = export("services:\n  s:\n    image: app\n    ipc: 'container:external name%h'\n");
	let unit = &unit_named(&out, "p-s.container").contents;
	assert!(
		unit.contains("PodmanArgs=--ipc=\"container:external name%%h\""),
		"{unit}"
	);
}
