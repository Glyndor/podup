use super::super::super::tests::report_for;

// ---------------------------------------------------------------------------
// host_device
// ---------------------------------------------------------------------------

#[test]
fn audit_host_device_flags_every_devices_entry() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    devices:
      - /dev/kvm
      - /dev/fuse:/dev/fuse:rwm
"#;
	let findings: Vec<_> = report_for(yaml)
		.into_iter()
		.filter(|f| f.check == "host_device")
		.collect();
	assert_eq!(findings.len(), 2, "{findings:#?}");
	assert!(findings[0].reason.contains("/dev/kvm"), "{findings:#?}");
}

#[test]
fn audit_host_device_passes_without_devices() {
	// A cgroup rule alone opens no device node of its own; it is the other
	// check's finding, not this one's.
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    devices: []
    device_cgroup_rules: ["c 10:229 rwm"]
"#;
	let findings = report_for(yaml);
	assert!(
		!findings.iter().any(|f| f.check == "host_device"),
		"{findings:#?}"
	);
}

// ---------------------------------------------------------------------------
// device_cgroup_rule
// ---------------------------------------------------------------------------

#[test]
fn audit_device_cgroup_rule_flags_every_rule() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    device_cgroup_rules:
      - "c 1:3 mr"
      - "a *:* rwm"
"#;
	let findings: Vec<_> = report_for(yaml)
		.into_iter()
		.filter(|f| f.check == "device_cgroup_rule")
		.collect();
	assert_eq!(findings.len(), 2, "{findings:#?}");
	assert!(findings[1].reason.contains("a *:* rwm"), "{findings:#?}");
}

#[test]
fn audit_device_cgroup_rule_passes_with_devices_only() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    devices: [/dev/fuse]
"#;
	let findings = report_for(yaml);
	assert!(
		!findings.iter().any(|f| f.check == "device_cgroup_rule"),
		"{findings:#?}"
	);
}
