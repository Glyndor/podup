//! The platform a build query names, checked before the request is sent.

use crate::compose::types::{BuildConfig, Service};
use crate::error::{ComposeError, Result};
use crate::libpod::validate::build_field_error;

/// The platform of a service's build: the one `build.platforms` lists, else the
/// service's own `platform`.
///
/// # Errors
/// [`crate::libpod::PodmanError::Field`] when that names more than one
/// platform. The libpod build endpoint gives every platform of a request the
/// same image name, so the name ends up on whichever platform finishes last,
/// and the host may not be able to run that one.
pub(super) fn platform<'a>(
	service: &'a Service,
	build: &'a BuildConfig,
) -> Result<Option<&'a str>> {
	let (field, entries): (&str, &[String]) = match build {
		BuildConfig::Config { platforms, .. } if !platforms.is_empty() => {
			("build.platforms", platforms)
		}
		_ => ("platform", service.platform.as_slice()),
	};
	// libpod splits a single `platform=` value at its commas.
	let named: Vec<&str> = entries.iter().flat_map(|e| e.split(',')).collect();
	match named.as_slice() {
		[] => Ok(None),
		// Merged compose files may list one platform twice.
		[first, rest @ ..] if rest.iter().all(|other| other == first) => Ok(Some(first)),
		_ => Err(ComposeError::Podman(build_field_error(
			field,
			named.join(", "),
			"podup builds one platform at a time: libpod would give every platform the same \
			 image name, and the name would end up on whichever finished last; name one platform",
		))),
	}
}
