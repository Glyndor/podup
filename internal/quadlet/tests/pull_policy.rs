//! #1970 round 2: a typo'd `pull_policy:` must be rejected at validation
//! time, not silently turned into a `Pull=` line Quadlet cannot honour.
//! Prebuilt-mode autostart overrides the user's value to `Pull=never` on
//! every buildable service, so without this check a typo'd
//! `pull_policy: alaways` installs cleanly as `Pull=never` even though the
//! user clearly asked for something specific. `generate quadlet` does
//! emit the user's value verbatim, so an invalid one would also write a
//! broken `Pull=` line Quadlet drops or fails on at daemon-reload. Same
//! root cause, same fix, both paths.

use crate::parse_str;
use crate::quadlet::validate_for_quadlet;

/// A known-typo is the case the review measured; pin it first.
#[test]
fn validate_rejects_an_unknown_pull_policy() {
	let yaml = "services:\n  web:\n    image: nginx\n    pull_policy: alaways\n";
	let err = validate_for_quadlet(&parse_str(yaml).unwrap())
		.expect_err("a typo'd pull_policy must fail validation");
	let msg = err.to_string();
	assert!(
		msg.contains("alaways"),
		"the rejected value must appear in the error so the operator sees the typo: {msg}"
	);
	// The error wording is the engine's `pull_policy_checked` output, so a
	// future rename of accepted policies (or the wording) is caught by the
	// engine's own tests; we check the actionable substrings stay there.
	assert!(
		msg.contains("pull_policy"),
		"field name must be present: {msg}"
	);
}

/// Every value `pull_policy_checked` accepts must also pass the Quadlet
/// path's gate, otherwise a project that runs cleanly under `up`/`pull` would
/// fail at `generate quadlet` / `autostart install --mode quadlet`.
#[test]
fn validate_accepts_every_documented_pull_policy() {
	for value in [
		"always",
		"missing",
		"if_not_present",
		"newer",
		"never",
		"build",
	] {
		let yaml = format!("services:\n  web:\n    image: nginx\n    pull_policy: {value}\n");
		validate_for_quadlet(&parse_str(&yaml).unwrap())
			.unwrap_or_else(|e| panic!("{value} must validate, got: {e}"));
	}
}

/// The same gate covers both paths: `internal/generate.rs` runs
/// `validate_for_quadlet` ahead of `generate_at`, and `install_quadlet`
/// runs it ahead of `generate_for_autostart`. A typo on a buildable service
/// (the place the prebuilt-mode `Pull=never` override would otherwise mask
/// it) is caught here before either path runs.
#[test]
fn validate_rejects_unknown_pull_policy_on_a_buildable_service() {
	let yaml =
		"services:\n  web:\n    build: .\n    image: reg.example/app:1\n    pull_policy: alaways\n";
	let err = validate_for_quadlet(&parse_str(yaml).unwrap())
		.expect_err("a typo'd pull_policy on a buildable service must fail validation");
	let msg = err.to_string();
	assert!(msg.contains("alaways"), "got: {msg}");
}
