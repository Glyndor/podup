//! The `uts` and `cgroup` namespace fields.
//!
//! Split out of `tests/compose/parsing/parse/coverage/network_volume/mod.rs`; the imports are
//! inherited via `use super::*`.

use super::*;

// ---------------------------------------------------------------------------
// uts, cgroup namespace
// ---------------------------------------------------------------------------

#[test]
fn uts_host() {
	let yaml = "services:\n  app:\n    image: alpine\n    uts: host\n";
	assert_eq!(
		parse_str(yaml).unwrap().services["app"].uts.as_deref(),
		Some("host")
	);
}

#[test]
fn cgroup_field() {
	let yaml = "services:\n  app:\n    image: alpine\n    cgroup: host\n";
	assert_eq!(
		parse_str(yaml).unwrap().services["app"].cgroup.as_deref(),
		Some("host")
	);
}
