use super::*;

// ---------------------------------------------------------------------------
// sensitive_bind_mount: one integration test per row of the list.
//
// The unit suite already pins the verdict for each path in isolation;
// this section drives the compiled binary so the JSON shape and exit
// code also cover the check end to end. One test per row of the
// prefix list, one per exact socket match, and three shapes asserted
// rather than implied: a non-sensitive mount does not fire, `:ro` still
// fires, and a path the operator chose on purpose (the project
// directory) does not fire.
// ---------------------------------------------------------------------------

/// Run `audit` against `body` and return the JSON output as a parsed
/// value. Forces `--strict` so a missing finding flips the exit code
/// to 1, the property CI consumers depend on.
fn audit_json_for(body: &str) -> (serde_json::Value, std::process::ExitStatus) {
	let path = write_compose(body);
	let p = path.to_str().unwrap();
	let out = run(&["-f", p, "audit", "--strict", "--format", "json"]);
	assert!(
		out.status.success() || out.status.code() == Some(1),
		"`audit --strict` exits 0 or 1; got {:?}\nstderr: {}\nstdout: {}",
		out.status.code(),
		String::from_utf8_lossy(&out.stderr),
		String::from_utf8_lossy(&out.stdout),
	);
	let stdout = String::from_utf8_lossy(&out.stdout);
	let v: serde_json::Value = serde_json::from_str(&stdout).expect("audit --format json parses");
	(v, out.status)
}

/// Whether `findings` contains an entry for `service` whose `check` id
/// equals `sensitive_bind_mount` and whose `reason` contains `needle`.
fn sensitive_match(findings: &[serde_json::Value], service: &str, needle: &str) -> bool {
	findings.iter().any(|f| {
		f.get("service").and_then(|s| s.as_str()) == Some(service)
			&& f.get("check").and_then(|c| c.as_str()) == Some("sensitive_bind_mount")
			&& f.get("reason")
				.and_then(|r| r.as_str())
				.is_some_and(|r| r.contains(needle))
	})
}

/// One row per directory-prefix entry of the sensitive list. Each test
/// pins a single path so a regression that drops an entry is caught by
/// its own name. The negative sibling sits in `audit_sensitive_bind_mount_does_not_fire_on_project_directory`.
#[test]
fn audit_sensitive_bind_mount_flags_every_directory_prefix() {
	for (service, src) in [
		("proc_svc", "/proc"),
		("sys_svc", "/sys"),
		("dev_svc", "/dev"),
		("etc_svc", "/etc"),
		("boot_svc", "/boot"),
		("root_svc", "/root"),
		("etc_subpath_svc", "/etc/ssh"),
		("proc_subpath_svc", "/proc/1/root"),
		("root_subpath_svc", "/root/.ssh"),
	] {
		let body = format!(
			"services:\n  {service}:\n    image: alpine:3.20\n    volumes:\n      - {src}:/data\n"
		);
		let (v, status) = audit_json_for(&body);
		assert_eq!(
			status.code(),
			Some(1),
			"sensitive bind mount of {src} must fail --strict; got {:?}\nstdout: {}",
			status.code(),
			v
		);
		let arr = v
			.get("findings")
			.and_then(|f| f.as_array())
			.expect("`findings` array");
		assert!(
			sensitive_match(arr, service, src),
			"expected sensitive_bind_mount for `{service}` mentioning `{src}`; got {arr:?}"
		);
	}
}

/// The four exact-socket rows. The reason uses stronger wording than
/// the directory-prefix matches, so the test pins that wording rather
/// than just the check id.
#[test]
fn audit_sensitive_bind_mount_flags_every_socket_exact_path() {
	for (service, src) in [
		("docker_var_run_svc", "/var/run/docker.sock"),
		("docker_run_svc", "/run/docker.sock"),
		("podman_var_run_svc", "/var/run/podman/podman.sock"),
		("podman_run_svc", "/run/podman/podman.sock"),
	] {
		let body = format!(
			"services:\n  {service}:\n    image: alpine:3.20\n    volumes:\n      - {src}:/sock\n"
		);
		let (v, status) = audit_json_for(&body);
		assert_eq!(
			status.code(),
			Some(1),
			"socket mount of {src} must fail --strict; got {:?}\nstdout: {}",
			status.code(),
			v
		);
		let arr = v
			.get("findings")
			.and_then(|f| f.as_array())
			.expect("`findings` array");
		assert!(
			sensitive_match(arr, service, src)
				&& sensitive_match(arr, service, "container runtime socket"),
			"expected sensitive_bind_mount for `{service}` mentioning both `{src}` and \
			 `container runtime socket`; got {arr:?}"
		);
	}
}

/// Mounting the runtime directory (not the socket file itself) still
/// exposes the socket to the container. The directory-prefix wording
/// fires here, not the exact-socket wording.
#[test]
fn audit_sensitive_bind_mount_flags_runtime_directory_prefixes() {
	for (service, src) in [
		("run_podman_svc", "/run/podman"),
		("run_podman_extra_svc", "/run/podman/extra"),
		("var_run_podman_svc", "/var/run/podman"),
		("run_docker_svc", "/run/docker"),
		("var_run_docker_svc", "/var/run/docker"),
	] {
		let body = format!(
			"services:\n  {service}:\n    image: alpine:3.20\n    volumes:\n      - {src}:/data\n"
		);
		let (v, _status) = audit_json_for(&body);
		let arr = v
			.get("findings")
			.and_then(|f| f.as_array())
			.expect("`findings` array");
		assert!(
			sensitive_match(arr, service, src)
				&& sensitive_match(arr, service, "runtime directory"),
			"expected sensitive_bind_mount for `{service}` mentioning both `{src}` and \
			 `runtime directory`; got {arr:?}"
		);
	}
}

/// A read-only bind of a sensitive path is still a finding: a `/etc`
/// mount that is `:ro`
/// discloses hostname, sshd config, and the user list. The CI gate
/// must not silently pass on `:ro`.
#[test]
fn audit_sensitive_bind_mount_read_only_still_fires() {
	let body = "services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - /etc:/data:ro\n";
	let (v, status) = audit_json_for(body);
	assert_eq!(
		status.code(),
		Some(1),
		"`:ro` of /etc must fail --strict; got {:?}\nstdout: {}",
		status.code(),
		v
	);
	let arr = v
		.get("findings")
		.and_then(|f| f.as_array())
		.expect("`findings` array");
	assert!(
		sensitive_match(arr, "web", "/etc"),
		"expected sensitive_bind_mount mentioning /etc; got {arr:?}"
	);
	assert!(
		sensitive_match(arr, "web", "read-only"),
		"`:ro` reason must name the access mode; got {arr:?}"
	);
	// And the same path without `:ro` must still fire and the reason
	// must not say read-only: a regression that hard-codes the
	// access-mode wording is caught.
	let body_rw = "services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - /etc:/data\n";
	let (v_rw, status_rw) = audit_json_for(body_rw);
	assert_eq!(
		status_rw.code(),
		Some(1),
		"writable bind of /etc must fail --strict; got {:?}\nstdout: {}",
		status_rw.code(),
		v_rw
	);
	let arr_rw = v_rw
		.get("findings")
		.and_then(|f| f.as_array())
		.expect("`findings` array");
	assert!(
		sensitive_match(arr_rw, "web", "/etc") && !sensitive_match(arr_rw, "web", "read-only"),
		"writable bind must name /etc and not say read-only; got {arr_rw:?}"
	);
}

/// Paths operators legitimately mount must stay silent on purpose.
/// The project directory is conventionally `/srv/app`,
/// `/home/<user>/<repo>`, or wherever the operator cloned it. None of
/// those roots are sensitive, and a check that flags every
/// `/srv/app/data` mount teaches operators to ignore the audit.
#[test]
fn audit_sensitive_bind_mount_does_not_fire_on_project_directory() {
	// The service carries every hardening flag so the only remaining
	// exit-code 1 would be the sensitive_bind_mount finding under
	// test. Without this, a regression that flips the verdict for
	// `/srv/app/data` would be masked by the other checks that fire
	// on an unhardened service.
	let header = "image: alpine:3.20\n    read_only: true\n    cap_drop: [ALL]\n    \
	              security_opt: [no-new-privileges:true]\n    pids_limit: 200\n    \
	              mem_limit: 512m\n    userns_mode: auto\n    volumes:\n      - ";
	for spec in [
		"/srv/app/data:/data",
		"/home/user/project:/app",
		"/opt/myapp/cache:/cache",
		"/var/lib/myapp:/data",
		"./data:/data",
	] {
		let body = format!("services:\n  web:\n    {header}{spec}\n");
		let (v, status) = audit_json_for(&body);
		assert!(
			status.success(),
			"project-directory mount {spec} must NOT fail --strict; got {:?}\nstdout: {}",
			status.code(),
			v
		);
		let arr = v
			.get("findings")
			.and_then(|f| f.as_array())
			.expect("`findings` array");
		assert!(
			!sensitive_match(arr, "web", "/srv")
				&& !sensitive_match(arr, "web", "/home")
				&& !sensitive_match(arr, "web", "/opt")
				&& !sensitive_match(arr, "web", "/var/lib")
				&& !arr.iter().any(
					|f| f.get("check").and_then(|c| c.as_str()) == Some("sensitive_bind_mount")
				),
			"project-directory mount {spec} must not fire sensitive_bind_mount; got {arr:?}"
		);
	}
}

/// Non-bind mounts (named volumes, anonymous volumes, tmpfs) are not
/// host filesystem mounts at all; the check has nothing to do with
/// them. A regression that flags every `volumes:` entry is caught.
#[test]
fn audit_sensitive_bind_mount_does_not_fire_on_named_or_tmpfs_mounts() {
	for spec in [
		"data:/data",      // named volume
		"/data",           // anonymous volume
		"/tmp/data:/data", // bind of /tmp: not sensitive
	] {
		let body =
			format!("services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - {spec}\n");
		let (v, status) = audit_json_for(&body);
		// The other checks may fire (writable_root, no_cap_drop_all, ...);
		// the property under test is that this one does not.
		assert!(
			!sensitive_match(
				v.get("findings").and_then(|f| f.as_array()).unwrap(),
				"web",
				""
			),
			"non-bind mount {spec} must not fire sensitive_bind_mount; got status {:?} stdout: {}",
			status.code(),
			v
		);
	}
}

/// The host root bind is a superset of every other entry on the list:
/// the container sees every file the operator has. Matched as an exact
/// entry so the prefix loop does not fire on every absolute path.
/// Short and long form, default (rw) and read-only, all four shapes
/// must reach the same finding.
#[test]
fn audit_sensitive_bind_mount_flags_host_root() {
	for (service, src) in [
		("root_rw_short_svc", "/:/host"),
		("root_ro_short_svc", "/:/host:ro"),
		("root_x_short_svc", "/:/x:ro"),
		("root_data_short_svc", "/:/data:ro"),
	] {
		let body = format!(
			"services:\n  {service}:\n    image: alpine:3.20\n    volumes:\n      - {src}\n"
		);
		let (v, status) = audit_json_for(&body);
		assert_eq!(
			status.code(),
			Some(1),
			"sensitive bind mount of {src} must fail --strict; got {:?}\nstdout: {}",
			status.code(),
			v
		);
		let arr = v
			.get("findings")
			.and_then(|f| f.as_array())
			.expect("`findings` array");
		assert!(
			sensitive_match(arr, service, "whole host filesystem"),
			"expected sensitive_bind_mount for `{service}` mentioning the host-root \
			 capability; got {arr:?}"
		);
	}
	// And the long form, both access modes. The check must agree on
	// the same intent the short form names so a regression that
	// only parses the short form does not let `type: bind` mounts
	// slip through.
	let body_long_rw = r#"
services:
  root_long_rw_svc:
    image: alpine:3.20
    volumes:
      - type: bind
        source: /
        target: /host
"#;
	let (v_rw, status_rw) = audit_json_for(body_long_rw);
	assert_eq!(
		status_rw.code(),
		Some(1),
		"long-form host-root bind must fail --strict; got {:?}\nstdout: {}",
		status_rw.code(),
		v_rw
	);
	let arr_rw = v_rw
		.get("findings")
		.and_then(|f| f.as_array())
		.expect("`findings` array");
	assert!(
		sensitive_match(arr_rw, "root_long_rw_svc", "whole host filesystem"),
		"long-form host-root bind must produce a sensitive_bind_mount finding; got {arr_rw:?}"
	);
	let body_long_ro = r#"
services:
  root_long_ro_svc:
    image: alpine:3.20
    volumes:
      - type: bind
        source: /
        target: /host
        read_only: true
"#;
	let (v_ro, status_ro) = audit_json_for(body_long_ro);
	assert_eq!(
		status_ro.code(),
		Some(1),
		"long-form read-only host-root bind must fail --strict; got {:?}\nstdout: {}",
		status_ro.code(),
		v_ro
	);
	let arr_ro = v_ro
		.get("findings")
		.and_then(|f| f.as_array())
		.expect("`findings` array");
	assert!(
		sensitive_match(arr_ro, "root_long_ro_svc", "whole host filesystem")
			&& sensitive_match(arr_ro, "root_long_ro_svc", "read-only"),
		"long-form read-only host-root bind must name the access mode and capability; \
		 got {arr_ro:?}"
	);
}

/// The host root is an exact match, not a prefix match. `/tmp`,
/// `/home`, `/srv`, `/var` and `/opt` stay silent on purpose: the
/// project directory and scratch space live there, and operators
/// legitimately mount them. A regression that promotes the entry to
/// a prefix match fires here, on every absolute path in the compose
/// file, and the test goes red.
#[test]
fn audit_sensitive_bind_mount_host_root_does_not_act_as_prefix() {
	for (service, src) in [
		("tmp_svc", "/tmp:/t"),
		("home_svc", "/home:/h"),
		("srv_svc", "/srv:/s"),
		("var_svc", "/var:/v"),
		("opt_svc", "/opt:/o"),
	] {
		let body = format!(
			"services:\n  {service}:\n    image: alpine:3.20\n    volumes:\n      - {src}\n"
		);
		let (v, _status) = audit_json_for(&body);
		let arr = v
			.get("findings")
			.and_then(|f| f.as_array())
			.expect("`findings` array");
		assert!(
			!arr.iter().any(|f| f.get("check").and_then(|c| c.as_str())
				== Some("sensitive_bind_mount")
				&& f.get("reason")
					.and_then(|r| r.as_str())
					.is_some_and(|r| r.contains("whole host filesystem"))),
			"{src} must not fire as a host-root finding; got {arr:?}"
		);
	}
}

/// POSIX treats duplicate leading slashes (`//`, `///`) as a single
/// `/`, and `/.` as the current directory of `/`, which is `/`
/// itself. The normaliser folds every spelling the issue calls out
/// to the canonical `/` so the exact-match entry fires once and only
/// once regardless of how the operator wrote the path. Asserts the
/// actual behaviour of the normaliser at the binary level.
#[test]
fn audit_sensitive_bind_mount_normalises_host_root_spellings() {
	for (service, src) in [
		("root_bare_svc", "/:/host"),
		("root_double_svc", "//:/host"),
		("root_triple_svc", "///:/host"),
		("root_dot_svc", "/.:/host"),
		("root_dot_slash_svc", "/./:/host"),
		("root_double_dot_svc", "//.:/host"),
	] {
		let body = format!(
			"services:\n  {service}:\n    image: alpine:3.20\n    volumes:\n      - {src}\n"
		);
		let (v, status) = audit_json_for(&body);
		assert_eq!(
			status.code(),
			Some(1),
			"host-root spelling {src} must fail --strict; got {:?}\nstdout: {}",
			status.code(),
			v
		);
		let arr = v
			.get("findings")
			.and_then(|f| f.as_array())
			.expect("`findings` array");
		assert!(
			sensitive_match(arr, service, "whole host filesystem"),
			"{src} must fold to / and fire sensitive_bind_mount; got {arr:?}"
		);
	}
	// And the negative shape: a path the normaliser does NOT fold to
	// `/` must stay silent on the host-root wording, even when it
	// looks root-ish.
	for (service, src) in [
		("rootish_x_svc", "/x:/data"),
		("rootish_dot_x_svc", "/.x:/data"),
	] {
		let body = format!(
			"services:\n  {service}:\n    image: alpine:3.20\n    volumes:\n      - {src}\n"
		);
		let (v, _status) = audit_json_for(&body);
		let arr = v
			.get("findings")
			.and_then(|f| f.as_array())
			.expect("`findings` array");
		assert!(
			!arr.iter().any(|f| f.get("check").and_then(|c| c.as_str())
				== Some("sensitive_bind_mount")
				&& f.get("reason")
					.and_then(|r| r.as_str())
					.is_some_and(|r| r.contains("whole host filesystem"))),
			"{src} must not be reported as the host root; got {arr:?}"
		);
	}
}
