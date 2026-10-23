//! Rows matched by an exact socket path. The reason uses stronger wording
//! than the directory-prefix matches, so the tests pin that wording rather
//! than just the check id.
//!
//! Split out of `tests/audit_exit_codes/sensitive_bind/mod.rs`; the helpers
//! are inherited via `use super::*`.

use super::*;

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
