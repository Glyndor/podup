use super::super::tests::report_for;

// ---------------------------------------------------------------------------
// host_namespace
// ---------------------------------------------------------------------------

#[test]
fn audit_host_namespace_flags_network_mode_host() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    network_mode: host
"#;
	let findings = report_for(yaml);
	let kinds: Vec<&'static str> = findings.iter().map(|f| f.check).collect();
	assert!(kinds.contains(&"host_namespace"), "got {kinds:?}");
}

#[test]
fn audit_host_namespace_flags_pid_ipc_uts_cgroup_userns_host() {
	// One service per bad field, so each test pins a specific field rather
	// than asserting the whole shape at once.
	for (yaml_field, expected) in [
		("pid: host", "pid"),
		("ipc: host", "ipc"),
		("uts: host", "uts"),
		("cgroup: host", "cgroup"),
		("userns_mode: host", "userns_mode"),
	] {
		let yaml = format!("services:\n  web:\n    image: alpine:3.20\n    {yaml_field}\n");
		let findings = report_for(&yaml);
		assert!(
			findings
				.iter()
				.any(|f| f.check == "host_namespace" && f.reason.contains(expected)),
			"expected host_namespace finding mentioning `{expected}`; got {findings:#?}"
		);
	}
}

/// `container:<id>` is the share-another-container mode the runtime
/// detector in `internal/engine/container/host_mode.rs` already warns
/// on; the audit detector must agree, otherwise `podup audit --strict`
/// would pass a file the engine later refuses silently (#1746 entry 4).
/// The two lists are now one: every mode the runtime flags, the audit
/// flags, so a finding on the same file is consistent across the two
/// commands.
#[test]
fn audit_host_namespace_flags_container_id_on_every_namespace_field() {
	for (yaml_field, expected_field) in [
		("pid: container:sidecar", "pid"),
		("ipc: container:sidecar", "ipc"),
		("uts: container:sidecar", "uts"),
		("cgroup: container:sidecar", "cgroup"),
		("userns_mode: container:sidecar", "userns_mode"),
	] {
		let yaml = format!("services:\n  web:\n    image: alpine:3.20\n    {yaml_field}\n");
		let findings = report_for(&yaml);
		assert!(
			findings.iter().any(|f| f.check == "host_namespace"
				&& f.reason.contains(expected_field)
				&& f.reason.contains("container:sidecar")),
			"expected host_namespace finding for `{yaml_field}` mentioning the \
			 target id; got {findings:#?}"
		);
	}
}

/// `network_mode: container:<id>` is its own branch (the field is a
/// sibling of the per-namespace keys, not one of them). Cover it
/// separately so a regression that fixes the `pid`/`ipc`/... loop but
/// leaves `network_mode` behind gets caught.
#[test]
fn audit_host_namespace_flags_network_mode_container_id() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    network_mode: container:sidecar
"#;
	let findings = report_for(yaml);
	assert!(
		findings.iter().any(|f| f.check == "host_namespace"
			&& f.reason.contains("network_mode")
			&& f.reason.contains("container:sidecar")),
		"expected network_mode container:<id> finding; got {findings:#?}"
	);
}

#[test]
fn audit_host_namespace_passes_when_clean() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
"#;
	let findings = report_for(yaml);
	assert!(
		!findings.iter().any(|f| f.check == "host_namespace"),
		"baseline must not flag host_namespace: {findings:#?}"
	);
	// And a service that names a non-host namespace value is also clean.
	let yaml_service = r#"
services:
  web:
    image: alpine:3.20
    pid: service:sidecar
"#;
	let findings = report_for(yaml_service);
	assert!(
		!findings.iter().any(|f| f.check == "host_namespace"),
		"service:<name> share must not flag host_namespace: {findings:#?}"
	);
}

// ---------------------------------------------------------------------------
// no_userns
// ---------------------------------------------------------------------------

#[test]
fn audit_no_userns_flags_when_unset() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
"#;
	let findings = report_for(yaml);
	assert!(
		findings.iter().any(|f| f.check == "no_userns"),
		"unset userns_mode must fire: {findings:#?}"
	);
}

#[test]
fn audit_no_userns_passes_when_set() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    userns_mode: keep-id
"#;
	let findings = report_for(yaml);
	assert!(
		!findings.iter().any(|f| f.check == "no_userns"),
		"userns_mode: keep-id must pass: {findings:#?}"
	);
}
