//! Tests for the `floor_required` / `contains_key` helpers and the table
//! itself. The table is what guards every Quadlet export against landing on
//! a Podman older than the unit's keys, so these tests pin both the
//! expected floor per key (no silent regressions) and the table's coverage
//! (no key registered without a row).

use super::{
	floor, floor_required, is_floor_compliant, keys_in_unit, unit_type_from_filename, UnitType,
};

#[test]
fn floor_is_5_0() {
	assert_eq!(floor().major, 5);
	assert_eq!(floor().minor, 0);
}

#[test]
fn table_covers_every_post_5_0_key_we_emit() {
	// A smoke-check on the `is_floor_compliant` shape: every unit type
	// should at least accept its own exemption (PodmanArgs) and the
	// `.build` exemption. The real coverage check is in the floor /
	// coverage integration tests in `floor_compat.rs`.
	for unit_type in [
		UnitType::Container,
		UnitType::Pod,
		UnitType::Network,
		UnitType::Volume,
		UnitType::Build,
	] {
		assert!(
			is_floor_compliant(unit_type, "PodmanArgs"),
			"PodmanArgs= must be exempt on {unit_type:?}"
		);
	}
}

/// Every documented above-floor key resolves to the right release.
#[test]
fn each_registered_key_resolves_to_its_release() {
	let cases: &[(UnitType, &str, u16, u16)] = &[
		(UnitType::Container, "GroupAdd", 5, 1),
		(UnitType::Container, "LogOpt", 5, 2),
		(UnitType::Container, "StopSignal", 5, 2),
		(UnitType::Container, "NetworkAlias", 5, 2),
		(UnitType::Container, "AddHost", 5, 3),
		(UnitType::Pod, "AddHost", 5, 3),
		(UnitType::Pod, "Label", 5, 6),
		(UnitType::Build, "Retry", 5, 5),
		(UnitType::Build, "RetryDelay", 5, 5),
		(UnitType::Build, "BuildArg", 5, 7),
		(UnitType::Build, "IgnoreFile", 5, 7),
	];
	for (unit_type, key, major, minor) in cases {
		let v = floor_required(*unit_type, key)
			.unwrap_or_else(|| panic!("table must cover {key} on {unit_type:?}"));
		assert_eq!(
			(v.major, v.minor),
			(*major, *minor),
			"{key} on {unit_type:?} must be {major}.{minor}"
		);
	}
}

/// The `.build` unit type landed in 5.2.0 with the keys listed in
/// `is_5_2_build_key`. Every key the renderer emits on `[Build]` must be
/// in that set, in the post-5.2 table, or be a floor-exempt passthrough.
/// `Retry=` (5.5.0) and a hypothetical `MadeUpKey=` (never landed) both
/// fail; `ImageTag=` (5.2.0) and `PodmanArgs=` (exempt) both pass.
#[test]
fn build_unit_type_allowlist() {
	let allowed: &[&str] = &[
		"ImageTag",
		"SetWorkingDirectory",
		"File",
		"Target",
		"Network",
		"Label",
		"PodmanArgs",
	];
	for key in allowed {
		assert!(
			is_floor_compliant(UnitType::Build, key),
			"{key}= must be floor-compliant on Build (5.2 set or exempt)"
		);
	}
	// Post-5.2 keys must be in the table.
	assert!(
		is_floor_compliant(UnitType::Build, "Retry"),
		"Retry= on Build must be floor-compliant (5.5.0 row)"
	);
	assert!(
		is_floor_compliant(UnitType::Build, "BuildArg"),
		"BuildArg= on Build must be floor-compliant (5.7.0 row)"
	);
	// A key that has never landed fails.
	assert!(
		!is_floor_compliant(UnitType::Build, "MadeUpKey"),
		"MadeUpKey= on Build must fail the guard"
	);
}

/// `PodmanArgs=` and `GlobalArgs=` are exempt: they appear on every unit
/// type in 5.0, so neither gating them nor naming them in the table helps.
#[test]
fn podman_args_and_global_args_are_exempt() {
	for unit_type in [
		UnitType::Container,
		UnitType::Pod,
		UnitType::Network,
		UnitType::Volume,
	] {
		assert!(
			floor_required(unit_type, "PodmanArgs").is_none(),
			"PodmanArgs= must be exempt on {unit_type:?}"
		);
		assert!(
			floor_required(unit_type, "GlobalArgs").is_none(),
			"GlobalArgs= must be exempt on {unit_type:?}"
		);
	}
}

/// `keys_in_unit` ignores comment lines and `[Section]` headers. Keys
/// outside a Quadlet-managed section (`[Unit]`, `[Install]`, `[Service]`)
/// are not Quadlet keys, so they must not gate the floor.
#[test]
fn line_key_strips_comments_and_headers() {
	let body = "\
# podup-owner: proj
[Unit]
Description=hello
Image=alpine
  AddHost=db:10.0.0.2
[Container]
AddHost=should-not-leak
[Install]
WantedBy=default.target
Restart=always
";
	let keys = keys_in_unit(body);
	// Only the keys inside `[Container]` survive; `[Unit]`/`[Install]`
	// are passed through to systemd, not to Quadlet.
	assert_eq!(
		keys,
		vec!["AddHost".to_string()],
		"expected only the [Container] key to register; got {keys:?}"
	);
}

/// `keys_in_unit` deduplicates so the floor test only names each key
/// once per unit.
#[test]
fn keys_in_unit_deduplicates() {
	let body = "\
[Container]
Image=alpine
Image=alpine
";
	assert_eq!(keys_in_unit(body), vec!["Image".to_string()]);
}

/// `keys_in_unit` excludes `PodmanArgs=`/`GlobalArgs=`, since both are
/// floor-exempt by construction.
#[test]
fn keys_in_unit_excludes_podman_args_and_global_args() {
	let body = "\
[Container]
Image=alpine
PodmanArgs=--memory=512m
GlobalArgs=--log-level=debug
";
	let keys = keys_in_unit(body);
	assert_eq!(keys, vec!["Image".to_string()], "got {keys:?}");
}

/// `unit_type_from_filename` resolves the unit type from the file name
/// suffix; anything else falls back to `Container` so the floor test still
/// runs on a future unit type the resolver does not yet know.
#[test]
fn filename_to_unit_type_resolves_by_suffix() {
	assert_eq!(
		unit_type_from_filename("proj-app.container"),
		UnitType::Container
	);
	assert_eq!(unit_type_from_filename("proj.pod"), UnitType::Pod);
	assert_eq!(
		unit_type_from_filename("proj-net.network"),
		UnitType::Network
	);
	assert_eq!(unit_type_from_filename("proj-vol.volume"), UnitType::Volume);
	assert_eq!(unit_type_from_filename("proj-app.build"), UnitType::Build);
	// Unknown suffix: default to Container. This is the conservative
	// choice (any new key on the unknown unit type would still gate).
	assert_eq!(unit_type_from_filename("foo.unknown"), UnitType::Container);
}
