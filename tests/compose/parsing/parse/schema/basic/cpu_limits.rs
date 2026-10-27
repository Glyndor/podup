//! `cpu_shares`, `cpuset`, `mem_limit` and `cpus`, in the spellings the compose
//! spec writes them.
//!
//! Split out of `tests/compose/parsing/parse/schema/basic/mod.rs`; the imports are inherited via
//! `use super::*`.

use super::*;

#[test]
fn cpu_fields() {
	let yaml = r#"
services:
  app:
    image: alpine
    cpu_shares: 512
    cpuset: "0-1"
    mem_limit: 256m
"#;
	let svc = &parse_str(yaml).unwrap().services["app"];
	assert_eq!(svc.cpu_shares, Some(512));
	assert_eq!(svc.cpuset.as_deref(), Some("0-1"));
	assert_eq!(svc.mem_limit.as_deref(), Some("256m"));
}

#[test]
fn cpus_accepts_unquoted_number_and_quoted_string() {
	// The compose spec writes `cpus: 0.5` unquoted; both the number and the quoted
	// string form must parse to the same stored value (top-level and deploy).
	let yaml = r#"
services:
  num:
    image: alpine
    cpus: 0.5
  str:
    image: alpine
    cpus: "0.5"
  dep:
    image: alpine
    deploy:
      resources:
        limits:
          cpus: 1.5
"#;
	let file = parse_str(yaml).unwrap();
	assert_eq!(file.services["num"].cpus.as_deref(), Some("0.5"));
	assert_eq!(file.services["str"].cpus.as_deref(), Some("0.5"));
	assert_eq!(
		file.services["dep"]
			.deploy
			.as_ref()
			.unwrap()
			.resources
			.as_ref()
			.unwrap()
			.limits
			.as_ref()
			.unwrap()
			.cpus
			.as_deref(),
		Some("1.5")
	);
}
