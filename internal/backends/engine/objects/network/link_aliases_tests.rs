use super::*;
use crate::parse_str;

/// The file as the engine sees it: the implicit `default` network is already
/// attached to every service that names none.
fn file(yaml: &str) -> ComposeFile {
	let mut file = parse_str(yaml).unwrap();
	crate::compose::normalize_default_network(&mut file);
	file
}

#[test]
fn alias_link_adds_the_alias_on_a_shared_network() {
	let f = file(
		"services:\n  db:\n    image: x\n  web:\n    image: x\n    links:\n      - db:database\n",
	);
	assert_eq!(link_aliases(&f, "db", "default"), vec!["database"]);
}

#[test]
fn bare_link_and_same_name_alias_add_nothing() {
	let f = file(
		"services:\n  db:\n    image: x\n  web:\n    image: x\n    links:\n      - db\n      - db:db\n",
	);
	assert!(link_aliases(&f, "db", "default").is_empty());
}

#[test]
fn only_the_linked_service_gets_the_alias() {
	let f = file(
		"services:\n  db:\n    image: x\n  cache:\n    image: x\n  web:\n    image: x\n    links:\n      - db:database\n",
	);
	assert!(link_aliases(&f, "cache", "default").is_empty());
	assert!(link_aliases(&f, "web", "default").is_empty());
}

#[test]
fn no_shared_network_means_no_alias() {
	let f = file(
		"services:\n  db:\n    image: x\n    networks: [back]\n  web:\n    image: x\n    networks: [front]\n    links:\n      - db:database\nnetworks:\n  back: {}\n  front: {}\n",
	);
	// `db` only joins `back`, where `web` is not.
	assert!(link_aliases(&f, "db", "back").is_empty());
}

#[test]
fn a_linker_on_two_networks_aliases_both_and_several_linkers_dedupe() {
	let f = file(
		"services:\n  db:\n    image: x\n    networks: [a, b]\n  web:\n    image: x\n    networks: [a, b]\n    links:\n      - db:database\n  worker:\n    image: x\n    networks: [a]\n    links:\n      - db:database\n      - db:sql\nnetworks:\n  a: {}\n  b: {}\n",
	);
	assert_eq!(link_aliases(&f, "db", "a"), vec!["database", "sql"]);
	assert_eq!(link_aliases(&f, "db", "b"), vec!["database"]);
}

#[test]
fn a_linker_in_network_mode_joins_no_network() {
	let f = file(
		"services:\n  db:\n    image: x\n  web:\n    image: x\n    network_mode: host\n    links:\n      - db:database\n",
	);
	assert!(link_aliases(&f, "db", "default").is_empty());
}

#[test]
fn hostile_link_entries_do_not_panic_or_match() {
	let f = file(
		"services:\n  db:\n    image: x\n  web:\n    image: x\n    links:\n      - \":\"\n      - \"db:\"\n      - \":database\"\n      - \"db:a:b\"\n",
	);
	// An empty alias is skipped; `db:a:b` splits at the first colon, so the
	// target is `db` and the alias is `a:b`, which Podman then validates.
	assert_eq!(link_aliases(&f, "db", "default"), vec!["a:b"]);
}
