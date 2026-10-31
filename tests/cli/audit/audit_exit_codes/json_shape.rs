//! `--format json`: every finding listed, stable schema, no escapes.
//!
//! Split out from `tests/cli/audit/audit_exit_codes.rs` so the parent file stays under
//! the repository line limit; the helpers (`run`, `write_compose`) are
//! inherited via `use super::*`.

use super::*;

#[test]
fn audit_json_lists_every_finding() {
	// Three services, each deliberately failing a different check. The
	// JSON output must carry every one of them: a regression where a check
	// silently no-ops in JSON (while still appearing in the table) would
	// pass the unit tests and break CI consumers in production.
	let path = write_compose(
		r#"services:
  pr:
    image: nginx
  caps:
    image: nginx:1.27
    cap_add: [SYS_ADMIN]
  secret:
    image: nginx:1.27
    environment:
      - DB_PASSWORD=hunter2
"#,
	);
	let p = path.to_str().unwrap();
	let out = run(&["-f", p, "audit", "--format", "json"]);
	assert!(out.status.success(), "audit must succeed");
	let stdout = String::from_utf8_lossy(&out.stdout);
	// Machine output never carries colour, even when
	// forced. A regression that leaks an escape into `--format json`
	// would corrupt every CI consumer parsing the output; verify the
	// unforced (no `--ansi always`) path first, then assert the data.
	assert!(
		!stdout.contains('\u{1b}'),
		"JSON output must not carry escapes: {stdout:?}"
	);
	let v: serde_json::Value = serde_json::from_str(&stdout).expect("JSON parses");
	let arr = v
		.get("findings")
		.and_then(|f| f.as_array())
		.expect("`findings` array");
	// Each service is expected to fire at least one finding; json-list
	// must reflect them all.
	assert!(arr.len() >= 3, "expected at least 3 findings, got {arr:?}");
	let services: Vec<&str> = arr
		.iter()
		.map(|f| f.get("service").and_then(|s| s.as_str()).unwrap_or("?"))
		.collect();
	for needed in ["pr", "caps", "secret"] {
		assert!(
			services.contains(&needed),
			"missing finding for `{needed}` in {arr:?}"
		);
	}
	// Per-object shape: every entry must carry all three keys, with
	// non-empty strings. A consumer reading `reason: ""` as "no finding"
	// would mis-classify an empty row, so we pin the absence.
	for entry in arr {
		for key in ["service", "check", "reason"] {
			let s = entry.get(key).and_then(|v| v.as_str()).unwrap_or("");
			assert!(
				!s.is_empty(),
				"every finding must carry non-empty `{key}`: {entry:?}"
			);
		}
	}
}
