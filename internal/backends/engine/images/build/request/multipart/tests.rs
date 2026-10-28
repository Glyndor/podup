use super::*;

#[test]
fn a_boundary_is_drawn_per_request() {
	let first = boundary();
	let second = boundary();
	assert!(
		first.starts_with("podup-build-"),
		"unexpected boundary: {first}"
	);
	assert_ne!(first, second, "two requests must not share a boundary");
}

#[test]
fn a_part_header_names_the_part() {
	let mut out = Vec::new();
	write_part_header(&mut out, "BOUND", "build-context-resources").unwrap();
	let header = String::from_utf8(out).expect("the header is ASCII");
	assert!(
		header.starts_with("--BOUND\r\n"),
		"the part must start with the boundary: {header:?}"
	);
	assert!(
		header.contains("name=\"build-context-resources\""),
		"the part must name itself: {header:?}"
	);
	assert!(
		header.ends_with("\r\n\r\n"),
		"the header must end the part's headers: {header:?}"
	);
}

#[test]
fn the_main_context_part_is_named_as_the_endpoint_expects() {
	let mut out = Vec::new();
	write_part_header(&mut out, "BOUND", "MainContext").unwrap();
	let header = String::from_utf8(out).expect("the header is ASCII");
	assert!(
		header.contains("name=\"MainContext\""),
		"the main context part must carry the name the endpoint reads: {header:?}"
	);
}
