//! The unit creates the bind sources `up` would create before the container
//! starts, with `ExecStartPre=-mkdir -p`: the `-` keeps a failure (a source
//! that is already a file) from stopping the unit.

use crate::parse_str;
use crate::quadlet::generate_at;
use crate::quadlet::tests::unit_named;

/// `up` creates a missing bind source (`create_host_path`); without it the unit
/// fails at `statfs`. The unit creates it before the container starts. A path
/// with characters a command line cannot carry safely (here `$` and `%`) is
/// left out and reported, and a long-form bind with `create_host_path: false`
/// is left alone.
#[cfg(unix)]
#[test]
fn bind_sources_are_created_before_the_container_starts() {
	let yaml = "services:\n  s:\n    image: x\n    volumes:\n      - ./cache:/cache\n      - \"/srv/a b:/spaced\"\n      - \"/srv/odd$$x%y:/odd\"\n      - {type: bind, source: /srv/given, target: /g, bind: {create_host_path: false}}\n      - named:/n\nvolumes:\n  named: {}\n";
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-s.container").contents;
	assert!(
		c.contains("ExecStartPre=-mkdir -p -- \"/srv/app/./cache\"\n"),
		"{c}"
	);
	assert!(
		c.contains("ExecStartPre=-mkdir -p -- \"/srv/a b\"\n"),
		"{c}"
	);
	assert_eq!(c.matches("ExecStartPre=").count(), 2, "{c}");
	assert!(
		out.warnings
			.iter()
			.any(|w| w.contains("/srv/odd") && w.contains("create it")),
		"{:?}",
		out.warnings
	);
}
