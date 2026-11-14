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

/// The warnings of `generate_at` for one service whose files are in `dir`.
fn env_warnings(dir: &std::path::Path, service: &str) -> String {
	let yaml = format!("services:\n  s:\n    image: x\n{service}");
	let file = parse_str(&yaml).unwrap();
	generate_at(&file, "p", dir).warnings.join("\n")
}

/// A line inside a multi-line value is read by `podman --env-file` as a key of
/// its own; naming it would print part of the value. Only the keys `up` reads
/// are named.
#[test]
fn a_multi_line_value_is_never_named_in_the_warning() {
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(
		dir.path().join("app.env"),
		"TOKEN=\"first\nS3CR3T_FRAGMENT=tail\"\n",
	)
	.unwrap();
	let joined = env_warnings(dir.path(), "    env_file: app.env\n");
	assert!(joined.contains("TOKEN"), "{joined}");
	assert!(
		!joined.contains("S3CR3T_FRAGMENT"),
		"part of a value leaked: {joined}"
	);
}

/// A FIFO as an env file does not hang the export.
#[cfg(unix)]
#[test]
fn a_fifo_env_file_does_not_hang_the_export() {
	let dir = tempfile::tempdir().unwrap();
	let fifo = dir.path().join("app.env");
	let status = std::process::Command::new("mkfifo")
		.arg(&fifo)
		.status()
		.unwrap();
	assert!(status.success());
	let (tx, rx) = std::sync::mpsc::channel();
	let path = dir.path().to_path_buf();
	std::thread::spawn(move || {
		let _ = tx.send(env_warnings(&path, "    env_file: app.env\n"));
	});
	rx.recv_timeout(std::time::Duration::from_secs(10))
		.expect("generate blocked on the FIFO");
}

/// The comparison resolves a value the way `up` does, with the service's
/// `environment:` and the files before it in scope.
#[test]
fn the_check_sees_the_service_environment_and_earlier_files() {
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(
		dir.path().join("app.env"),
		"TOKEN=${PODUP_SRC_7Q:?required}\n",
	)
	.unwrap();
	std::fs::write(dir.path().join("first.env"), "PODUP_SRC_7Q=present\n").unwrap();
	let from_env = env_warnings(
		dir.path(),
		"    env_file: app.env\n    environment:\n      PODUP_SRC_7Q: present\n",
	);
	assert!(from_env.contains("TOKEN"), "{from_env}");
	let from_file = env_warnings(dir.path(), "    env_file: [first.env, app.env]\n");
	assert!(from_file.contains("TOKEN"), "{from_file}");
}

/// A key the service's `environment:` or a later file sets again never reaches
/// the container from this file, so it is not named.
#[test]
fn an_overridden_key_is_not_named() {
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(dir.path().join("app.env"), "TOKEN=\"quoted\"\n").unwrap();
	std::fs::write(dir.path().join("later.env"), "TOKEN=safe\n").unwrap();
	let by_env = env_warnings(
		dir.path(),
		"    env_file: app.env\n    environment:\n      TOKEN: safe\n",
	);
	assert!(!by_env.contains("TOKEN"), "{by_env}");
	let by_file = env_warnings(dir.path(), "    env_file: [app.env, later.env]\n");
	assert!(!by_file.contains("TOKEN"), "{by_file}");
}
