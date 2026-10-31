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
