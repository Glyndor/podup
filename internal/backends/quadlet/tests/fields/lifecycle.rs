use crate::parse_str;
use crate::quadlet::generate_at;
use crate::quadlet::tests::unit_named;

/// #1656: the `x-podman-autoupdate` extension reaches the Quadlet unit as
/// `AutoUpdate=<value>`, so `generate quadlet` and `autostart --mode quadlet`
/// carry it too, not just the live `up` path. Quadlet derives the
/// `io.containers.autoupdate` label itself from that key, so the
/// `Label=io.containers.autoupdate=...` line is intentionally not emitted
/// (duplicating it would mask a real override on the Quadlet side).
#[test]
fn quadlet_emits_autoupdate_and_no_duplicate_label() {
	let yaml = r#"
services:
  app:
    image: x
    x-podman-autoupdate: registry
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-app.container").contents;
	assert!(c.contains("AutoUpdate=registry"), "{c}");
	assert!(
		!c.contains("Label=io.containers.autoupdate"),
		"Quadlet sets the label itself from AutoUpdate=, so the Label= line \
		 must not be duplicated: {c}"
	);
}

/// #1656: `local` is a distinct allowed value and must reach the unit verbatim.
#[test]
fn quadlet_emits_autoupdate_local() {
	let yaml = r#"
services:
  app:
    image: x
    x-podman-autoupdate: local
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-app.container").contents;
	assert!(c.contains("AutoUpdate=local"), "{c}");
}

/// #1656: invalid values warn rather than emitting, the way
/// `x-podman-on-failure` does, generation has no error channel, and a bad
/// `AutoUpdate=` would make Quadlet drop the whole unit at daemon-reload.
#[test]
fn quadlet_warns_on_an_invalid_autoupdate_value() {
	let yaml = r#"
services:
  app:
    image: x
    x-podman-autoupdate: always
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-app.container").contents;
	assert!(!c.contains("AutoUpdate="), "must not emit a bad value: {c}");
	assert!(
		out.warnings.iter().any(|w| w.contains("always")),
		"expected a warning naming the bad value: {:?}",
		out.warnings
	);
}

#[test]
fn restart_policies_map_to_systemd() {
	let cases = [
		("no", "Restart=no"),
		("always", "Restart=always"),
		("unless-stopped", "Restart=always"),
		("on-failure", "Restart=on-failure"),
	];
	for (policy, expected) in cases {
		let yaml = format!("services:\n  s:\n    image: x\n    restart: {policy}\n");
		let file = parse_str(&yaml).unwrap();
		let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
		assert!(
			unit_named(&out, "p-s.container")
				.contents
				.contains(expected),
			"{policy} -> {expected}"
		);
	}
}

#[test]
fn optional_dependency_uses_wants_not_requires() {
	let yaml = r#"
services:
  web:
    image: nginx
    depends_on:
      cache:
        condition: service_started
        required: false
  cache:
    image: redis
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "proj", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "proj-web.container").contents;
	assert!(c.contains("After=proj-cache.service"));
	assert!(c.contains("Wants=proj-cache.service"));
	assert!(!c.contains("Requires=proj-cache.service"));
}

/// A grace period written as more than one number and unit, the way the
/// compose specification writes it, reaches the unit as seconds.
#[test]
fn a_compound_stop_grace_period_reaches_the_unit() {
	let yaml = r#"
services:
  app:
    image: x
    stop_grace_period: 1m30s
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-app.container").contents;
	assert!(c.contains("StopTimeout=90"), "{c}");
}

/// A grace period of half a second is half a second, not an instant kill: the
/// timeout is rounded up, never down to nothing.
#[test]
fn a_fractional_stop_grace_period_is_rounded_up() {
	let yaml = r#"
services:
  app:
    image: x
    stop_grace_period: 500ms
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-app.container").contents;
	assert!(c.contains("StopTimeout=1"), "{c}");
	assert!(!c.contains("StopTimeout=0"), "{c}");
}
