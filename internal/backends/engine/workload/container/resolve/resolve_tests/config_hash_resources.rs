use super::*;

/// A top-level network's real name is part of the container's spec: the
/// container is attached to the *resolved* name, not to the compose key. A
/// rename of that name must therefore change the hash, or `up` keeps the
/// container attached to the network it was created with.
#[test]
fn config_hash_tracks_the_resolved_network_name() {
	let base = temp_base();
	let a = parse_str(
		"services:\n  web:\n    image: x\n    networks: [interna]\nnetworks:\n  interna:\n    name: red_v1\n",
	)
	.unwrap();
	let b = parse_str(
		"services:\n  web:\n    image: x\n    networks: [interna]\nnetworks:\n  interna:\n    name: red_v2\n",
	)
	.unwrap();
	assert_ne!(
		config_hash(
			&a.services["web"],
			&a,
			PROJECT,
			base.path(),
			&empty_digests(),
			&empty_env()
		)
		.unwrap(),
		config_hash(
			&b.services["web"],
			&b,
			PROJECT,
			base.path(),
			&empty_digests(),
			&empty_env()
		)
		.unwrap(),
		"renaming the network the service attaches to must change the hash",
	);
}

/// The same contract for a named volume: the container mounts the *resolved*
/// volume name, so renaming the top-level volume must recreate the container.
#[test]
fn config_hash_tracks_the_resolved_volume_name() {
	let base = temp_base();
	let a = parse_str(
		"services:\n  web:\n    image: x\n    volumes: [datos:/data]\nvolumes:\n  datos:\n    name: vol_v1\n",
	)
	.unwrap();
	let b = parse_str(
		"services:\n  web:\n    image: x\n    volumes: [datos:/data]\nvolumes:\n  datos:\n    name: vol_v2\n",
	)
	.unwrap();
	assert_ne!(
		config_hash(
			&a.services["web"],
			&a,
			PROJECT,
			base.path(),
			&empty_digests(),
			&empty_env()
		)
		.unwrap(),
		config_hash(
			&b.services["web"],
			&b,
			PROJECT,
			base.path(),
			&empty_digests(),
			&empty_env()
		)
		.unwrap(),
		"renaming the volume the service mounts must change the hash",
	);
}

/// The control on the two tests above: a top-level resource the service does
/// not reference changes nothing about the container's spec, so it must not
/// change the hash either. Without this, a fix that hashes the whole
/// top-level `networks:`/`volumes:` map would pass the two tests above and
/// recreate every container on every unrelated rename.
#[test]
fn config_hash_ignores_resources_the_service_does_not_reference() {
	let base = temp_base();
	let a = parse_str(
		"services:\n  web:\n    image: x\n    networks: [interna]\nnetworks:\n  interna:\n    name: red_v1\n  otra:\n    name: red_x\nvolumes:\n  datos:\n    name: vol_v1\n",
	)
	.unwrap();
	let b = parse_str(
		"services:\n  web:\n    image: x\n    networks: [interna]\nnetworks:\n  interna:\n    name: red_v1\n  otra:\n    name: red_y\nvolumes:\n  datos:\n    name: vol_z\n",
	)
	.unwrap();
	assert_eq!(
		config_hash(
			&a.services["web"],
			&a,
			PROJECT,
			base.path(),
			&empty_digests(),
			&empty_env()
		)
		.unwrap(),
		config_hash(
			&b.services["web"],
			&b,
			PROJECT,
			base.path(),
			&empty_digests(),
			&empty_env()
		)
		.unwrap(),
		"a rename of a resource the service never references must not change the hash",
	);
}

/// The project-prefixed default name of a referenced network or volume is
/// derived from the compose key the service already carries, so it is not
/// folded into the hash. Folding it would change the hash of every existing
/// container that has a network or a named volume on the first `up` after the
/// upgrade, recreating containers whose writable layer would be lost.
#[test]
fn config_hash_leaves_the_default_resource_names_out() {
	let file = parse_str(
		"services:\n  web:\n    image: x\n    networks: [interna]\n    volumes: [datos:/data]\n",
	)
	.unwrap();
	let identities = overridden_resource_identities(&file.services["web"], &file, PROJECT);
	assert!(
		identities.is_empty(),
		"a network and a volume that keep their default names contribute nothing: {identities:?}",
	);
}

/// The other half of the same rule: a name the top-level block *overrides* is
/// not derivable from the service's own key, so it is folded in — under the
/// name the container is really attached to or mounts.
#[test]
fn config_hash_folds_an_overridden_resource_name() {
	let file = parse_str(
		"services:\n  web:\n    image: x\n    networks: [interna]\nnetworks:\n  interna:\n    name: red_v1\n",
	)
	.unwrap();
	assert_eq!(
		overridden_resource_identities(&file.services["web"], &file, PROJECT),
		vec![(&b"network"[..], "interna".to_string(), "red_v1".to_string())],
	);
}

/// An `external: true` resource is named by its own key rather than by the
/// project-prefixed default, so it is an override too and has to be folded in:
/// the container is attached to (or mounts) a different name than it would
/// without the flag.
#[test]
fn config_hash_folds_an_external_resource_name() {
	let file = parse_str(
		"services:\n  web:\n    image: x\n    volumes: [datos:/data]\nvolumes:\n  datos:\n    external: true\n",
	)
	.unwrap();
	assert_eq!(
		overridden_resource_identities(&file.services["web"], &file, PROJECT),
		vec![(&b"volume"[..], "datos".to_string(), "datos".to_string())],
	);
}
