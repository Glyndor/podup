use super::*;

#[test]
fn volume_name_resolution() {
	let f = parse_str(
		"services:\n  s:\n    image: x\nvolumes:\n  data:\n  ext:\n    external: true\n  custom:\n    name: my-vol\n",
	)
	.unwrap();
	assert_eq!(
		resolve_volume_name("data", "proj", &f).unwrap(),
		"proj_data"
	);
	assert_eq!(resolve_volume_name("ext", "proj", &f).unwrap(), "ext");
	assert_eq!(resolve_volume_name("custom", "proj", &f).unwrap(), "my-vol");
	// An empty reference is an anonymous volume: no name to resolve.
	assert_eq!(resolve_volume_name("", "proj", &f).unwrap(), "");
	// A non-empty reference not declared under top-level `volumes:` is
	// rejected rather than escaping into a bare, unprefixed global volume.
	let err = resolve_volume_name("missing", "proj", &f).unwrap_err();
	assert!(err.to_string().contains("missing"), "got: {err}");
}
