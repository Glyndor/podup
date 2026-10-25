//! The chain-depth guard, on the in-memory parser.
//!
//! Split out of `tests/compose/parsing/parse/extends/mod.rs`; the imports are inherited via
//! `use super::*`.

use super::*;

fn make_chain_yaml(depth: usize) -> String {
	let mut yaml = "services:\n".to_string();
	// Reverse order so the deepest service comes first, forcing full recursion.
	for i in (1..=depth).rev() {
		yaml.push_str(&format!("  s{i}:\n    extends: s{}\n", i - 1));
	}
	yaml.push_str("  s0:\n    image: alpine\n");
	yaml
}

#[test]
fn extends_chain_within_depth_limit() {
	// 16 services = 15 hops, which must succeed.
	let yaml = make_chain_yaml(15);
	assert!(
		parse_str(&yaml).is_ok(),
		"chain of 15 hops must be accepted"
	);
}

#[test]
fn extends_chain_exceeds_depth_limit() {
	// 17 services = 16 hops, which must be rejected.
	let yaml = make_chain_yaml(16);
	let err = parse_str(&yaml).unwrap_err();
	let msg = err.to_string();
	assert!(
		msg.contains("exceeds maximum depth"),
		"error must mention depth limit, got: {msg}"
	);
}
