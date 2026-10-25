//! Mounts the check must leave alone on purpose: the project directory an
//! operator chose, and the volume forms that are not host binds at all.
//!
//! Split out of `tests/audit/audit_exit_codes/sensitive_bind/mod.rs`; the helpers
//! are inherited via `use super::*`.

use super::*;

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
	              mem_limit: 512m\n    memswap_limit: 512m\n    init: true\n    \
	              restart: unless-stopped\n    cpus: \"1\"\n    \
	              healthcheck:\n      test: [\"CMD\", \"true\"]\n      \
	              x-podman-on-failure: restart\n    userns_mode: auto\n    \
	              volumes:\n      - ";
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
