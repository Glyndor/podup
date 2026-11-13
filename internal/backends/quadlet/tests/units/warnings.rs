use crate::parse_str;
use crate::quadlet::generate_at;
use crate::quadlet::tests::unit_named;

#[test]
fn warns_for_every_unmapped_field() {
	let yaml = r#"
services:
  s:
    image: x
    network_mode: "bridge:custom"
    profiles: [debug]
    volumes_from:
      - other
    deploy:
      replicas: 3
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let joined = out.warnings.join("\n");
	for needle in ["network_mode", "profiles", "volumes_from", "scale/replicas"] {
		assert!(joined.contains(needle), "expected warning for {needle}");
	}
}

/// #1092: `env_file: [{path, required: false}]` says a missing file is fine.
/// Quadlet cannot express that: `EnvironmentFile=` becomes `--env-file`, which
/// is fatal on a missing path, so the entry is emitted anyway and the container
/// refuses to start. That is the only behaviour available; what must not happen
/// is emitting it silently.
#[test]
fn warns_when_an_optional_env_file_cannot_stay_optional() {
	let yaml = r#"
services:
  s:
    image: x
    env_file:
      - path: .env
        required: true
      - path: .env.production
        required: false
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let joined = out.warnings.join("\n");
	assert!(
		joined.contains("env_file") && joined.contains("required: false"),
		"expected a warning naming the unmappable `required: false`, got:\n{joined}"
	);
	// The entry is still emitted; dropping it would lose configuration the user
	// asked for whenever the file does exist. Built with `join` so the separator
	// matches the host (this test is not Unix-gated).
	let c = &unit_named(&out, "p-s.container").contents;
	// A backslash makes the value a quoted word with the backslash doubled.
	let shown = std::path::Path::new("/srv/app")
		.join(".env.production")
		.display()
		.to_string();
	let value = if shown.contains('\\') {
		format!("\"{}\"", shown.replace('\\', "\\\\"))
	} else {
		shown
	};
	let needle = format!("EnvironmentFile={value}");
	assert!(c.contains(&needle), "missing `{needle}` in:\n{c}");
}

/// The warning is about `required: false` specifically, so an env_file list
/// where every entry is required must stay quiet.
#[test]
fn no_env_file_warning_when_every_entry_is_required() {
	let yaml = "services:\n  s:\n    image: x\n    env_file:\n      - .env\n";
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	assert!(
		!out.warnings.iter().any(|w| w.contains("env_file")),
		"unexpected env_file warning: {:?}",
		out.warnings
	);
}

/// `podman --env-file` reads each line as written, while `up` reads the file as
/// dotenv. A file that quotes, exports, comments inline or interpolates gives
/// the container other values under Quadlet, so the keys are named (never the
/// values, which may be secrets).
#[test]
fn warns_when_podman_reads_an_env_file_differently() {
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(
		dir.path().join("app.env"),
		"Q1=\"quoted value\"\nexport EX=1\nC1=val # comment\nPLAIN=ok\n",
	)
	.unwrap();
	std::fs::write(dir.path().join("clean.env"), "PLAIN=ok\n# note\nOTHER=2\n").unwrap();
	let yaml = "services:\n  s:\n    image: x\n    env_file: [app.env, clean.env]\n";
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", dir.path());
	let joined = out.warnings.join("\n");
	assert!(joined.contains("\"app.env\""), "{joined}");
	for key in ["Q1", "EX", "C1"] {
		assert!(joined.contains(key), "{key} missing from: {joined}");
	}
	assert!(!joined.contains("quoted value"), "a value leaked: {joined}");
	assert!(!joined.contains("PLAIN"), "{joined}");
	assert!(!joined.contains("clean.env"), "{joined}");
}
