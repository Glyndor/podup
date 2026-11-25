use super::*;

fn hash_of(file: &crate::compose::types::ComposeFile, service: &str) -> String {
	let base = temp_base();
	config_hash(
		&file.services[service],
		file,
		PROJECT,
		base.path(),
		&empty_digests(),
		&empty_env(),
	)
	.unwrap()
}

fn with_default_network(yaml: &str) -> crate::compose::types::ComposeFile {
	let mut file = parse_str(yaml).unwrap();
	crate::compose::normalize_default_network(&mut file);
	file
}

/// A link alias is a network alias of the container it points at, so adding
/// one to a sibling has to recreate the target and nothing else.
#[test]
fn a_link_alias_on_a_sibling_changes_the_target_hash_only() {
	let without = with_default_network(
		"services:\n  db:\n    image: x\n  cache:\n    image: x\n  web:\n    image: x\n",
	);
	let with = with_default_network(
		"services:\n  db:\n    image: x\n  cache:\n    image: x\n  web:\n    image: x\n    links:\n      - db:database\n",
	);
	assert_ne!(hash_of(&without, "db"), hash_of(&with, "db"));
	assert_eq!(hash_of(&without, "cache"), hash_of(&with, "cache"));
}

/// A bare link adds no alias, so the target's hash is what it was before this
/// existed and no container is recreated by an upgrade.
#[test]
fn a_bare_link_leaves_the_target_hash_alone() {
	let without = with_default_network("services:\n  db:\n    image: x\n  web:\n    image: x\n");
	let bare = with_default_network(
		"services:\n  db:\n    image: x\n  web:\n    image: x\n    links:\n      - db\n",
	);
	assert_eq!(hash_of(&without, "db"), hash_of(&bare, "db"));
}

#[test]
fn a_renamed_link_alias_changes_the_target_hash() {
	let a = with_default_network(
		"services:\n  db:\n    image: x\n  web:\n    image: x\n    links:\n      - db:one\n",
	);
	let b = with_default_network(
		"services:\n  db:\n    image: x\n  web:\n    image: x\n    links:\n      - db:two\n",
	);
	assert_ne!(hash_of(&a, "db"), hash_of(&b, "db"));
}
