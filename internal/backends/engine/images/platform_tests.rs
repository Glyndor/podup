use super::*;

fn parts(platform: &str) -> (Option<String>, Option<String>, Option<String>) {
	let p = Platform::parse(platform);
	(p.os, p.arch, p.variant)
}

fn some(value: &str) -> Option<String> {
	Some(value.to_string())
}

#[test]
fn os_and_architecture_are_split() {
	assert_eq!(parts("linux/arm64"), (some("linux"), some("arm64"), None));
}

#[test]
fn a_variant_stays_out_of_the_architecture() {
	assert_eq!(
		parts("linux/arm/v7"),
		(some("linux"), some("arm"), some("v7"))
	);
}

#[test]
fn a_bare_architecture_has_no_os() {
	assert_eq!(parts("arm64"), (None, some("arm64"), None));
}

#[test]
fn the_pull_query_names_every_part_it_has() {
	assert_eq!(
		Platform::parse("linux/arm/v7").pull_query(),
		"&os=linux&arch=arm&variant=v7"
	);
	assert_eq!(Platform::parse("arm64").pull_query(), "&arch=arm64");
}
