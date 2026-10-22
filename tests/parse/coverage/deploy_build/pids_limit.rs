//! The top-level service `pids_limit` field.
//!
//! Split out of `tests/parse/coverage/deploy_build/mod.rs`; the imports are
//! inherited via `use super::*`.

use super::*;

// ---------------------------------------------------------------------------
// pids_limit (top-level service field)
// ---------------------------------------------------------------------------

#[test]
fn pids_limit_service_field() {
	let yaml = "services:\n  app:\n    image: alpine\n    pids_limit: 256\n";
	assert_eq!(
		parse_str(yaml).unwrap().services["app"].pids_limit,
		Some(256)
	);
}
