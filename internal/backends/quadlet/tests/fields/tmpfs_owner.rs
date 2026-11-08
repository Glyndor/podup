//! `uid=`/`gid=` in a `tmpfs:` entry reach a unit the way they reach `up`: as
//! Podman's `U` when they name the service's `user:`, refused otherwise.

use crate::parse_str;
use crate::quadlet::{generate_at, validate_for_quadlet};

use super::super::unit_named;

const OWNED: &str = "services:\n  s:\n    image: app:1.0\n    user: \"1000:1000\"\n    tmpfs: [\"/t:uid=1000,gid=1000,mode=1777\"]\n";

#[test]
fn an_owner_that_is_the_user_is_written_as_u() {
	let file = parse_str(OWNED).unwrap();
	validate_for_quadlet(&file).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-s.container").contents;
	assert!(c.contains("Tmpfs=/t:mode=1777,U"), "{c}");
	assert!(!c.contains("uid=1000"), "{c}");
}

#[test]
fn an_owner_that_is_not_the_user_is_refused_before_a_unit_is_written() {
	let yaml = OWNED.replace("1000:1000", "2000:2000");
	let err = validate_for_quadlet(&parse_str(&yaml).unwrap()).expect_err("not the user");
	let msg = err.to_string();
	assert!(
		msg.contains("Podman has no uid/gid option for a tmpfs"),
		"{msg}"
	);
}

#[test]
fn a_control_character_in_the_owner_does_not_reach_the_message() {
	let yaml = "services:\n  s:\n    image: app:1.0\n    tmpfs: [\"/t:uid=\\u001b[31m\"]\n";
	let msg = validate_for_quadlet(&parse_str(yaml).unwrap())
		.expect_err("no user")
		.to_string();
	assert!(!msg.contains('\u{1b}'), "{msg:?}");
}
