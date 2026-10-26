use super::*;

#[test]
fn links_resolve_to_container_names_external_links_verbatim() {
	let file = parse_str(
		"services:\n  db:\n    image: x\n  web:\n    image: x\n    links:\n      - db\n      - db:primary\n    external_links:\n      - legacy_db:db\n",
	)
	.unwrap();
	let links = resolve_links(&file.services["web"], &file, "proj");
	// Auto-generated names are always index-suffixed; a link targets the
	// first replica.
	assert!(links.contains(&"proj-db-1:db".to_string()));
	assert!(links.contains(&"proj-db-1:primary".to_string()));
	assert!(links.contains(&"legacy_db:db".to_string()));
}

#[test]
fn links_honour_custom_container_name() {
	let file = parse_str(
		"services:\n  db:\n    image: x\n    container_name: my-db\n  web:\n    image: x\n    links:\n      - db\n",
	)
	.unwrap();
	let links = resolve_links(&file.services["web"], &file, "proj");
	assert_eq!(links, vec!["my-db:db".to_string()]);
}
