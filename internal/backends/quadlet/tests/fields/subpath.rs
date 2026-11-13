//! A volume with `volume.subpath` is written as `Mount=`, which `--mount`
//! reads; `Volume=` (`podman -v`) has no subpath option.

use crate::parse_str;
use crate::quadlet::generate_at;
use crate::quadlet::tests::unit_named;

/// `Volume=` becomes `podman -v`, which has no subpath option, so the whole
/// volume was mounted instead of the folder `up` mounts. A volume with a
/// `subpath` goes out as `Mount=`, which `--mount` reads, hardening flags kept.
#[test]
fn a_volume_subpath_is_written_as_a_mount() {
	let yaml = "services:\n  s:\n    image: x\n    volumes:\n      - {type: volume, source: vol, target: /sub, read_only: true, volume: {subpath: inner, noexec: true}}\n      - {type: volume, source: vol, target: /all}\nvolumes: {vol: {}}\n";
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-s.container").contents;
	assert!(
		c.contains(
			"Mount=type=volume,source=p-vol.volume,destination=/sub,subpath=inner,ro,noexec\n"
		),
		"{c}"
	);
	assert!(c.contains("Volume=p-vol.volume:/all\n"), "{c}");
	assert!(!c.contains("Volume=p-vol.volume:/sub"), "{c}");
}

/// A subpath that would break the option list falls back to `Volume=`.
#[test]
fn a_subpath_with_a_comma_is_not_written_as_a_mount_option() {
	let yaml = "services:\n  s:\n    image: x\n    volumes:\n      - {type: volume, source: vol, target: /sub, volume: {subpath: \"a,ro=false\"}}\nvolumes: {vol: {}}\n";
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-s.container").contents;
	assert!(!c.contains("Mount="), "{c}");
}
