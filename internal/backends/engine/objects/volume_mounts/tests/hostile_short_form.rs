//! Short-form volume strings that try to turn a container path into a host bind.
use super::super::spec::parse_volume_string;

/// What one entry became: a host bind (`bind`) or a volume (`volume`).
fn classify(s: &str) -> (Option<String>, Option<String>) {
	let (mount, named) = parse_volume_string(s).expect("every string parses to one of the two");
	assert!(
		mount.is_some() != named.is_some(),
		"{s:?} must become exactly one of a mount and a volume"
	);
	(
		mount.map(|m| m.source.unwrap_or_default()),
		named.map(|n| n.dest),
	)
}

/// With no separator the entry is a container path and never a host bind, even
/// when it starts like a host path. `/data` read as a bind would mount the
/// host's own `/data` into the container.
#[test]
fn an_entry_without_a_separator_is_never_a_bind() {
	for s in [
		"/data",
		"cache",
		"./rel",
		"~/home",
		"",
		"\0",
		"/etc/passwd",
		"..",
		"\u{202e}/x",
	] {
		let (bind, volume) = classify(s);
		assert!(bind.is_none(), "{s:?} must not become a host bind");
		assert_eq!(
			volume.as_deref(),
			Some(s),
			"{s:?} is the destination of an anonymous volume"
		);
	}
}

/// Control: the same host paths with a destination are binds.
#[test]
fn a_host_path_with_a_destination_is_a_bind() {
	for (s, src) in [
		("/data:/data", "/data"),
		("./a:/data:ro", "./a"),
		("~/h:/data", "~/h"),
		("C:\\tmp:/data:ro", "C:\\tmp"),
		("C:/tmp:/data:ro", "C:/tmp"),
	] {
		let (bind, volume) = classify(s);
		assert_eq!(bind.as_deref(), Some(src), "{s:?} source");
		assert!(volume.is_none(), "{s:?} is not a named volume");
	}
}

/// A bare name before the colon is a named volume, whatever follows.
#[test]
fn a_bare_name_with_a_destination_is_a_named_volume() {
	for s in ["cache:/data", "cache:/data:ro", "a-b_c.d:/data"] {
		let (bind, volume) = classify(s);
		assert!(bind.is_none(), "{s:?} must not become a host bind");
		assert_eq!(volume.as_deref(), Some("/data"), "{s:?} destination");
	}
}

/// Empty options between commas are dropped, not forwarded as blank flags.
#[test]
fn empty_options_are_dropped() {
	let (mount, _) = parse_volume_string("/h:/d:,,ro,,").unwrap();
	assert_eq!(mount.unwrap().options, vec!["ro"]);
}
