//! The CPU realtime, count and percent fields.
//!
//! Split out of `tests/compose/parsing/parse/schema/coverage/network_volume/mod.rs`; the imports are
//! inherited via `use super::*`.

use super::*;

// ---------------------------------------------------------------------------
// CPU realtime / cpu_count / cpu_percent fields
// ---------------------------------------------------------------------------

#[test]
fn cpu_count_and_percent() {
	let yaml = r#"
services:
  app:
    image: alpine
    cpu_count: 4
    cpu_percent: 75
"#;
	let svc = &parse_str(yaml).unwrap().services["app"];
	assert_eq!(svc.cpu_count, Some(4));
	assert_eq!(svc.cpu_percent, Some(75));
}

#[test]
fn cpu_rt_runtime_and_period() {
	let yaml = r#"
services:
  app:
    image: alpine
    cpu_rt_runtime: 950000
    cpu_rt_period: 1000000
"#;
	let svc = &parse_str(yaml).unwrap().services["app"];
	assert_eq!(svc.cpu_rt_runtime, Some(950000));
	assert_eq!(svc.cpu_rt_period, Some(1000000));
}
