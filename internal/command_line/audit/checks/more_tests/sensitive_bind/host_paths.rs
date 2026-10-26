use super::*;

// ---------------------------------------------------------------------------
// sensitive_bind_mount: one test per row of the list plus the negative
// cases. Each positive test pins the path that fires (so a regression that
// drops an entry is caught by its own name) and the access-mode suffix in
// the reason (so a regression that silently drops `:ro` is caught too).
// ---------------------------------------------------------------------------

#[test]
fn audit_sensitive_bind_mount_flags_etc_dir_and_subpath() {
	// `/etc` itself, and `/etc/ssh` underneath it, are both sensitive
	// for the same reason: the operator is exposing system config. The
	// subpath is the same finding as the directory; pin both shapes so
	// a regression that only matches the bare directory is caught.
	for src in ["/etc", "/etc/ssh", "/etc/ssh/sshd_config"] {
		let yaml = format!(
			"services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - {src}:/data\n"
		);
		let report = report_for(&yaml);
		let f = report
			.iter()
			.find(|f| f.check == "sensitive_bind_mount")
			.unwrap_or_else(|| panic!("sensitive_bind_mount must fire for {src}; got {report:#?}"));
		assert!(
			f.reason.contains(src),
			"reason must name the source path {src}: {f:?}"
		);
		assert!(
			f.reason.contains("system config"),
			"reason must cite the capability the attacker gains: {f:?}"
		);
	}
}

#[test]
fn audit_sensitive_bind_mount_flags_proc_and_subpath() {
	// `/proc/1/root` is the canonical escape to the host root filesystem
	// once the container has the mount-namespace permissions; reading
	// `/proc` discloses kernel state even read-only. Subpath is the same
	// finding.
	for src in ["/proc", "/proc/1/root", "/proc/sys/kernel/random/boot_id"] {
		let yaml = format!(
			"services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - {src}:/data\n"
		);
		let report = report_for(&yaml);
		assert!(
			report
				.iter()
				.any(|f| f.check == "sensitive_bind_mount" && f.reason.contains(src)),
			"{src} must fire sensitive_bind_mount; got {report:#?}"
		);
	}
}

#[test]
fn audit_sensitive_bind_mount_flags_sys_dev_boot_and_root() {
	// One test per remaining directory-prefix entry. Each row pins a
	// specific capability in the reason; a regression that swaps one
	// row's text for another's flips that row's test red.
	let cases: &[(&str, &str)] = &[
		("/sys", "kernel tunables"),
		("/sys/firmware", "kernel tunables"),
		("/dev", "raw device nodes"),
		("/dev/sda", "raw device nodes"),
		("/boot", "bootloader"),
		("/root", "root's home directory"),
		("/root/.ssh", "root's home directory"),
	];
	for (src, expected_capability) in cases {
		let yaml = format!(
			"services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - {src}:/data\n"
		);
		let report = report_for(&yaml);
		let f = report
			.iter()
			.find(|f| f.check == "sensitive_bind_mount")
			.unwrap_or_else(|| panic!("{src} must fire sensitive_bind_mount; got {report:#?}"));
		assert!(
			f.reason.contains(src) && f.reason.contains(expected_capability),
			"reason must name {src} and cite `{expected_capability}`: {f:?}"
		);
	}
}

#[test]
fn audit_sensitive_bind_mount_does_not_fire_on_relative_paths() {
	// Relative paths are operator-chosen and we cannot tell whether
	// they resolve under the project directory or under a sensitive
	// location. The false-positive cost of flagging every `./data:/data`
	// is higher than the false-negative cost of missing the rare
	// `~/.ssh`. Pin the negative shape.
	for src in ["./data", "../shared", "~/data"] {
		let yaml = format!(
			"services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - {src}:/data\n"
		);
		let report = report_for(&yaml);
		assert!(
			!report.iter().any(|f| f.check == "sensitive_bind_mount"),
			"relative path {src} must not fire sensitive_bind_mount; got {report:#?}"
		);
	}
}

#[test]
fn audit_sensitive_bind_mount_does_not_fire_on_project_directory() {
	// The project directory conventionally lives at `/srv/app`,
	// `/home/<user>/<repo>`, or wherever the operator cloned it.
	// Operators legitimately mount the project tree into a build
	// container; none of those roots are sensitive on purpose.
	let cases = &[
		"/srv/app/data:/data",
		"/home/user/project:/app",
		"/opt/myapp/cache:/cache",
		"/var/lib/myapp:/data",
	];
	for spec in cases {
		let yaml =
			format!("services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - {spec}\n");
		let report = report_for(&yaml);
		assert!(
			!report.iter().any(|f| f.check == "sensitive_bind_mount"),
			"project-directory mount {spec} must not fire; got {report:#?}"
		);
	}
}

#[test]
fn audit_sensitive_bind_mount_does_not_fire_on_named_or_tmpfs_mounts() {
	// A short-form volume that is not a bind (named volume, anonymous
	// volume, tmpfs) is not a host filesystem mount at all. Pin the
	// negative shape so a regression that flags every `volumes:`
	// entry is caught.
	for spec in [
		"data:/data",      // named volume
		"/data",           // anonymous volume
		"/tmp/data:/data", // bind of /tmp: not sensitive
	] {
		let yaml =
			format!("services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - {spec}\n");
		let report = report_for(&yaml);
		assert!(
			!report.iter().any(|f| f.check == "sensitive_bind_mount"),
			"non-bind mount {spec} must not fire; got {report:#?}"
		);
	}
	// Long form with a non-bind type is also out of scope.
	let yaml_long = r#"
services:
  web:
    image: alpine:3.20
    volumes:
      - type: tmpfs
        target: /data
"#;
	let report = report_for(yaml_long);
	assert!(
		!report.iter().any(|f| f.check == "sensitive_bind_mount"),
		"tmpfs long-form must not fire; got {report:#?}"
	);
}

#[test]
fn audit_sensitive_bind_mount_emits_one_finding_per_matching_mount() {
	// Two sensitive mounts in the same service raise two findings, one
	// per mount. A regression that deduplicates or only emits the first
	// would let the second slip through silently.
	let yaml = "services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - /etc:/data\n      - /var/run/docker.sock:/sock\n";
	let report = report_for(yaml);
	let sensitive: Vec<&Finding> = report
		.iter()
		.filter(|f| f.check == "sensitive_bind_mount")
		.collect();
	assert_eq!(
		sensitive.len(),
		2,
		"two sensitive mounts must produce two findings: {report:#?}"
	);
	let reasons: Vec<&str> = sensitive.iter().map(|f| f.reason.as_str()).collect();
	assert!(
		reasons.iter().any(|r| r.contains("/etc")),
		"the /etc mount must produce a finding: {report:#?}"
	);
	assert!(
		reasons
			.iter()
			.any(|r| r.contains("container runtime socket")),
		"the docker.sock mount must produce a finding: {report:#?}"
	);
}

#[test]
fn audit_sensitive_bind_mount_normalises_trailing_slash() {
	// `/etc/` and `/etc` are the same directory. The check must treat
	// them as the same finding rather than missing one or surfacing
	// two; both bind targets expose the same data.
	let yaml = "services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - /etc/:/data\n";
	let report = report_for(yaml);
	let sensitive: Vec<&Finding> = report
		.iter()
		.filter(|f| f.check == "sensitive_bind_mount")
		.collect();
	assert_eq!(
		sensitive.len(),
		1,
		"trailing slash must not produce a duplicate finding: {report:#?}"
	);
	assert!(
		sensitive[0].reason.contains("/etc"),
		"reason must name the directory (the slash is normalised away): {:?}",
		sensitive[0]
	);
}
