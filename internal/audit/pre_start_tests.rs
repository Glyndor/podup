//! The audit walks the init steps of a service, not only the service.
//!
//! An init step is a container of its own: it can ask for `privileged: true`,
//! for a capability from the dangerous list or for a mount of a host path,
//! and none of that is visible if the audit only looks at the services.

use podup::parse_str;

use crate::audit::{audit_file, audited_containers, render_table_to};

/// A step that asks for extended privileges or mounts a host path is a
/// finding about that step, named after the step.
#[test]
fn audit_reports_the_init_steps_of_a_service() {
	let file = parse_str(
		r#"
services:
  web:
    image: nginx:1.27
    pre_start:
      - command: init
      - image: busybox:1.36
        privileged: true
        volumes:
          - /var/run/docker.sock:/var/run/docker.sock
"#,
	)
	.unwrap();
	let report = audit_file(&file, &[]);
	let finding = |service: &str, check: &str| {
		report
			.findings
			.iter()
			.any(|f| f.service == service && f.check == check)
	};
	assert!(
		finding("web/pre_start[2]", "privileged"),
		"a privileged init step must be reported: {:#?}",
		report.findings
	);
	assert!(
		finding("web/pre_start[2]", "sensitive_bind_mount"),
		"an init step mounting the runtime socket must be reported: {:#?}",
		report.findings
	);
	assert!(
		!finding("web/pre_start[1]", "privileged"),
		"a step that declares nothing privileged must not be reported as such: {:#?}",
		report.findings
	);
}

/// The table has one row per container, so the step's findings are visible
/// next to the service's own.
#[test]
fn audit_table_lists_a_row_per_init_step() {
	let file = parse_str(
		"services:\n  web:\n    image: nginx:1.27\n    pre_start:\n      - command: init\n",
	)
	.unwrap();
	let report = audit_file(&file, &[]);
	let mut buf: Vec<u8> = Vec::new();
	render_table_to(&mut buf, &audited_containers(&file), &report).expect("render");
	let out = String::from_utf8_lossy(&buf);
	assert!(
		out.contains("web/pre_start[1]"),
		"the init step has no row of its own:\n{out}"
	);
}

/// A file without init steps walks exactly what it walked before: one
/// container per service, and no other finding.
#[test]
fn audit_without_init_steps_keeps_one_container_per_service() {
	let file = parse_str("services:\n  web:\n    image: nginx:1.27\n").unwrap();
	let containers = audited_containers(&file);
	assert_eq!(containers.len(), 1);
	assert_eq!(containers[0].name, "web");
	let report = audit_file(&file, &[]);
	assert!(
		report.findings.iter().all(|f| f.service == "web"),
		"a file without init steps reports only its services: {:#?}",
		report.findings
	);
}
