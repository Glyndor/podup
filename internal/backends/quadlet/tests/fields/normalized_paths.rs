//! Bind sources reach the unit normalized, as systemd needs them for
//! `RequiresMountsFor=`.

use crate::parse_str;
use crate::quadlet::generate_at;
use crate::quadlet::tests::unit_named;

/// Quadlet turns a `Volume=` source into `RequiresMountsFor=`, and systemd
/// ignores one that is not normalized ("path is not normalized, ignoring"), so
/// `./data` lost the mount ordering. The `.` is dropped; `..` is kept, because
/// through a symlink it does not name the directory read lexically.
#[cfg(unix)]
#[test]
fn a_bind_source_carries_no_current_dir_component() {
	let yaml = "services:\n  s:\n    image: x\n    volumes:\n      - ./data:/d\n      - {type: bind, source: ./cache, target: /c}\n      - ../up:/u\n";
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-s.container").contents;
	assert!(c.contains("Volume=/srv/app/data:/d\n"), "{c}");
	assert!(c.contains("Volume=/srv/app/cache:/c\n"), "{c}");
	assert!(c.contains("Volume=/srv/app/../up:/u\n"), "{c}");
}
