//! `build.cache_from` and `build.cache_to` as the libpod endpoint reads them.
//!
//! The endpoint takes a JSON array of image references: the caches it can
//! use are the ones a registry holds. Compose writes the same caches with
//! the global `[NAME|type=TYPE[,KEY=VALUE]]` syntax, where a bare name is
//! the shortcut for `type=registry,ref=NAME`.
//!
//! A cache type with no libpod equivalent is dropped, as the Compose
//! contract asks: unsupported caches are ignored and do not prevent the
//! build.

use tracing::warn;

/// Which compose field a cache entry came from.
pub(in crate::backends::engine::images::build) enum CacheField {
	/// `build.cache_from`: the caches the build reads from.
	From,
	/// `build.cache_to`: the caches the build exports to.
	To,
}

/// The image references the endpoint can use for the given cache entries.
///
/// Entries whose cache type has no libpod equivalent are skipped and
/// reported, so the build still runs without them.
pub(in crate::backends::engine::images::build) fn references(
	field: CacheField,
	entries: &[String],
) -> Vec<String> {
	let mut out = Vec::new();
	for entry in entries {
		match image_reference(entry) {
			Some(reference) => out.push(reference),
			None => match field {
				CacheField::From => warn!(
					"a build.cache_from entry uses a cache type with no libpod equivalent; \
					 ignoring it"
				),
				CacheField::To => warn!(
					"a build.cache_to entry uses a cache type with no libpod equivalent; \
					 ignoring it"
				),
			},
		}
	}
	out
}

/// The image reference one compose cache entry names, when it names one.
///
/// A bare name is the shortcut for `type=registry,ref=NAME` and travels as
/// it is. A `type=registry` entry travels as its `ref`. Every other cache
/// type has no libpod equivalent.
fn image_reference(entry: &str) -> Option<String> {
	let trimmed = entry.trim();
	if trimmed.is_empty() {
		return None;
	}
	let Some(kind) = trimmed.strip_prefix("type=") else {
		return Some(trimmed.to_string());
	};
	if kind.split(',').next()?.trim() != "registry" {
		return None;
	}
	trimmed
		.split(',')
		.filter_map(|part| part.split_once('='))
		.find(|(key, _)| key.trim() == "ref")
		.map(|(_, value)| value.trim().to_string())
		.filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests;
