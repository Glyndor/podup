//! The platform and the isolation a build query names, checked before the
//! request is built.

use crate::compose::types::{BuildConfig, Service};
use crate::error::{ComposeError, Result};
use crate::libpod::validate::build_field_error;

/// The isolation names the libpod build endpoint reads. Buildah, which parses
/// them, compares without regard to case.
const ISOLATIONS: [&str; 4] = ["oci", "default", "rootless", "chroot"];

/// The platform and the isolation of a service's build, as the endpoint takes
/// them. The platform is the one `build.platforms` lists, else the service's
/// own `platform`; an empty `build.isolation` is unset.
///
/// # Errors
/// [`crate::libpod::PodmanError::Field`] when the platform names more than
/// one: the libpod build endpoint gives every platform of a request the same
/// image name, so the name ends up on whichever platform finishes last, and
/// the host may not be able to run that one. Also for an isolation the
/// endpoint does not read, such as Docker's `process` and `hyperv`. The
/// endpoint takes a bare number too, as a raw Buildah constant; podup does not.
pub(super) fn checked<'a>(
	service: &'a Service,
	build: &'a BuildConfig,
) -> Result<(Option<&'a str>, Option<&'a str>)> {
	let (mut field, mut entries) = ("platform", service.platform.as_slice());
	let mut isolation = None;
	if let BuildConfig::Config {
		platforms,
		isolation: named,
		..
	} = build
	{
		if !platforms.is_empty() {
			(field, entries) = ("build.platforms", platforms);
		}
		isolation = named.as_deref().filter(|value| !value.is_empty());
	}
	let platform = entries.first();
	// libpod splits a single `platform=` value at its commas. Merged compose
	// files may list one platform twice.
	let (field, value, message) = if entries
		.iter()
		.any(|entry| entry.contains(',') || Some(entry) != platform)
	{
		let message = "libpod would give every platform the same image name; name one platform";
		(field, entries.join(","), message)
	} else if let Some(value) = isolation.filter(|value| {
		!ISOLATIONS
			.iter()
			.any(|name| name.eq_ignore_ascii_case(value))
	}) {
		let message = "Podman takes oci, default, rootless or chroot";
		("build.isolation", value.to_owned(), message)
	} else {
		return Ok((platform.map(String::as_str), isolation));
	};
	Err(ComposeError::Podman(build_field_error(
		field,
		value.as_str(),
		message.to_string(),
	)))
}
