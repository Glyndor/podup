//! Tests for the `secret_in_environment` check, split from `checks_more_tests.rs`
//! to keep that file under the repository line limit. The check has two shapes
//! to verify: the verdict (flag/passthrough) and the segment split helper.

use super::tests::report_for;

// ---------------------------------------------------------------------------
// secret_in_environment
// ---------------------------------------------------------------------------

#[test]
fn audit_secret_in_environment_flags_literal_secret_keys() {
	// One per keyword so a regression that misses one is caught by its own
	// test rather than masked by the others passing.
	for (key, _hint) in [
		("DB_PASSWORD", "PASSWORD"),
		("AUTH_SECRET", "SECRET"),
		("API_TOKEN", "TOKEN"),
		("SIGNING_KEY", "KEY"),
	] {
		let yaml = format!(
			"services:\n  app:\n    image: alpine:3.20\n    environment:\n      - {key}=literal\n"
		);
		let findings = report_for(&yaml);
		assert!(
			findings.iter().any(|f| f.check == "secret_in_environment"),
			"environment: {key}=literal must fire secret_in_environment; got {findings:#?}"
		);
		// The reason must name the key without echoing the value back:
		// the literal value would defeat the whole point of the audit.
		let f = findings
			.iter()
			.find(|f| f.check == "secret_in_environment")
			.expect("finding");
		assert!(
			!f.reason.contains("literal"),
			"reason must not echo the value: {f:?}"
		);
		assert!(
			f.reason.contains(key),
			"reason must name the key {key}: {f:?}"
		);
	}
}

#[test]
fn audit_secret_in_environment_passes_for_unrelated_keys() {
	let yaml = r#"
services:
  app:
    image: alpine:3.20
    environment:
      - LOG_LEVEL=info
      - HOSTNAME=host-1
      - PORT=8080
"#;
	let findings = report_for(yaml);
	assert!(
		!findings.iter().any(|f| f.check == "secret_in_environment"),
		"unrelated env keys must not fire secret_in_environment: {findings:#?}"
	);
}

// ---------------------------------------------------------------------------
// secret_in_environment: segment equality replaces the old substring match
// (issue 1709). The four keyword-segment rows below each pin one shape:
// a name where a keyword only appears as a substring, a name where the
// segment merely starts with a keyword, underscore/dot separators, and
// camelCase splits. The fifth test pins the helper itself, so a regression
// in how segments are computed is caught before it can masquerade as a
// verdict change.
// ---------------------------------------------------------------------------

/// `MONKEY_HABITAT` contains `KEY` only as a substring; its segments
/// (`MONKEY`, `HABITAT`) match no keyword. The old `contains` rule
/// flagged this name; segment equality leaves it silent.
#[test]
fn audit_secret_in_environment_silent_when_keyword_only_appears_as_substring() {
	let yaml = r#"
services:
  app:
    image: alpine:3.20
    environment:
      - MONKEY_HABITAT=bananas
"#;
	let findings = report_for(yaml);
	assert!(
		!findings.iter().any(|f| f.check == "secret_in_environment"),
		"MONKEY_HABITAT must NOT fire secret_in_environment; got {findings:#?}"
	);
}

/// `MD_Keywords` has segments `MD`, `KEYWORDS`. `KEYWORDS` starts with
/// `KEY` but does not equal it, so the check stays silent. The same is
/// true for `MD_Terminos`, which has no overlap with any keyword at
/// all; grouping them keeps the load-bearing case (KEYWORDS vs KEY)
/// next to its non-overlap sibling.
#[test]
fn audit_secret_in_environment_silent_when_segment_only_starts_with_keyword() {
	for key in ["MD_Keywords", "MD_Terminos"] {
		let yaml = format!(
			"services:\n  app:\n    image: alpine:3.20\n    environment:\n      - {key}=value\n"
		);
		let findings = report_for(&yaml);
		assert!(
			!findings.iter().any(|f| f.check == "secret_in_environment"),
			"{key} must NOT fire secret_in_environment; got {findings:#?}"
		);
	}
}

/// `DB_PASSWORD`, `AWS_SECRET_ACCESS_KEY`, `API_X_Token`, `API_YT_Key`,
/// `my.secret.value` each split into segments that include a full
/// keyword. `_` and `.` are the two separators the helper treats like
/// `WORD` boundaries; they are pinned together here so a regression
/// that drops one separator is caught.
#[test]
fn audit_secret_in_environment_flags_keys_split_by_underscore_or_dot() {
	for key in [
		"DB_PASSWORD",
		"AWS_SECRET_ACCESS_KEY",
		"API_X_Token",
		"API_YT_Key",
		"my.secret.value",
	] {
		let yaml = format!(
			"services:\n  app:\n    image: alpine:3.20\n    environment:\n      - {key}=value\n"
		);
		let findings = report_for(&yaml);
		assert!(
			findings.iter().any(|f| f.check == "secret_in_environment"),
			"{key} must fire secret_in_environment; got {findings:#?}"
		);
	}
}

/// `apiToken` and `API_T_ClientSecret` carry their keyword inside a
/// run-on word; the camelCase split is what surfaces it. Without it
/// the helper would have left each name as a single segment, and
/// these two cases would have become false negatives after the
/// substring match was removed.
#[test]
fn audit_secret_in_environment_flags_camel_case_keys() {
	for key in ["apiToken", "API_T_ClientSecret"] {
		let yaml = format!(
			"services:\n  app:\n    image: alpine:3.20\n    environment:\n      - {key}=value\n"
		);
		let findings = report_for(&yaml);
		assert!(
			findings.iter().any(|f| f.check == "secret_in_environment"),
			"{key} must fire secret_in_environment; got {findings:#?}"
		);
	}
}

/// A non-ASCII lead byte counts as the lower side of a camelCase
/// boundary. Measured while reviewing #1709: without that clause the
/// segment split swallowed the key whole, so a name the old substring
/// match reported went silent, trading the false positive for a false
/// negative.
#[test]
fn audit_secret_in_environment_flags_camel_case_after_a_non_ascii_byte() {
	let yaml =
		"services:\n  app:\n    image: alpine:3.20\n    environment:\n      - \u{d1}Key=value\n";
	let findings = report_for(yaml);
	assert!(
		findings.iter().any(|f| f.check == "secret_in_environment"),
		"a key whose Key segment follows a non-ASCII byte must fire secret_in_environment; got {findings:#?}"
	);
}

/// Helper contract: the segments the helper produces for each row of
/// the issue's truth table. A regression in the split (a missing
/// camelCase boundary, a dropped separator) flips this test red
/// before the verdict tests above can pass on accident.
#[test]
fn audit_secret_in_environment_segments_split_at_case_and_separator_boundaries() {
	let cases: &[(&str, &[&str])] = &[
		("MONKEY_HABITAT", &["MONKEY", "HABITAT"]),
		("MD_Keywords", &["MD", "KEYWORDS"]),
		("MD_Terminos", &["MD", "TERMINOS"]),
		("API_X_Token", &["API", "X", "TOKEN"]),
		("API_T_ClientSecret", &["API", "T", "CLIENT", "SECRET"]),
		("API_YT_Key", &["API", "YT", "KEY"]),
		("DB_PASSWORD", &["DB", "PASSWORD"]),
		("AWS_SECRET_ACCESS_KEY", &["AWS", "SECRET", "ACCESS", "KEY"]),
		("apiToken", &["API", "TOKEN"]),
		("my.secret.value", &["MY", "SECRET", "VALUE"]),
	];
	for (name, expected_segments) in cases {
		let got = super::segments(name);
		let got_refs: Vec<&str> = got.iter().map(String::as_str).collect();
		assert_eq!(got_refs, *expected_segments, "wrong segments for {name}");
	}
}

// ---------------------------------------------------------------------------
// `_FILE` suffix: the documented convention for keeping a secret out of the
// environment. The value is a path the application reads at runtime; the
// path is not the secret. The exemption is the `_FILE` suffix alone, so
// every separator the segment helper recognises must reach the same
// conclusion. The outside-secrets-mount row pins the "value can be any
// path" half of the decision.
// ---------------------------------------------------------------------------

/// Each row exercises a different separator that the segment helper
/// already treats as a boundary. `KeyFile` is camelCase, `KEY-FILE` uses
/// the `-` separator, `KEY.FILE` uses the `.` separator, `POSTGRES_PASSWORD_FILE`
/// is the reporter's exact key, and `MY_KEY_FILE` and `_TOKEN_FILE` cover
/// the multi-segment and leading-underscore shapes. The convention is the
/// suffix, not the prefix, so every row stays silent.
#[test]
fn audit_secret_in_environment_silent_for_keys_with_the_file_suffix() {
	for (key, value) in [
		("POSTGRES_PASSWORD_FILE", "/run/secrets/pg"),
		("MY_KEY_FILE", "/run/secrets/k"),
		("_TOKEN_FILE", "/run/secrets/t"),
		("KeyFile", "/run/secrets/camel"),
		("KEY-FILE", "/run/secrets/dash"),
		("KEY.FILE", "/run/secrets/dot"),
	] {
		let yaml = format!(
			"services:\n  app:\n    image: alpine:3.20\n    environment:\n      - {key}={value}\n"
		);
		let findings = report_for(&yaml);
		assert!(
			!findings.iter().any(|f| f.check == "secret_in_environment"),
			"{key}={value} must NOT fire secret_in_environment; got {findings:#?}"
		);
	}
}

/// The exemption is the `_FILE` suffix, not the `/run/secrets/` prefix.
/// A `_FILE` key pointing somewhere that is not the secrets mount is
/// still a path, not a secret in the environment, so the check stays
/// silent. Verifying what the path points to (file permissions, mount
/// provenance) is a different audit concern and is out of scope here.
#[test]
fn audit_secret_in_environment_silent_for_file_suffix_pointing_outside_secrets_mount() {
	for (key, value) in [
		("PASSWORD_FILE", "/etc/passwd"),
		("PASSWORD_FILE", "relative/path"),
		("SECRET_FILE", "/tmp/whatever"),
		("TOKEN_FILE", ""),
	] {
		let yaml = format!(
			"services:\n  app:\n    image: alpine:3.20\n    environment:\n      - {key}={value}\n"
		);
		let findings = report_for(&yaml);
		assert!(
			!findings.iter().any(|f| f.check == "secret_in_environment"),
			"{key}={value} must NOT fire secret_in_environment; got {findings:#?}"
		);
	}
}

/// `FILE` in the middle of the key is not the `_FILE` convention. A
/// segment other than the last being `FILE` leaves the last segment as
/// the keyword, so the check still fires. `PASSWORD_FILE_BACKUP` (last
/// segment `BACKUP`) and `FILE_PASSWORD` (last segment `PASSWORD`, and
/// the bare `FILE` first segment is not a keyword) are both covered.
#[test]
fn audit_secret_in_environment_flags_when_file_is_not_the_last_segment() {
	for key in ["PASSWORD_FILE_BACKUP", "FILE_PASSWORD"] {
		let yaml = format!(
			"services:\n  app:\n    image: alpine:3.20\n    environment:\n      - {key}=value\n"
		);
		let findings = report_for(&yaml);
		assert!(
			findings.iter().any(|f| f.check == "secret_in_environment"),
			"{key}=value must fire secret_in_environment; got {findings:#?}"
		);
	}
}
