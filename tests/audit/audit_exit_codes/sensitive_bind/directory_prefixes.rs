//! Rows matched by directory prefix: the host directories on the sensitive
//! list, and the runtime directories that expose a socket without naming it.
//!
//! Split out of `tests/audit/audit_exit_codes/sensitive_bind/mod.rs`; the helpers
//! are inherited via `use super::*`.

use super::*;

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
