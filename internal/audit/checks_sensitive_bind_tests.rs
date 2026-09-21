//! Per-row coverage for the `sensitive_bind_mount` check.

use super::*;
use crate::audit::Finding;

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
fn audit_sensitive_bind_mount_flags_socket_with_stronger_message() {
	// The container runtime socket is qualitatively worse than every
	// other entry on the list: the bind target itself is the attack
	// vector, not a directory of files. Pin both spellings (Docker and
	// Podman, `/var/run` and `/run`) and assert the message uses the
	// stronger wording rather than the directory-prefix wording.
	for src in [
		"/var/run/docker.sock",
		"/run/docker.sock",
		"/var/run/podman/podman.sock",
		"/run/podman/podman.sock",
	] {
		let yaml = format!(
			"services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - {src}:/sock\n"
		);
		let report = report_for(&yaml);
		let f = report
			.iter()
			.find(|f| f.check == "sensitive_bind_mount")
			.unwrap_or_else(|| panic!("{src} must fire sensitive_bind_mount; got {report:#?}"));
		assert!(
			f.reason.contains("container runtime socket"),
			"socket reason must distinguish itself from the directory wording: {f:?}"
		);
		assert!(
			f.reason.contains(src),
			"reason must name the socket path {src}: {f:?}"
		);
		// No sub-directory wording; the exact match wins over the prefix
		// matches even when both would apply (the socket path is a
		// subpath of `/run/podman` / `/run/docker`).
		assert!(
			!f.reason.contains("runtime directory"),
			"the exact-match wording must take priority over the directory-prefix wording: {f:?}"
		);
	}
}

#[test]
fn audit_sensitive_bind_mount_flags_runtime_directory_prefixes() {
	// Mounting the directory rather than the socket file still exposes
	// the socket to the container; a CI recipe that mounts `/run/podman`
	// whole is the more common mistake. Pin each prefix in turn so a
	// regression that drops one is caught by its own test.
	for src in [
		"/run/podman",
		"/run/podman/extra",
		"/var/run/podman",
		"/run/docker",
		"/var/run/docker",
	] {
		let yaml = format!(
			"services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - {src}:/data\n"
		);
		let report = report_for(&yaml);
		let f = report
			.iter()
			.find(|f| f.check == "sensitive_bind_mount")
			.unwrap_or_else(|| panic!("{src} must fire sensitive_bind_mount; got {report:#?}"));
		assert!(
			f.reason.contains(src),
			"reason must name the directory path {src}: {f:?}"
		);
		assert!(
			f.reason.contains("runtime directory"),
			"reason must cite the runtime-directory capability: {f:?}"
		);
	}
}

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

#[test]
fn audit_sensitive_bind_mount_flags_the_rootless_podman_socket() {
	// The rootless socket is the one a rootless operator has and the one
	// podup connects to by default. It sits under the user's runtime
	// directory, so the uid varies and the exact list cannot hold it.
	// Both spellings, and more than one uid, so a match that hard-coded
	// 1000 would fail here.
	for src in [
		"/run/user/1000/podman/podman.sock",
		"/run/user/0/podman/podman.sock",
		"/var/run/user/1001/podman/podman.sock",
	] {
		let yaml = format!(
			"services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - {src}:/sock\n"
		);
		let report = report_for(&yaml);
		let f = report
			.iter()
			.find(|f| f.check == "sensitive_bind_mount")
			.unwrap_or_else(|| panic!("{src} must fire sensitive_bind_mount; got {report:#?}"));
		assert!(
			f.reason.contains("container runtime socket") && f.reason.contains(src),
			"the rootless socket gets the socket wording and names its path: {f:?}"
		);
	}
}

#[test]
fn audit_sensitive_bind_mount_flags_the_rootless_podman_directory() {
	for src in ["/run/user/1000/podman", "/run/user/1000/podman/extra"] {
		let yaml = format!(
			"services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - {src}:/data\n"
		);
		let report = report_for(&yaml);
		let f = report
			.iter()
			.find(|f| f.check == "sensitive_bind_mount")
			.unwrap_or_else(|| panic!("{src} must fire sensitive_bind_mount; got {report:#?}"));
		assert!(
			f.reason.contains("rootless Podman runtime directory"),
			"the directory names the rootless runtime directory: {f:?}"
		);
	}
}

#[test]
fn audit_sensitive_bind_mount_leaves_the_rest_of_the_user_runtime_dir_alone() {
	// The runtime directory also holds the session bus, the audio server
	// and the display socket, which desktop containers mount on purpose.
	// Only the podman subtree is flagged, and only under an all-digit uid.
	for src in [
		"/run/user/1000/pulse/native",
		"/run/user/1000/bus",
		"/run/user/1000/wayland-0",
		"/run/user/1000",
		"/run/user/me/podman/podman.sock",
		"/run/user/1000/podmanx/podman.sock",
	] {
		let yaml = format!(
			"services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - {src}:/data\n"
		);
		let report = report_for(&yaml);
		assert!(
			!report.iter().any(|f| f.check == "sensitive_bind_mount"),
			"{src} must not fire sensitive_bind_mount; got {report:#?}"
		);
	}
}
