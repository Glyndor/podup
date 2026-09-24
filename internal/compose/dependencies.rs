//! Grammar for a single `volumes_from:` entry, shared between the
//! container-name rewrite and the implicit-dependency computation.
//!
//! A second parser would drift from this one the moment one of them grew a
//! new entry shape, so the same function feeds both call sites.

/// One parsed `volumes_from:` entry. Split off the optional trailing
/// access mode so the caller can reattach it after the service-name lookup.
/// `container:<name>` is never a service, so it is a separate variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VolumesFromRef<'a> {
	/// `name[:ro|rw]` or `service:name[:ro|rw]`: a sibling service in the
	/// same compose file. The lifetime points into the raw entry string.
	Service(&'a str),
	/// `container:name[:ro|rw]`: an existing container outside the project.
	/// The engine must look up an actual container by this name at run time.
	Container(&'a str),
}

/// Trailing `:ro`/`:rw` on a `volumes_from:` entry, kept aside so the
/// service-name rewrite below does not drop it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VolumesFromMode {
	/// No trailing mode; default behaviour.
	Default,
	/// Explicit `:ro` (read-only) or `:rw` (read-write) suffix.
	Explicit(&'static str),
}

/// Split a single `volumes_from:` entry into the referent and its optional
/// `:ro`/`:rw` access mode. `container:<name>` is recognised as a
/// non-service reference (a container outside the project) and returned in
/// [`VolumesFromRef::Container`]; the bare `<name>`, `service:<name>` and
/// `service:<name>:ro` forms are returned in [`VolumesFromRef::Service`].
/// The grammar is the one
/// [`crate::engine::container::resolve::resolve_volumes_from`] already
/// accepts; this helper exists so the same parser feeds both the
/// container-name rewrite and any future consumer, and a future shape
/// change cannot drift between them.
pub(crate) fn parse_volumes_from_entry(entry: &str) -> (VolumesFromRef<'_>, VolumesFromMode) {
	let (reference, mode) = match entry.rsplit_once(':') {
		Some((head, "ro")) => (head, VolumesFromMode::Explicit("ro")),
		Some((head, "rw")) => (head, VolumesFromMode::Explicit("rw")),
		_ => (entry, VolumesFromMode::Default),
	};
	let reference = if let Some(name) = reference.strip_prefix("container:") {
		VolumesFromRef::Container(name)
	} else {
		let target = reference.strip_prefix("service:").unwrap_or(reference);
		VolumesFromRef::Service(target)
	};
	(reference, mode)
}
