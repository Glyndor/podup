use super::*;

fn reference(entry: &str) -> Option<String> {
	image_reference(entry)
}

#[test]
fn a_bare_name_is_an_image_reference() {
	assert_eq!(
		reference("other.example/cache:latest"),
		Some("other.example/cache:latest".to_string())
	);
	assert_eq!(
		reference("  other.example/cache:latest  "),
		Some("other.example/cache:latest".to_string())
	);
}

#[test]
fn a_registry_cache_names_its_reference() {
	assert_eq!(
		reference("type=registry,ref=example/app:cache"),
		Some("example/app:cache".to_string())
	);
	assert_eq!(
		reference("type=registry,ref=example/app:cache,mode=max"),
		Some("example/app:cache".to_string())
	);
}

#[test]
fn a_registry_cache_without_a_reference_is_dropped() {
	assert_eq!(reference("type=registry"), None);
	assert_eq!(reference("type=registry,ref="), None);
}

#[test]
fn a_cache_type_without_an_equivalent_is_dropped() {
	assert_eq!(reference("type=local,src=path/to/cache"), None);
	assert_eq!(reference("type=local,dest=path/to/cache"), None);
	assert_eq!(reference("type=gha"), None);
}

#[test]
fn an_empty_entry_is_dropped() {
	assert_eq!(reference(""), None);
	assert_eq!(reference("   "), None);
}

#[test]
fn a_tag_is_cut_from_the_repository() {
	assert_eq!(repository("example/app:cache"), "example/app");
	assert_eq!(repository("example/app"), "example/app");
}

#[test]
fn a_digest_is_cut_from_the_repository() {
	let digest = "a".repeat(64);
	assert_eq!(
		repository(&format!("example/app@sha256:{digest}")),
		"example/app"
	);
	assert_eq!(
		repository(&format!("example/app:cache@sha256:{digest}")),
		"example/app"
	);
}

#[test]
fn a_registry_port_is_not_a_tag() {
	assert_eq!(
		repository("registry.example:5000/team/app:cache"),
		"registry.example:5000/team/app"
	);
	assert_eq!(
		repository("registry.example:5000/team/app"),
		"registry.example:5000/team/app"
	);
}

#[test]
fn references_travel_without_tag_or_digest() {
	let entries = vec![
		"example/app:cache".to_string(),
		"type=registry,ref=registry.example:5000/app:v2,mode=max".to_string(),
	];
	assert_eq!(
		references(CacheField::From, &entries),
		vec![
			"example/app".to_string(),
			"registry.example:5000/app".to_string()
		]
	);
	assert_eq!(
		references(CacheField::To, &entries),
		vec![
			"example/app".to_string(),
			"registry.example:5000/app".to_string()
		]
	);
}
