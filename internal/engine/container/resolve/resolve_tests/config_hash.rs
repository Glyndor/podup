use super::*;

#[test]
fn config_hash_is_stable_and_sensitive() {
	let base = temp_base();
	let a = parse_str("services:\n  web:\n    image: nginx:1.27\n").unwrap();
	let b = parse_str("services:\n  web:\n    image: nginx:1.27\n").unwrap();
	let c = parse_str("services:\n  web:\n    image: nginx:1.28\n").unwrap();
	let ha = config_hash(
		&a.services["web"],
		&a,
		PROJECT,
		base.path(),
		&empty_digests(),
	)
	.unwrap();
	let hb = config_hash(
		&b.services["web"],
		&b,
		PROJECT,
		base.path(),
		&empty_digests(),
	)
	.unwrap();
	let hc = config_hash(
		&c.services["web"],
		&c,
		PROJECT,
		base.path(),
		&empty_digests(),
	)
	.unwrap();
	assert_eq!(ha, hb, "same config produces the same hash");
	assert_ne!(ha, hc, "a changed image produces a different hash");
	assert_eq!(ha.len(), 64, "sha-256 hex is 64 chars");
}

#[test]
fn config_hash_tracks_inline_secret_content() {
	// Rotating an inline `content:` secret must change the hash so the
	// container is recreated to pick up the new (point-in-time) native secret.
	let base = temp_base();
	let a = parse_str(
		"services:\n  web:\n    image: x\n    secrets: [tok]\nsecrets:\n  tok:\n    content: v1\n",
	)
	.unwrap();
	let b = parse_str(
		"services:\n  web:\n    image: x\n    secrets: [tok]\nsecrets:\n  tok:\n    content: v2\n",
	)
	.unwrap();
	assert_ne!(
		config_hash(
			&a.services["web"],
			&a,
			PROJECT,
			base.path(),
			&empty_digests()
		)
		.unwrap(),
		config_hash(
			&b.services["web"],
			&b,
			PROJECT,
			base.path(),
			&empty_digests()
		)
		.unwrap(),
		"changed inline secret content must change the hash",
	);
}

#[test]
fn config_hash_ignores_external_secret_identity() {
	// An `external:` secret is by-reference (no payload), so it does not add
	// to the hash beyond the service's own secret list.
	let base = temp_base();
	let a = parse_str(
		"services:\n  web:\n    image: x\n    secrets: [tok]\nsecrets:\n  tok:\n    external: true\n",
	)
	.unwrap();
	let b = parse_str(
		"services:\n  web:\n    image: x\n    secrets: [tok]\nsecrets:\n  tok:\n    external: true\n    name: other\n",
	)
	.unwrap();
	// The service definition is identical; only the top-level external name
	// differs, which is resolved at attach time, not baked into the hash.
	assert_eq!(
		config_hash(
			&a.services["web"],
			&a,
			PROJECT,
			base.path(),
			&empty_digests()
		)
		.unwrap(),
		config_hash(
			&b.services["web"],
			&b,
			PROJECT,
			base.path(),
			&empty_digests()
		)
		.unwrap(),
	);
}

#[test]
fn config_hash_tracks_inline_config_content() {
	// The same recreate-on-rotation contract as inline secrets applies to
	// inline `configs:` content.
	let base = temp_base();
	let a = parse_str(
		"services:\n  web:\n    image: x\n    configs: [cfg]\nconfigs:\n  cfg:\n    content: a\n",
	)
	.unwrap();
	let b = parse_str(
		"services:\n  web:\n    image: x\n    configs: [cfg]\nconfigs:\n  cfg:\n    content: b\n",
	)
	.unwrap();
	assert_ne!(
		config_hash(
			&a.services["web"],
			&a,
			PROJECT,
			base.path(),
			&empty_digests()
		)
		.unwrap(),
		config_hash(
			&b.services["web"],
			&b,
			PROJECT,
			base.path(),
			&empty_digests()
		)
		.unwrap(),
		"changed inline config content must change the hash",
	);
}

#[test]
fn config_hash_tracks_environment_sourced_secret() {
	// An `environment:`-sourced secret folds the *current* value of the named
	// variable into the hash, so a change to that variable recreates.
	let base = temp_base();
	let file = parse_str(
		"services:\n  web:\n    image: x\n    secrets: [tok]\nsecrets:\n  tok:\n    environment: PODUP_TEST_SECRET\n",
	)
	.unwrap();
	let with_a = temp_env::with_var("PODUP_TEST_SECRET", Some("alpha"), || {
		config_hash(
			&file.services["web"],
			&file,
			PROJECT,
			base.path(),
			&empty_digests(),
		)
		.unwrap()
	});
	let with_b = temp_env::with_var("PODUP_TEST_SECRET", Some("beta"), || {
		config_hash(
			&file.services["web"],
			&file,
			PROJECT,
			base.path(),
			&empty_digests(),
		)
		.unwrap()
	});
	assert_ne!(
		with_a, with_b,
		"a changed environment-sourced secret value must change the hash",
	);
}

#[test]
fn config_hash_stable_despite_map_field_order() {
	// `storage_opt` is a HashMap; canonical serialisation must sort its keys
	// so the hash does not flap and trigger a spurious recreate on `up`.
	let base = temp_base();
	let a = parse_str(
		"services:\n  web:\n    image: x\n    storage_opt:\n      size: \"10G\"\n      foo: bar\n      baz: qux\n",
	)
	.unwrap();
	let b = parse_str(
		"services:\n  web:\n    image: x\n    storage_opt:\n      baz: qux\n      size: \"10G\"\n      foo: bar\n",
	)
	.unwrap();
	assert_eq!(
		config_hash(
			&a.services["web"],
			&a,
			PROJECT,
			base.path(),
			&empty_digests()
		)
		.unwrap(),
		config_hash(
			&b.services["web"],
			&b,
			PROJECT,
			base.path(),
			&empty_digests()
		)
		.unwrap(),
		"hash must be independent of storage_opt key order",
	);
}
