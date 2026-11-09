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

#[test]
fn a_part_name_that_would_split_the_header_is_refused() {
	for name in [
		"x\"\r\nX-Injected: 1",
		"x\r\nX-Injected: 1",
		"x\nX-Injected: 1",
		"a\"b",
		"a\\b",
		"nul\0byte",
		"esc\u{1b}[31m",
	] {
		let mut out = Vec::new();
		let error =
			write_part_header(&mut out, "BOUND", &format!("build-context-{name}")).expect_err(name);
		assert!(
			error.to_string().contains("build context name"),
			"{name:?}: {error}"
		);
		assert!(out.is_empty(), "nothing is written for {name:?}");
	}
}

#[test]
fn an_ordinary_context_name_is_still_written() {
	let mut out = Vec::new();
	write_part_header(&mut out, "BOUND", "build-context-my_ctx.v2-b").unwrap();
	let header = String::from_utf8(out).unwrap();
	assert_eq!(
		header.matches("Content-Disposition").count(),
		1,
		"{header:?}"
	);
	assert!(!header.contains("X-Injected"), "{header:?}");
}
