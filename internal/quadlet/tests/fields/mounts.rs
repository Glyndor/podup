use crate::parse_str;
use crate::quadlet::generate_at;
use crate::quadlet::tests::unit_named;

#[test]
fn service_secret_maps_to_secret_key() {
	let yaml = r#"
services:
  s:
    image: x
    secrets:
      - tok
      - source: cred
        target: /run/cred
        uid: "100"
secrets:
  tok:
    file: ./tok
  cred:
    file: ./cred
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-s.container").contents;
	assert!(c.contains("Secret=tok"));
	assert!(c.contains("Secret=cred,target=/run/cred,uid=100"));
	assert!(!out.warnings.iter().any(|w| w.contains("secrets")));
}

#[test]
fn long_form_tmpfs_mount_renders_as_tmpfs_not_volume() {
	// A long-form `type: tmpfs` mount must become `Tmpfs=`, not `Volume=`;
	// the latter would persist it as a volume instead of an in-memory fs.
	let yaml = r#"
services:
  s:
    image: app:1.0
    volumes:
      - type: tmpfs
        target: /cache
        tmpfs:
          size: 64000000
          mode: 0o755
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-s.container").contents;
	assert!(
		c.contains("Tmpfs=/cache:size=64000000,mode=755"),
		"tmpfs not rendered as Tmpfs= with options in:\n{c}"
	);
	assert!(
		!c.contains("Volume=/cache"),
		"tmpfs wrongly emitted as a Volume in:\n{c}"
	);
}

/// #1091: `EnvironmentFile=` is resolved by podman-systemd.unit(5) against the
/// unit file's own directory, not the compose file's. Units land in
/// `~/.config/containers/systemd`, so a relative entry emitted verbatim points
/// at a file that is not there, and `--env-file` on a missing path is fatal, so
/// the container never starts. Every relative entry must come out absolute
/// against the compose base directory; an already-absolute one is untouched.
#[test]
fn env_file_entries_are_absolute_against_the_compose_base_dir() {
	let yaml = r#"
services:
  app:
    image: app:1.0
    env_file:
      - .env
      - ./config/extra.env
      - ../shared/team.env
      - /etc/glyndor/absolute.env
"#;
	let file = parse_str(yaml).unwrap();
	// A base directory that is deliberately not the process's cwd, so a bug that
	// resolved against the cwd instead would still show up here.
	let base = std::path::Path::new("/srv/app");
	let out = generate_at(&file, "p", base);
	let c = &unit_named(&out, "p-app.container").contents;
	// `abs_against` joins with the OS separator, so build the expectations the
	// same way rather than as POSIX literals; these render tests are not
	// Unix-gated and would otherwise fail on Windows.
	for rel in [".env", "config/extra.env", "../shared/team.env"] {
		let needle = format!("EnvironmentFile={}", base.join(rel).display());
		assert!(c.contains(&needle), "missing `{needle}` in:\n{c}");
	}
	// An already-absolute entry is passed through verbatim, separators included.
	assert!(
		c.contains("EnvironmentFile=/etc/glyndor/absolute.env"),
		"an absolute entry must be untouched in:\n{c}"
	);
	// No entry may survive as a compose-relative path: systemd would resolve it
	// against the unit directory. Checked by prefix rather than `is_absolute()`,
	// which on Windows demands a drive letter that a `/srv/app` base never has.
	let base_prefix = base.display().to_string();
	for line in c.lines().filter(|l| l.starts_with("EnvironmentFile=")) {
		let value = line.trim_start_matches("EnvironmentFile=");
		assert!(
			value.starts_with(&base_prefix) || value.starts_with("/etc/"),
			"`{line}` was not resolved against the compose base directory"
		);
	}
}
