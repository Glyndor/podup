//! `_FILE` suffix: the documented convention for keeping a secret out of
//! the environment. The value is a path the application reads at runtime;
//! the path is not the secret. The exemption is the `_FILE` suffix AND a
//! value that reads as a path (starts with `/`, `./`, or `../`). A
//! `_FILE` key whose value does not start with one of those three
//! prefixes falls through to the flagging rule, and `--strict` exits
//! non-zero: the suffix alone is not an exemption.
//!
//! Split out from `tests/audit_exit_codes.rs` so the parent file stays
//! under the repository line limit; the helpers (`run`, `write_compose`)
//! are inherited via `use super::*`.

use super::*;

#[test]
fn audit_secret_in_environment_does_not_flag_file_suffix_pointing_at_secrets_mount() {
	// The reporter's exact shape: a secret-bearing key with the `_FILE`
	// convention pointing at `/run/secrets/<name>`. The service is
	// hardened on every other axis the audit checks so the only
	// finding the gate could raise is `secret_in_environment`. `--strict`
	// must stay green, otherwise `podup audit --strict` carves out this
	// finding class by hand in every consumer that uses the official
	// Postgres, MariaDB, or MySQL image conventions.
	let body = r#"services:
  db:
    image: postgres:16@sha256:0e7bb5afc7e5e22ee46c4f2cd4a8b3fa63ad3f5d5e5e5e5e5e5e5e5e5e5e5e5e5e
    read_only: true
    cap_drop: [ALL]
    security_opt: [no-new-privileges:true]
    pids_limit: 200
    mem_limit: 512m
    memswap_limit: 512m
    init: true
    restart: unless-stopped
    cpus: "1"
    healthcheck:
      test: ["CMD", "true"]
      x-podman-on-failure: restart
    userns_mode: auto
    environment:
      - POSTGRES_PASSWORD_FILE=/run/secrets/pg
"#;
	let path = write_compose(body);
	let p = path.to_str().unwrap();
	let out = run(&["-f", p, "audit", "--strict"]);
	assert!(
		out.status.success(),
		"POSTGRES_PASSWORD_FILE=/run/secrets/pg must pass --strict; got {:?}\nstderr: {}\nstdout: {}",
		out.status.code(),
		String::from_utf8_lossy(&out.stderr),
		String::from_utf8_lossy(&out.stdout),
	);
	let stdout = String::from_utf8_lossy(&out.stdout);
	assert!(
		!stdout.contains("secret_in_environment"),
		"POSTGRES_PASSWORD_FILE must not fire secret_in_environment: {stdout}"
	);
}

#[test]
fn audit_secret_in_environment_does_not_flag_file_suffix_pointing_outside_secrets_mount() {
	// The exemption is the path-shape test, not the `/run/secrets/`
	// prefix. A `_FILE` key pointing at any absolute path is still a
	// path, not a secret in the environment, so the check stays
	// silent. Verifying what the path points to (file permissions,
	// mount provenance) is a different audit concern and is out of
	// scope here. Same hardened scaffolding as the `/run/secrets/`
	// row above so `--strict` reads only the verdict under test.
	for value in ["/etc/passwd", "/tmp/whatever"] {
		let key = "PASSWORD_FILE";
		let body = format!(
			"services:\n  app:\n    \
			 image: alpine:3.20@sha256:0e7bb5afc7e5e22ee46c4f2cd4a8b3fa63ad3f5d5e5e5e5e5e5e5e5e5e5e5e5e5e\n    \
			 read_only: true\n    \
			 cap_drop: [ALL]\n    \
			 security_opt: [no-new-privileges:true]\n    \
			 pids_limit: 200\n    \
			 mem_limit: 512m\n    \
			 memswap_limit: 512m\n    \
			 init: true\n    \
			 restart: unless-stopped\n    \
			 cpus: \"1\"\n    \
			 healthcheck:\n      \
			 test: [\"CMD\", \"true\"]\n      \
			 x-podman-on-failure: restart\n    \
			 userns_mode: auto\n    \
			 environment:\n      \
			 - {key}={value}\n"
		);
		let path = write_compose(&body);
		let p = path.to_str().unwrap();
		let out = run(&["-f", p, "audit", "--strict"]);
		assert!(
			out.status.success(),
			"{key}={value} must pass --strict; got {:?}\nstderr: {}\nstdout: {}",
			out.status.code(),
			String::from_utf8_lossy(&out.stderr),
			String::from_utf8_lossy(&out.stdout),
		);
		let stdout = String::from_utf8_lossy(&out.stdout);
		assert!(
			!stdout.contains("secret_in_environment"),
			"{key}={value} must not fire secret_in_environment: {stdout}"
		);
	}
}

#[test]
fn audit_secret_in_environment_does_not_flag_file_suffix_with_relative_path_prefix() {
	// The path-shape test accepts the two relative-path prefixes the
	// same way it accepts `/`. `./secrets/pg` and `../pg` are paths an
	// operator can write under the same `_FILE` convention the
	// absolute-path rows pin; `--strict` stays green on each. Same
	// hardened scaffolding as the rows above so `--strict` reads only
	// the verdict under test.
	for value in ["./secrets/pg", "../pg"] {
		let key = "PASSWORD_FILE";
		let body = format!(
			"services:\n  app:\n    \
			 image: alpine:3.20@sha256:0e7bb5afc7e5e22ee46c4f2cd4a8b3fa63ad3f5d5e5e5e5e5e5e5e5e5e5e5e5e5e\n    \
			 read_only: true\n    \
			 cap_drop: [ALL]\n    \
			 security_opt: [no-new-privileges:true]\n    \
			 pids_limit: 200\n    \
			 mem_limit: 512m\n    \
			 memswap_limit: 512m\n    \
			 init: true\n    \
			 restart: unless-stopped\n    \
			 cpus: \"1\"\n    \
			 healthcheck:\n      \
			 test: [\"CMD\", \"true\"]\n      \
			 x-podman-on-failure: restart\n    \
			 userns_mode: auto\n    \
			 environment:\n      \
			 - {key}={value}\n"
		);
		let path = write_compose(&body);
		let p = path.to_str().unwrap();
		let out = run(&["-f", p, "audit", "--strict"]);
		assert!(
			out.status.success(),
			"{key}={value} must pass --strict; got {:?}\nstderr: {}\nstdout: {}",
			out.status.code(),
			String::from_utf8_lossy(&out.stderr),
			String::from_utf8_lossy(&out.stdout),
		);
		let stdout = String::from_utf8_lossy(&out.stdout);
		assert!(
			!stdout.contains("secret_in_environment"),
			"{key}={value} must not fire secret_in_environment: {stdout}"
		);
	}
}

#[test]
fn audit_secret_in_environment_flags_file_suffix_when_value_is_not_path_shaped() {
	// The `_FILE` suffix is the operator's signal, not an exemption.
	// A literal password written into a `_FILE` key, or a value that
	// merely contains a slash but does not start with one of the three
	// path prefixes, falls through to the flagging rule. The reporter's
	// exact shape (`POSTGRES_PASSWORD_FILE: hunter2-real-password`) is
	// pinned alongside `a/b` (contains a slash, does not start with
	// one) and `relative/path` so a regression that misses one is
	// caught by its own row. Same hardened scaffolding as the rows
	// above so `--strict` reads only the verdict under test.
	for (key, value) in [
		("POSTGRES_PASSWORD_FILE", "hunter2-real-password"),
		("API_TOKEN_FILE", "literal-token"),
		("PASSWORD_FILE", "a/b"),
		("SECRET_FILE", "relative/path"),
	] {
		let body = format!(
			"services:\n  app:\n    \
			 image: alpine:3.20@sha256:0e7bb5afc7e5e22ee46c4f2cd4a8b3fa63ad3f5d5e5e5e5e5e5e5e5e5e5e5e5e5e\n    \
			 read_only: true\n    \
			 cap_drop: [ALL]\n    \
			 security_opt: [no-new-privileges:true]\n    \
			 pids_limit: 200\n    \
			 mem_limit: 512m\n    \
			 memswap_limit: 512m\n    \
			 init: true\n    \
			 restart: unless-stopped\n    \
			 cpus: \"1\"\n    \
			 healthcheck:\n      \
			 test: [\"CMD\", \"true\"]\n      \
			 x-podman-on-failure: restart\n    \
			 userns_mode: auto\n    \
			 environment:\n      \
			 - {key}={value}\n"
		);
		let path = write_compose(&body);
		let p = path.to_str().unwrap();
		let out = run(&["-f", p, "audit", "--strict"]);
		assert!(
			!out.status.success(),
			"{key}={value} must FAIL --strict; got {:?}\nstderr: {}\nstdout: {}",
			out.status.code(),
			String::from_utf8_lossy(&out.stderr),
			String::from_utf8_lossy(&out.stdout),
		);
		let stdout = String::from_utf8_lossy(&out.stdout);
		assert!(
			stdout.contains("secret_in_environment"),
			"{key}={value} must fire secret_in_environment: {stdout}"
		);
	}
}

#[test]
fn audit_secret_in_environment_does_not_flag_non_secret_keys_ending_in_file() {
	// `_FILE` is the operator's signal for "path", not a token in
	// itself. The check only reaches the `_FILE` branch when the
	// key's segments include one of `PASSWORD|SECRET|TOKEN|KEY`.
	// `CONFIG_FILE` segments into `CONFIG`, `FILE`; neither matches a
	// keyword, so the iteration continues before the path-shape test
	// runs and the check stays silent on `CONFIG_FILE: x` regardless
	// of the value's shape. Same hardened scaffolding as the rows
	// above so `--strict` reads only the verdict under test.
	for value in ["x", "/etc/something"] {
		let key = "CONFIG_FILE";
		let body = format!(
			"services:\n  app:\n    \
			 image: alpine:3.20@sha256:0e7bb5afc7e5e22ee46c4f2cd4a8b3fa63ad3f5d5e5e5e5e5e5e5e5e5e5e5e5e5e\n    \
			 read_only: true\n    \
			 cap_drop: [ALL]\n    \
			 security_opt: [no-new-privileges:true]\n    \
			 pids_limit: 200\n    \
			 mem_limit: 512m\n    \
			 memswap_limit: 512m\n    \
			 init: true\n    \
			 restart: unless-stopped\n    \
			 cpus: \"1\"\n    \
			 healthcheck:\n      \
			 test: [\"CMD\", \"true\"]\n      \
			 x-podman-on-failure: restart\n    \
			 userns_mode: auto\n    \
			 environment:\n      \
			 - {key}={value}\n"
		);
		let path = write_compose(&body);
		let p = path.to_str().unwrap();
		let out = run(&["-f", p, "audit", "--strict"]);
		assert!(
			out.status.success(),
			"{key}={value} must pass --strict; got {:?}\nstderr: {}\nstdout: {}",
			out.status.code(),
			String::from_utf8_lossy(&out.stderr),
			String::from_utf8_lossy(&out.stdout),
		);
		let stdout = String::from_utf8_lossy(&out.stdout);
		assert!(
			!stdout.contains("secret_in_environment"),
			"{key}={value} must not fire secret_in_environment: {stdout}"
		);
	}
}
