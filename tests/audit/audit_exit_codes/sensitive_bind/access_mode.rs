//! The access mode never clears a finding: `:ro` discloses as much as `:rw`.
//!
//! Split out of `tests/audit/audit_exit_codes/sensitive_bind/mod.rs`; the helpers
//! are inherited via `use super::*`.

use super::*;

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
