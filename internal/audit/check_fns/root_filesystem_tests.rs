use super::super::tests::report_for;

// ---------------------------------------------------------------------------
// writable_root
// ---------------------------------------------------------------------------

#[test]
fn audit_writable_root_flags_when_read_only_false_or_absent() {
	for yaml in [
		// absent entirely.
		r#"services:
  web:
    image: alpine:3.20
"#,
		// explicit false (the compose default).
		r#"services:
  web:
    image: alpine:3.20
    read_only: false
"#,
	] {
		let findings = report_for(yaml);
		assert!(
			findings.iter().any(|f| f.check == "writable_root"),
			"the writable_root check must fire here: {findings:#?}"
		);
	}
}

#[test]
fn audit_writable_root_passes_when_read_only_true() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    read_only: true
"#;
	let findings = report_for(yaml);
	assert!(
		!findings.iter().any(|f| f.check == "writable_root"),
		"read_only: true must pass the writable_root check: {findings:#?}"
	);
}
