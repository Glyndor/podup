//! The host root entry: a superset of every other row on the list, matched
//! exactly rather than by prefix, in every spelling the normaliser folds.
//!
//! Split out of `tests/audit/audit_exit_codes/sensitive_bind/mod.rs`; the helpers
//! are inherited via `use super::*`.

use super::*;

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
