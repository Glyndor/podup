//! Inspect native-secret mount modes and ownership at the daemon boundary.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;

#[path = "../support/fake_engine.rs"]
mod fake_engine;
#[path = "../support/host_files.rs"]
mod host_files;

use fake_engine::FakeEngine;
use host_files::{accepts, body, Running};

fn compose(kind: &str, fields: &str) -> String {
	format!("services:\n  web:\n    image: alpine\n    {kind}:\n      - source: private\n{fields}\n{kind}:\n  private:\n    file: ./private.txt\n")
}

#[test]
fn hostile_modes_and_owners_never_widen_secret_or_config_permissions() {
	let mut failures = Vec::new();
	for kind in ["secrets", "configs"] {
		for (fields, expected_mode, uid, gid) in [
			("", 0o600, None, None),
			(
				"        mode: '0400'\n        uid: '4294967295'\n        gid: '0'",
				0o400,
				Some(u32::MAX),
				Some(0u32),
			),
			(
				"        mode: '0000'\n        uid: '-1'\n        gid: '4294967296'",
				0,
				None,
				None,
			),
			(
				"        mode: '0600'\n        uid: 'nobody'\n        gid: 'nogroup'",
				0o600,
				None,
				None,
			),
		] {
			let fake = FakeEngine::start(accepts);
			fs::write(fake.dir().join("private.txt"), "private payload").unwrap();
			fs::set_permissions(
				fake.dir().join("private.txt"),
				fs::Permissions::from_mode(0o600),
			)
			.unwrap();
			fs::write(fake.dir().join("compose.yaml"), compose(kind, fields)).unwrap();
			let mut child = Running::start(&fake, &["create"]);
			assert!(child.wait().success(), "{}", child.text());
			let requests = fake.requests();
			let request = requests
				.iter()
				.find(|r| {
					r.starts_with(b"POST ")
						&& String::from_utf8_lossy(r)
							.lines()
							.next()
							.unwrap()
							.contains("/containers/create")
				})
				.expect("no container spec");
			let spec: serde_json::Value = serde_json::from_slice(&body(request)).unwrap();
			let secrets = spec["secrets"].as_array().expect("no native-secret mounts");
			assert_eq!(secrets.len(), 1, "{spec}");
			let mount = &secrets[0];
			assert_eq!(
				mount["Mode"].as_u64(),
				Some(expected_mode),
				"{kind}: {spec}"
			);
			assert_eq!(
				mount["UID"].as_u64().unwrap_or(0),
				u64::from(uid.unwrap_or(0)),
				"{spec}"
			);
			assert_eq!(
				mount["GID"].as_u64().unwrap_or(0),
				u64::from(gid.unwrap_or(0)),
				"{spec}"
			);
		}
		for mode in [
			"'0100'",
			"'04700'",
			"'02000'",
			"'01000'",
			"'10000'",
			"'100000'",
			"'0899'",
			"'0o888'",
			"'-1'",
			"'40000000000'",
		] {
			let fake = FakeEngine::start(accepts);
			fs::write(fake.dir().join("private.txt"), "private payload").unwrap();
			fs::write(
				fake.dir().join("compose.yaml"),
				compose(kind, &format!("        mode: {mode}")),
			)
			.unwrap();
			let mut child = Running::start(&fake, &["create"]);
			let status = child.wait();
			let text = child.text();
			if status.success() || fake.sent().contains("/containers/create") {
				failures.push(format!("{kind} mode={mode}: status={status}; {text}"));
			}
		}
	}
	assert!(failures.is_empty(), "{}", failures.join("\n"));
}
