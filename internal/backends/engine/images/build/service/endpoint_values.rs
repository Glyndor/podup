//! The platform and the isolation a build query names, checked before the
//! request is sent.

use crate::compose::types::{BuildConfig, Service};
use crate::error::{ComposeError, Result};
use crate::libpod::validate::build_field_error;

/// The isolation names the libpod build endpoint reads. Buildah, which parses
/// them, compares without regard to case.
const ISOLATIONS: [&str; 4] = ["oci", "default", "rootless", "chroot"];

/// `build.isolation` as the endpoint takes it, `None` when unset or empty.
///
/// # Errors
/// [`crate::libpod::PodmanError::Field`] for a name the endpoint does not
/// read, such as Docker's `process` and `hyperv`. The endpoint also accepts a
/// bare number, as a raw Buildah constant; podup does not.
pub(super) fn isolation(build: &BuildConfig) -> Result<Option<&str>> {
	let BuildConfig::Config {
		isolation: Some(value),
		..
	} = build
	else {
		return Ok(None);
	};
	if value.is_empty() {
		return Ok(None);
	}
	if ISOLATIONS
		.iter()
		.any(|name| name.eq_ignore_ascii_case(value))
	{
		return Ok(Some(value));
	}
	Err(ComposeError::Podman(build_field_error(
		"build.isolation",
		value.as_str(),
		format!("Podman takes {}", ISOLATIONS.join(", ")),
	)))
}

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
