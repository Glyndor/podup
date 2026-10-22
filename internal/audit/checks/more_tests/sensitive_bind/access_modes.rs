use super::*;

#[test]
fn audit_sensitive_bind_mount_read_only_still_fires() {
	// A read-only `/etc` discloses hostname, sshd config, and the user
	// list; the check must not silently pass on `:ro`. The message
	// distinguishes the two cases so the operator can tell which
	// exposure they have.
	let yaml = "services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - /etc:/data:ro\n";
	let report = report_for(yaml);
	let f = report
		.iter()
		.find(|f| f.check == "sensitive_bind_mount")
		.expect("`:ro` must still fire");
	assert!(
		f.reason.contains("read-only"),
		"`:ro` reason must name the access mode so the operator can act: {f:?}"
	);
	// Same path, default access (rw, no `:ro` suffix): the reason names
	// the path and the capability but does not prefix the access mode.
	let yaml_rw = "services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - /etc:/data\n";
	let report_rw = report_for(yaml_rw);
	let f_rw = report_rw
		.iter()
		.find(|f| f.check == "sensitive_bind_mount")
		.expect("writable bind must fire");
	assert!(
		!f_rw.reason.contains("read-only"),
		"default access mode is rw; the reason must not say read-only: {f_rw:?}"
	);
	assert!(
		f_rw.reason.contains("/etc"),
		"reason must still name the path: {f_rw:?}"
	);
}

#[test]
fn audit_sensitive_bind_mount_read_only_socket_still_fires() {
	// A read-only bind of the socket file is still the attack vector:
	// the daemon does not honour file mode on its socket, and the
	// container only needs to be able to open it. The `:ro` suffix
	// changes the message wording but not the verdict.
	let yaml = "services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - /var/run/docker.sock:/sock:ro\n";
	let report = report_for(yaml);
	let f = report
		.iter()
		.find(|f| f.check == "sensitive_bind_mount")
		.expect("`:ro` socket bind must still fire");
	assert!(
		f.reason.contains("container runtime socket") && f.reason.contains("read-only"),
		"socket reason with `:ro` must distinguish itself: {f:?}"
	);
}

#[test]
fn audit_sensitive_bind_mount_long_form_bind_fires() {
	// The long form is the same intent as the short form. The check
	// must agree on both shapes; otherwise a regression that only
	// parses the short form would let `type: bind` mounts slip through.
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    volumes:
      - type: bind
        source: /etc
        target: /data
"#;
	let report = report_for(yaml);
	let f = report
		.iter()
		.find(|f| f.check == "sensitive_bind_mount")
		.expect("long-form bind of /etc must fire");
	assert!(
		f.reason.contains("/etc"),
		"reason must name the source path: {f:?}"
	);
	// And the long form honours `read_only: true` the same way the
	// short form honours `:ro`.
	let yaml_ro = r#"
services:
  web:
    image: alpine:3.20
    volumes:
      - type: bind
        source: /etc
        target: /data
        read_only: true
"#;
	let report_ro = report_for(yaml_ro);
	let f_ro = report_ro
		.iter()
		.find(|f| f.check == "sensitive_bind_mount")
		.expect("long-form read_only bind must fire");
	assert!(
		f_ro.reason.contains("read-only"),
		"long-form `read_only: true` must surface in the reason: {f_ro:?}"
	);
}
