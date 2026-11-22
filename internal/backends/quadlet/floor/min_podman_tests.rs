//! Tests for the Quadlet key allowlists and the helpers the floor guards use.
//! The allowlists are what stops a key newer than the floor's Quadlet from
//! reaching a unit, so these tests pin what they accept and what they refuse.

use super::{is_at_floor, keys_in_unit, unit_type_from_filename, UnitType};

/// The keys that had to go through `PodmanArgs=` while Podman 5.0 was
/// supported are native keys of the floor's Quadlet now.
#[test]
fn keys_that_used_to_need_podman_args_are_floor_keys() {
	let cases: &[(UnitType, &str)] = &[
		(UnitType::Container, "GroupAdd"),
		(UnitType::Container, "LogOpt"),
		(UnitType::Container, "StopSignal"),
		(UnitType::Container, "NetworkAlias"),
		(UnitType::Container, "AddHost"),
		(UnitType::Container, "Memory"),
		(UnitType::Container, "AppArmor"),
		(UnitType::Container, "Umask"),
		(UnitType::Pod, "AddHost"),
		(UnitType::Pod, "Label"),
		(UnitType::Pod, "ExitPolicy"),
		(UnitType::Pod, "DNS"),
		(UnitType::Pod, "DNSOption"),
		(UnitType::Pod, "DNSSearch"),
		(UnitType::Pod, "UserNS"),
		(UnitType::Pod, "ShmSize"),
		(UnitType::Pod, "IP"),
		(UnitType::Pod, "IP6"),
		(UnitType::Pod, "NetworkAlias"),
		(UnitType::Pod, "StopTimeout"),
		(UnitType::Build, "BuildArg"),
		(UnitType::Build, "IgnoreFile"),
		(UnitType::Build, "ImageTag"),
	];
	for (unit_type, key) in cases {
		assert!(
			is_at_floor(*unit_type, key),
			"{key} on {unit_type:?} must be a key of the floor's Quadlet"
		);
	}
}

/// Measured on Podman 6.2.0 (2026-11-22): the generator rejects these as
/// unsupported keys, which is why the renderer still writes them as
/// `PodmanArgs=` flags.
#[test]
fn flags_without_a_quadlet_key_are_not_floor_keys() {
	let cases: &[(UnitType, &str)] = &[
		(UnitType::Container, "Privileged"),
		(UnitType::Container, "CPUs"),
		(UnitType::Container, "OomScoreAdj"),
		(UnitType::Pod, "Share"),
		(UnitType::Build, "NoCache"),
		(UnitType::Build, "Layers"),
		(UnitType::Build, "AddHost"),
	];
	for (unit_type, key) in cases {
		assert!(
			!is_at_floor(*unit_type, key),
			"{key} on {unit_type:?} is not a key of the floor's Quadlet"
		);
	}
}

#[test]
fn podman_args_and_global_args_exist_on_every_unit_type() {
	for unit_type in [
		UnitType::Container,
		UnitType::Pod,
		UnitType::Network,
		UnitType::Volume,
		UnitType::Build,
	] {
		for key in ["PodmanArgs", "GlobalArgs"] {
			assert!(is_at_floor(unit_type, key), "{key}= on {unit_type:?}");
		}
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
