use super::*;

#[test]
fn volumes_from_resolves_to_container_names() {
	let file = parse_str(
		"services:\n  db:\n    image: x\n  cache:\n    image: x\n    container_name: my-cache\n  web:\n    image: x\n    volumes_from:\n      - db\n      - cache\n",
	)
	.unwrap();
	let resolved = resolve_volumes_from(&file.services["web"], &file, "proj");
	// Bare service name resolves to the first replica `{project}-{service}-1`.
	assert!(resolved.contains(&"proj-db-1".to_string()));
	// An explicit container_name is honoured.
	assert!(resolved.contains(&"my-cache".to_string()));
}

#[test]
fn volumes_from_preserves_access_mode() {
	let file = parse_str(
		"services:\n  db:\n    image: x\n  web:\n    image: x\n    volumes_from:\n      - db:ro\n      - service:db:rw\n",
	)
	.unwrap();
	let resolved = resolve_volumes_from(&file.services["web"], &file, "proj");
	// The `:ro`/`:rw` suffix survives the rewrite, and the `service:` prefix
	// is stripped before resolving.
	assert!(resolved.contains(&"proj-db-1:ro".to_string()));
	assert!(resolved.contains(&"proj-db-1:rw".to_string()));
}

#[test]
fn volumes_from_passes_through_container_form_and_unknown() {
	let file = parse_str(
		"services:\n  web:\n    image: x\n    volumes_from:\n      - container:legacy-data\n      - container:legacy-data:ro\n      - missing\n",
	)
	.unwrap();
	let resolved = resolve_volumes_from(&file.services["web"], &file, "proj");
	// The `container:` form names a container outside the project verbatim.
	assert!(resolved.contains(&"legacy-data".to_string()));
	assert!(resolved.contains(&"legacy-data:ro".to_string()));
	// An unknown service is left unchanged.
	assert!(resolved.contains(&"missing".to_string()));
}
