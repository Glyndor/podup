//! The short-form volume split, reachable by the fuzz harness.

/// What a short-form entry became: `Some((true, source, destination))` for a
/// host bind, `Some((false, name, destination))` for a named or anonymous volume.
pub(crate) fn classify_short(entry: &str) -> Option<(bool, String, String)> {
	let (mount, named) = super::spec::parse_volume_string(entry)?;
	match (mount, named) {
		(Some(m), None) => Some((true, m.source.unwrap_or_default(), m.destination)),
		(None, Some(v)) => Some((false, v.name, v.dest)),
		_ => None,
	}
}
