//! Empty-value handling for the `PodmanArgs=` escape-hatch flags added in
//! round one.
//!
//! podup renders post-5.0 container keys through `PodmanArgs=` flags
//! rather than the native Quadlet keys (the native keys do not exist in
//! 5.0). The native Quadlet keys silently drop empty values; the
//! podman-args route has to skip them too, since `podman run --flag=`
//! fails with `error parsing additional short argument`.
//!
//! Split out into its own file to keep `floor_compat.rs` under the
//! per-file code-line cap. Each site that emits a `PodmanArgs=` flag
//! from a list or optional field has its own test here so a regression
//! that re-introduces empty handling on one but not another shows up
//! individually.

use super::unit_named;
use crate::parse_str;
use crate::quadlet::generate_at;

/// An empty `stop_signal:` (`""`) renders to a `--stop-signal=` flag, which
/// podman rejects with `error parsing additional short argument`. The native
/// `StopSignal=` Quadlet key omits empty values; the podman-args route must
/// skip them too.
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
		!c.contains("--stop-signal"),
		"empty stop_signal must not emit --stop-signal=; got:\n{c}"
	);
}

/// An empty entry in `group_add:` (`["audio", ""]`) would render a
/// `--group-add=` flag podman rejects. The native `GroupAdd=` key omits
/// empty values; the podman-args route must skip them too. The non-empty
/// entry still emits.
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
		!c.contains("--group-add=\"\""),
		"empty group_add entry must not emit --group-add=; got:\n{c}"
	);
	assert!(
		c.contains("PodmanArgs=--group-add=\"audio\""),
		"non-empty group_add entry must still emit; got:\n{c}"
	);
}

/// An empty entry in `extra_hosts:` (`["db:10.0.0.2", ""]`) would render a
/// `--add-host=` flag podman rejects. The native `AddHost=` key omits empty
/// values; the podman-args route must skip them too. The non-empty entry
/// still emits.
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
		!c.contains("--add-host=\"\""),
		"empty extra_hosts entry must not emit --add-host=; got:\n{c}"
	);
	assert!(
		c.contains("PodmanArgs=--add-host=\"db:10.0.0.2\""),
		"non-empty extra_hosts entry must still emit; got:\n{c}"
	);
}

/// An empty network alias (`aliases: ["web", ""]`) would render a
/// `--network-alias=` flag podman rejects. The native `NetworkAlias=` key
/// omits empty values; the podman-args route must skip them too. The
/// non-empty alias still emits.
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
		!c.contains("--network-alias=\"\""),
		"empty network alias must not emit --network-alias=; got:\n{c}"
	);
	assert!(
		c.contains("PodmanArgs=--network-alias=\"web\""),
		"non-empty network alias must still emit; got:\n{c}"
	);
}

/// An empty value in `logging.options:` (`{ tag: "" }`) would render a
/// `--log-opt=tag=` flag podman rejects. The native `LogOpt=` key omits
/// empty values; the podman-args route must skip them too. The non-empty
/// option still emits.
#[test]
fn empty_logging_option_value_emits_no_flag() {
	let yaml = r#"
services:
  s:
    image: x
    logging:
      driver: journald
      options:
        tag: ""
        path: /var/log/x.log
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-s.container").contents;
	assert!(
		!c.contains("--log-opt=\"tag=\""),
		"empty logging option value must not emit --log-opt=tag=; got:\n{c}"
	);
	assert!(
		c.contains("PodmanArgs=--log-opt=\"path=/var/log/x.log\""),
		"non-empty logging option value must still emit; got:\n{c}"
	);
}
