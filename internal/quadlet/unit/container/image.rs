//! Which image a container runs and how that image is obtained.

use crate::compose::types::Service;

use super::super::build::{
	build_image_tag, build_unit_filename, emits_build_unit, qualify_local_image_tag,
};
use super::{ContainerUnitMode, Section};

/// Emit the `Image=` line for the service: the sibling `.build` unit's filename,
/// the tag that build unit registers, or the service's own `image:`.
pub(super) fn emit_image(
	project: &str,
	name: &str,
	service: &Service,
	mode: ContainerUnitMode,
	container: &mut Section,
) {
	// A service with a buildable `build:` references its `.build` unit, so Quadlet
	// builds the image before running; otherwise the explicit `image:` is used.
	// In prebuilt mode the build runs once at install, so the container must
	// point at the tag the `.build` unit registers (the same value it stamps
	// as `ImageTag=`, resolved by `build_image_tag`), and Quadlet must NOT see
	// a reference to the `.build` filename, or it adds the `Requires=`/`After=`
	// dependency on the build service that re-runs it at every start and boot.
	match mode {
		ContainerUnitMode::Standard => {
			if emits_build_unit(service) {
				container.add("Image", build_unit_filename(project, name));
			} else if let Some(image) = &service.image {
				container.add("Image", image.clone());
			}
		}
		ContainerUnitMode::Prebuilt => {
			if emits_build_unit(service) {
				// The build step stored `localhost/<tag>` for a short tag and
				// `<registry>/<tag>` for a fully qualified one; pin the
				// container at that exact reference (`Pull=never` is what
				// stops a registry race). See qualify_local_image_tag.
				container.add(
					"Image",
					qualify_local_image_tag(&build_image_tag(service, project, name)),
				);
			} else if let Some(image) = &service.image {
				container.add("Image", image.clone());
			}
		}
	}
}

/// Emit the `Pull=` line, and pin the container to the locally built image in
/// prebuilt mode.
pub(super) fn emit_pull(service: &Service, mode: ContainerUnitMode, container: &mut Section) {
	if let Some(pull) = &service.pull_policy {
		// In prebuilt mode a buildable service already gets `Pull=never` below;
		// emitting the service's own `pull_policy:` would produce a second
		// `Pull=` line, leaving which one wins up to Quadlet. The whole
		// point of prebuilt is to use the locally built image only, so the
		// user's pull request is intentionally overridden.
		if !(mode == ContainerUnitMode::Prebuilt && emits_build_unit(service)) {
			container.add("Pull", pull.clone());
		}
	}
	// In prebuilt mode, pin the container to the locally built image: the
	// sibling `.build` unit writes that tag at install, and `Pull=never`
	// stops Quadlet from racing the registry against it on every start.
	if mode == ContainerUnitMode::Prebuilt && emits_build_unit(service) {
		container.add("Pull", "never".to_string());
	}
}
