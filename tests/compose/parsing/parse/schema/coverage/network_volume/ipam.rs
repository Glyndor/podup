//! The `ipam` options of a network.
//!
//! Split out of `tests/compose/parsing/parse/schema/coverage/network_volume/mod.rs`; the imports are
//! inherited via `use super::*`.

use super::*;

// ---------------------------------------------------------------------------
// Networks: ipam options, multiple pools
// ---------------------------------------------------------------------------

#[test]
fn network_ipam_options() {
	let yaml = r#"
networks:
  mynet:
    ipam:
      driver: custom
      options:
        foo: bar
"#;
	let file = parse_str(yaml).unwrap();
	let ipam = file.networks["mynet"]
		.as_ref()
		.unwrap()
		.ipam
		.as_ref()
		.unwrap();
	assert_eq!(ipam.driver.as_deref(), Some("custom"));
	assert_eq!(ipam.options.get("foo").map(|s| s.as_str()), Some("bar"));
}
