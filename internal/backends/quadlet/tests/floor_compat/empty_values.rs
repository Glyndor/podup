//! Empty-value handling for the native Quadlet keys the renderer writes from
//! lists and optional fields.
//!
//! A bare `Key=` line is not "no value" for Quadlet: it resets the list of
//! that key, and `podman run --flag=` fails with `error parsing additional
//! short argument`. Each site that emits a key from a list or optional field
//! skips empty values, and has its own test here so a regression that drops
//! the skip on one but not another shows up individually.

use crate::parse_str;
use crate::quadlet::generate_at;
use crate::quadlet::tests::unit_named;

/// An empty `stop_signal:` (`""`) would render a bare `StopSignal=`; it emits
/// nothing.
#[test]
fn empty_stop_signal_emits_no_flag() {
	let yaml = r#"
services:
  s:
    image: x
    stop_signal: ""
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-s.container").contents;
	assert!(
		!c.contains("StopSignal"),
		"empty stop_signal must not emit StopSignal=; got:\n{c}"
	);
}

/// An empty entry in `group_add:` (`["audio", ""]`) would render a bare
/// `GroupAdd=`, which resets the list. The non-empty entry still emits.
#[test]
fn empty_group_add_entry_emits_no_flag() {
	let yaml = r#"
services:
  s:
    image: x
    group_add:
      - audio
      - ""
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-s.container").contents;
	assert!(
		!c.contains("GroupAdd=\n"),
		"empty group_add entry must not emit a bare GroupAdd=; got:\n{c}"
	);
	assert!(
		c.contains("GroupAdd=audio\n"),
		"non-empty group_add entry must still emit; got:\n{c}"
	);
}

/// An empty entry in `extra_hosts:` (`["db:10.0.0.2", ""]`) would render a
/// bare `AddHost=`, which resets the list. The non-empty entry still emits.
#[test]
fn empty_extra_hosts_entry_emits_no_flag() {
	let yaml = r#"
services:
  s:
    image: x
    extra_hosts:
      - "db:10.0.0.2"
      - ""
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-s.container").contents;
	assert!(
		!c.contains("AddHost=\n"),
		"empty extra_hosts entry must not emit a bare AddHost=; got:\n{c}"
	);
	assert!(
		c.contains("AddHost=db:10.0.0.2\n"),
		"non-empty extra_hosts entry must still emit; got:\n{c}"
	);
}

/// An empty network alias (`aliases: ["web", ""]`) is left out of the
/// `Network=` line; the non-empty alias still emits.
#[test]
fn empty_network_alias_emits_no_flag() {
	let yaml = r#"
services:
  s:
    image: x
    networks:
      front:
        aliases:
          - web
          - ""
networks:
  front:
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-s.container").contents;
	assert!(
		c.contains("Network=p-front.network:alias=s,alias=web\n"),
		"the empty alias must be left out and the others kept; got:\n{c}"
	);
}

/// An empty value in `logging.options:` (`{ env: "" }`) would render
/// `LogOpt=env=`, which podman rejects. The non-empty option still emits.
#[test]
fn empty_logging_option_value_emits_no_flag() {
	let yaml = r#"
services:
  s:
    image: x
    logging:
      driver: k8s-file
      options:
        env: ""
        path: /var/log/x.log
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-s.container").contents;
	assert!(
		!c.contains("LogOpt=env="),
		"empty logging option value must not emit LogOpt=env=; got:\n{c}"
	);
	assert!(
		c.contains("LogOpt=path=/var/log/x.log\n"),
		"non-empty logging option value must still emit; got:\n{c}"
	);
}
