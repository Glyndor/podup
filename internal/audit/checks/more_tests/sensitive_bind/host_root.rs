use super::*;

#[test]
fn audit_sensitive_bind_mount_flags_host_root_short_form() {
	// A bind of the host root (`-v /:/host`) is a superset of every
	// path on the list: the container sees every file the operator
	// has. Matched as an exact entry so the prefix loop does not fire
	// on every absolute path. `:ro` still fires (the whole host
	// filesystem is a real leak even read-only), and the default
	// (rw) fires with the same wording without the access-mode prefix.
	for spec in ["/:/host", "/:/x:ro", "/:/data:ro", "/:/data"] {
		let yaml =
			format!("services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - {spec}\n");
		let report = report_for(&yaml);
		let f = report
			.iter()
			.find(|f| f.check == "sensitive_bind_mount")
			.unwrap_or_else(|| {
				panic!("sensitive_bind_mount must fire for {spec}; got {report:#?}")
			});
		assert!(
			f.reason.contains("/") && f.reason.contains("whole host filesystem"),
			"reason must name the path and the capability for {spec}: {f:?}"
		);
		// `:ro` must name the access mode; the default must not. The
		// distinction is what lets the operator tell which exposure
		// they have from the message alone.
		let want_ro = spec.contains(":ro");
		assert_eq!(
			f.reason.contains("read-only"),
			want_ro,
			"access-mode prefix must match the spec's `:ro` suffix; spec {spec}: {f:?}"
		);
	}
	// The root finding uses the prefix-list wording shape, not the
	// stronger socket wording: the bind target is a directory the
	// container can walk, not a single file that grants a daemon verb.
	let yaml = "services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - /:/host\n";
	let report = report_for(yaml);
	let f = report
		.iter()
		.find(|f| f.check == "sensitive_bind_mount")
		.expect("root bind must fire");
	assert!(
		!f.reason.contains("container runtime socket"),
		"the host root is not a runtime socket; it must use the directory wording: {f:?}"
	);
}

#[test]
fn audit_sensitive_bind_mount_flags_host_root_long_form() {
	// The long-form bind is the same intent as the short form. The
	// check must agree on both shapes; otherwise a regression that
	// only parses the short form would let `type: bind` mounts slip
	// through, and the gate would pass `-v /:/host` written the long
	// way while flagging the short spelling.
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    volumes:
      - type: bind
        source: /
        target: /host
"#;
	let report = report_for(yaml);
	let f = report
		.iter()
		.find(|f| f.check == "sensitive_bind_mount")
		.expect("long-form bind of / must fire");
	assert!(
		f.reason.contains("/") && f.reason.contains("whole host filesystem"),
		"long-form reason must name the path and the capability: {f:?}"
	);
	// And the long form honours `read_only: true` the same way the
	// short form honours `:ro`.
	let yaml_ro = r#"
services:
  web:
    image: alpine:3.20
    volumes:
      - type: bind
        source: /
        target: /host
        read_only: true
"#;
	let report_ro = report_for(yaml_ro);
	let f_ro = report_ro
		.iter()
		.find(|f| f.check == "sensitive_bind_mount")
		.expect("long-form read_only bind of / must fire");
	assert!(
		f_ro.reason.contains("read-only") && f_ro.reason.contains("whole host filesystem"),
		"long-form `read_only: true` must surface in the reason: {f_ro:?}"
	);
}

#[test]
fn audit_sensitive_bind_mount_host_root_does_not_act_as_prefix() {
	// The root entry is an exact match, not a prefix match. `/tmp`
	// and `/home` stay silent on purpose: the project directory and
	// scratch space live there, and operators legitimately mount
	// them. The root entry must fire only on the root itself, never
	// on a path under it.
	for src in ["/tmp:/t", "/home:/h", "/srv:/s", "/var:/v", "/opt:/o"] {
		let yaml =
			format!("services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - {src}\n");
		let report = report_for(&yaml);
		assert!(
			!report
				.iter()
				.any(|f| f.check == "sensitive_bind_mount"
					&& f.reason.contains("whole host filesystem")),
			"{src} must not fire as a host-root finding; got {report:#?}"
		);
	}
	// And the explicit shape from the issue: `/home:/h` is silent
	// even though the project-directory case in
	// `audit_sensitive_bind_mount_does_not_fire_on_project_directory`
	// covers `/home/<user>/...`. The bare directory form has to
	// stay silent on its own.
	let yaml = "services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - /home:/h\n";
	let report = report_for(yaml);
	assert!(
		!report.iter().any(|f| f.check == "sensitive_bind_mount"),
		"/home:/h must stay silent (project directory lives there); got {report:#?}"
	);
}

#[test]
fn audit_sensitive_bind_mount_normalises_host_root_spellings() {
	// POSIX treats duplicate leading slashes (`//`, `///`) as a single
	// `/`, and `/.` as the current directory of `/`, which is `/`
	// itself. The normaliser folds every spelling the issue calls
	// out to the canonical `/` so the exact-match entry fires once
	// regardless of how the operator wrote the path.
	let fire = [
		"/:/host",
		"//:/host",
		"///:/host",
		"/.:/host",
		"/./:/host",
		"//.:/host",
	];
	for spec in fire {
		let yaml =
			format!("services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - {spec}\n");
		let report = report_for(&yaml);
		assert!(
			report
				.iter()
				.any(|f| f.check == "sensitive_bind_mount"
					&& f.reason.contains("whole host filesystem")),
			"{spec} must fold to / and fire sensitive_bind_mount; got {report:#?}"
		);
	}
	// And the negative shape: a path the normaliser does NOT fold to
	// `/` must stay silent on the host-root wording, even when it
	// looks root-ish. `/x` is a top-level directory the operator
	// mounted; the prefix list does not name it, so the finding
	// should not fire at all.
	for spec in ["/x:/data", "/.x:/data"] {
		let yaml =
			format!("services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - {spec}\n");
		let report = report_for(&yaml);
		assert!(
			!report
				.iter()
				.any(|f| f.check == "sensitive_bind_mount"
					&& f.reason.contains("whole host filesystem")),
			"{spec} must not be reported as the host root; got {report:#?}"
		);
	}
}
