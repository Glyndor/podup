//! The release's dependency audit must be at least as strict as the weekly one.
//!
//! Two workflows run `cargo audit` and the strictness flag is written in both,
//! derived from neither. Before #1592 they disagreed: `reusable-rust-audit.yml`
//! ran `--deny warnings` and `release.yml` did not, so the gate standing in
//! front of an immutable artifact was the more permissive of the two. An
//! unmaintained or yanked crate is a warning, and a release is the one place a
//! warning cannot be looked at again later.
//!
//! The audit is not conditioned on paths: an advisory appears without any file
//! changing, so it runs on every pull request and every push to `develop`, and
//! the schedule audits both branches. A blocked release is fixed by releasing
//! again. A published one cannot be fixed at all.

use std::fs;
use std::path::Path;

fn read(name: &str) -> String {
	let path = Path::new(env!("CARGO_MANIFEST_DIR"))
		.join(".github/workflows")
		.join(name);
	fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name} is readable: {e}"))
}

/// Every line that INVOKES `cargo audit`, with its flags.
///
/// Two shapes are excluded and both are real. A comment line: both files
/// explain the flag in prose directly above the line that carries it, so a
/// search for the string finds it whether or not the command has it. And a
/// YAML key: `reusable-rust-audit.yml` names its job `cargo audit`, which is a
/// label, not a command. The first draft of this parser counted that label and
/// the test went red against a file that was already correct.
///
/// So the rule is positional: the trimmed line must BEGIN the command, either
/// on its own inside a `run: |` block or immediately after `run:`.
fn audit_invocations(workflow: &str) -> Vec<&str> {
	workflow
		.lines()
		.map(str::trim)
		.filter(|l| !l.starts_with('#'))
		.filter(|l| {
			l.starts_with("cargo audit")
				|| l.starts_with("cargo-audit audit")
				|| l.starts_with("run: cargo audit")
				|| l.starts_with("run: cargo-audit audit")
		})
		.collect()
}

#[test]
fn every_cargo_audit_denies_warnings() {
	for name in ["release.yml", "reusable-rust-audit.yml"] {
		let body = read(name);
		let calls = audit_invocations(&body);
		assert!(
			!calls.is_empty(),
			"{name} runs no cargo audit; either the step moved and this test is \
			 reading the wrong file, or the gate is gone"
		);
		for call in calls {
			assert!(
				call.contains("--deny warnings"),
				"{name} runs {call:?} without --deny warnings. The two gates \
				 must agree on strictness: whichever is weaker is the one that \
				 decides, and for the release that decision is immutable."
			);
		}
	}
}

#[test]
fn the_audit_runs_on_every_pull_request_and_every_push_to_develop() {
	let spec: serde_yaml::Value =
		serde_yaml::from_str(&read("audit.yml")).expect("audit.yml parses");
	let on = &spec["on"];
	for event in ["pull_request", "push"] {
		assert!(
			on.get(event).is_some(),
			"audit.yml does not run on {event}: an advisory published today would wait for the \
			 daily schedule"
		);
		assert!(
			on[event].get("paths").is_none() && on[event].get("paths-ignore").is_none(),
			"audit.yml's {event} trigger has a paths filter. A new advisory appears without any \
			 file changing, so the audit cannot be conditioned on paths"
		);
	}
	assert_eq!(
		on["push"]["branches"],
		serde_yaml::from_str::<serde_yaml::Value>("[develop]").unwrap(),
		"the push audit is for develop"
	);
	assert!(
		on.get("schedule").is_some(),
		"audit.yml has no schedule, so nothing audits a branch nobody pushes to"
	);
	assert!(
		read("audit.yml").contains("audit-develop:"),
		"the scheduled run no longer audits develop, only the default branch"
	);
}

/// A security tool is the official release binary, downloaded on every run and
/// checked against a sum before it runs: never restored from a cache, never
/// compiled and cached.
#[test]
fn the_audit_tools_are_downloaded_and_checked_and_never_restored_from_a_cache() {
	let body = read("reusable-rust-audit.yml");
	assert!(
		!body.contains("actions/cache"),
		"reusable-rust-audit.yml restores something from a cache; a security tool must not be"
	);
	assert!(
		!body.contains("cargo install"),
		"reusable-rust-audit.yml compiles a tool; the release binary is downloaded and checked"
	);
	assert_eq!(
		body.matches("sha256sum -c -").count(),
		2,
		"each of the two tools is checked against its sum before it runs"
	);
	for input in ["cargo-audit-sha256", "cargo-deny-sha256"] {
		assert!(body.contains(input), "{input} is not declared");
	}
}

/// The parser is what both tests trust. Pin it on input that differs from the
/// real files, including the shape it must not count.
#[test]
fn the_parser_ignores_the_prose_that_explains_the_flag() {
	let yml = "\
  audit:
    name: cargo audit
    steps:
      - name: Audit
        # cargo audit --deny warnings, explained here and not run
        run: |
          cargo audit --deny warnings
";
	let calls = audit_invocations(yml);
	assert_eq!(
		calls,
		vec!["cargo audit --deny warnings"],
		"exactly one line invokes it. The other two name it: a job label \
		 `name: cargo audit`, which is what the first draft of this parser \
		 counted, and a comment explaining the flag."
	);
}
