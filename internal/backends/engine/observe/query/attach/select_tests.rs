use super::*;

fn file() -> ComposeFile {
	crate::parse_str(
		"services:\n  web:\n    image: a\n  db:\n    image: b\n  \
		 worker:\n    image: c\n    attach: false\n",
	)
	.unwrap()
}

fn names(options: &AttachOptions) -> Vec<String> {
	attached_services(&file(), options)
		.unwrap()
		.into_iter()
		.map(|(name, _)| name.to_string())
		.collect()
}

fn list(items: &[&str]) -> Vec<String> {
	items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn every_service_but_the_ones_that_opted_out_is_followed_by_default() {
	let mut got = names(&AttachOptions::new());
	got.sort();
	assert_eq!(got, ["db", "web"]);
}

#[test]
fn attach_narrows_the_set_to_the_named_services() {
	let options = AttachOptions::new().with_attach(list(&["db"]));
	assert_eq!(names(&options), ["db"]);
}

#[test]
fn attach_does_not_override_a_service_that_opted_out() {
	let options = AttachOptions::new().with_attach(list(&["worker"]));
	assert!(names(&options).is_empty());
}

#[test]
fn no_attach_removes_the_named_services() {
	let options = AttachOptions::new().with_no_attach(list(&["web"]));
	assert_eq!(names(&options), ["db"]);
}

#[test]
fn no_attach_wins_over_attach() {
	let options = AttachOptions::new()
		.with_attach(list(&["web", "db"]))
		.with_no_attach(list(&["web"]));
	assert_eq!(names(&options), ["db"]);
}

#[test]
fn a_name_that_is_not_a_service_is_an_error() {
	for options in [
		AttachOptions::new().with_attach(list(&["nope"])),
		AttachOptions::new().with_no_attach(list(&["nope"])),
	] {
		let error = attached_services(&file(), &options).unwrap_err();
		assert!(matches!(error, ComposeError::ServiceNotFound(n) if n == "nope"));
	}
}
