//! The warnings about the `build:` fields that have neither a `.build` key nor
//! a `podman build` flag, so the operator knows the build unit is incomplete
//! rather than discovering it when the build runs.
//!
//! The fields that do have a mapping are reported by the `.build` renderer
//! itself, next to the code that decides what the unit can carry.

use crate::compose::types::BuildConfig;

pub(crate) fn collect_build_warnings(name: &str, build: &BuildConfig, warnings: &mut Vec<String>) {
	let BuildConfig::Config {
		privileged,
		entitlements,
		provenance,
		sbom,
		..
	} = build
	else {
		return;
	};
	let mut warn = |field: &str, detail: &str| {
		warnings.push(format!("{name}: build.{field} {detail}"));
	};
	if *privileged == Some(true) {
		warn(
			"privileged",
			"has no Quadlet equivalent and is skipped; `podman build` has no privileged flag",
		);
	}
	if !entitlements.is_empty() {
		warn(
			"entitlements",
			"has no Quadlet equivalent and is skipped; `podman build` has no entitlements flag",
		);
	}
	if provenance
		.as_ref()
		.is_some_and(|value| value.as_bool() != Some(false))
	{
		warn(
			"provenance",
			"has no Quadlet equivalent and is skipped; `podman build` has no provenance flag",
		);
	}
	if *sbom == Some(true) {
		warn(
			"sbom",
			"has no Quadlet equivalent and is skipped; `podman build --sbom` needs a scanner \
			 preset and an output path",
		);
	}
}
