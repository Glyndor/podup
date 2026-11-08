use super::*;

// ---------------------------------------------------------------------------
// secret_in_environment: bare keys stay silent, ${VAR} placeholders fire.
//
// A bare key (no `=`) inherits from the host; the operator has not authored
// a value in compose, so the check stays silent. A `${VAR}` placeholder
// resolves at parse time to the value of `VAR` in the loader's environment
// or to an empty string when unset, and either way ends up as the
// container's environment variable. The risk is identical to a literal
// secret, so the verdict stops depending on whether the caller happened
// to export `VAR`.
// ---------------------------------------------------------------------------

#[test]
fn audit_secret_in_environment_ignores_bare_keys_but_flags_placeholders() {
	// List form: a bare key stays silent; a `${VAR}` placeholder fires,
	// because the risk is the secret ending up in the container's
	// environment, and that happens whether `VAR` is exported or not.
	let yaml = r#"
services:
  app:
    image: alpine:3.20
    environment:
      - DB_PASSWORD
      - API_TOKEN=${TOKEN_FROM_ENV}
"#;
	let file = parse_str(yaml).expect("parses");
	let report = audit::audit_file(&file, &[]);
	assert!(
		!report.findings.iter().any(|f| f.service == "app"
			&& f.check == "secret_in_environment"
			&& f.reason.contains("DB_PASSWORD")),
		"bare key DB_PASSWORD must stay silent: {:#?}",
		report.findings
	);
	assert!(
		report.findings.iter().any(|f| f.service == "app"
			&& f.check == "secret_in_environment"
			&& f.reason.contains("API_TOKEN")),
		"placeholder API_TOKEN=${{TOKEN_FROM_ENV}} must fire regardless of VAR being \
		 exported: {:#?}",
		report.findings
	);

	// Map form: `null` is what compose / podman read as "inherit from the
	// host"; the parser turns it into a `None` value in the `to_map()`
	// view, so it stays silent. The `${VAR}` form still fires.
	let yaml_map = r#"
services:
  app:
    image: alpine:3.20
    environment:
      DB_PASSWORD: null
      API_TOKEN: ${TOKEN_FROM_ENV}
"#;
	let file = parse_str(yaml_map).expect("parses");
	let report = audit::audit_file(&file, &[]);
	assert!(
		!report.findings.iter().any(|f| f.service == "app"
			&& f.check == "secret_in_environment"
			&& f.reason.contains("DB_PASSWORD")),
		"map-form bare key DB_PASSWORD must stay silent: {:#?}",
		report.findings
	);
	assert!(
		report.findings.iter().any(|f| f.service == "app"
			&& f.check == "secret_in_environment"
			&& f.reason.contains("API_TOKEN")),
		"map-form placeholder API_TOKEN=${{TOKEN_FROM_ENV}} must fire regardless of VAR \
		 being exported: {:#?}",
		report.findings
	);
}

// ---------------------------------------------------------------------------
// unpinned_image: a digest always counts as pinned.
// ---------------------------------------------------------------------------

/// Find a finding for `service` whose check id equals `check`. Returns `None`
/// when no such finding exists. Many tests use this as a list
/// non-membership test (the early return they wanted was "no finding", the
/// absence is the assertion).
fn has_check(report: &AuditReport, service: &str, check: &str) -> bool {
	report
		.findings
		.iter()
		.any(|f| f.service == service && f.check == check)
}

#[test]
fn audit_unpinned_image_accepts_a_digest() {
	// The image reference carries an explicit tag *and* a digest. The
	// digest is the actual pin; the tag is a convenience. A regression
	// that ignores `@sha256:` and looks at the tag would flag this
	// reference (the tag is `latest`, no less) and the test flips red.
	let yaml = r#"
services:
  app:
    image: nginx:latest@sha256:0e7bb5afc7e5e22ee46c4f2cd4a8b3fa63ad3f5d5e5e5e5e5e5e5e5e5e5e5e5e
"#;
	let file = parse_str(yaml).expect("parses");
	let report = audit::audit_file(&file, &[]);
	assert!(
		!has_check(&report, "app", "unpinned_image"),
		"digest-pinned image must not be flagged: {:#?}",
		report.findings
	);
	// Also cover the no-tag case with a digest, so an `rfind` regression
	// on the colon/slash boundary gets caught too.
	let yaml_notag = r#"
services:
  app:
    image: nginx@sha256:0e7bb5afc7e5e22ee46c4f2cd4a8b3fa63ad3f5d5e5e5e5e5e5e5e5e5e5e5e5e
"#;
	let report = audit::audit_file(&parse_str(yaml_notag).unwrap(), &[]);
	assert!(
		!has_check(&report, "app", "unpinned_image"),
		"digest-only image must not be flagged"
	);
}

/// A value that merely ends in `}` is not a `${VAR}` placeholder.
#[test]
fn audit_secret_in_environment_flags_a_value_that_only_ends_in_a_brace() {
	let report = report_for_file("services:\n  web:\n    image: alpine:3.20\n    environment:\n      DB_PASSWORD: \"abc}\"\n");
	assert!(report
		.findings
		.iter()
		.any(|f| f.check == "secret_in_environment"));
}
