use crate::audit::Finding;

use super::super::tests::report_for;

// ---------------------------------------------------------------------------
// privileged
// ---------------------------------------------------------------------------

#[test]
fn audit_privileged_flags_when_true() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    privileged: true
"#;
	let findings = report_for(yaml);
	let privileged: Vec<&Finding> = findings
		.iter()
		.filter(|f| f.check == "privileged")
		.collect();
	assert_eq!(
		privileged.len(),
		1,
		"exactly one privileged finding expected: {findings:#?}"
	);
	assert_eq!(privileged[0].check, "privileged");
	assert_eq!(privileged[0].service, "web");
}

#[test]
fn audit_privileged_passes_when_false_or_absent() {
	// Explicit false: still fine. Absent: also fine. The check only fires on
	// the literal `true`.
	for yaml in [
		r#"services:
  web:
    image: alpine:3.20
    privileged: false
"#,
		r#"services:
  web:
    image: alpine:3.20
"#,
	] {
		let findings = report_for(yaml);
		assert!(
			!findings.iter().any(|f| f.check == "privileged"),
			"privileged check must not fire on the good shape: {findings:#?}"
		);
	}
}

// ---------------------------------------------------------------------------
// dangerous_capability
// ---------------------------------------------------------------------------

#[test]
fn audit_dangerous_capability_flags_sys_admin_and_all() {
	for cap in ["SYS_ADMIN", "ALL"] {
		let yaml =
			format!("services:\n  web:\n    image: alpine:3.20\n    cap_add:\n      - {cap}\n");
		let findings = report_for(&yaml);
		assert!(
			findings.iter().any(|f| f.check == "dangerous_capability"),
			"`cap_add: [{cap}]` should flag dangerous_capability; got {findings:#?}"
		);
	}
}

#[test]
fn audit_dangerous_capability_passes_when_safe() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    cap_add: [CHOWN]
"#;
	let findings = report_for(yaml);
	assert!(
		!findings.iter().any(|f| f.check == "dangerous_capability"),
		"non-dangerous cap_add must not fire: {findings:#?}"
	);
}

// ---------------------------------------------------------------------------
// no_cap_drop_all
// ---------------------------------------------------------------------------

#[test]
fn audit_no_cap_drop_all_flags_when_absent() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
"#;
	let findings = report_for(yaml);
	assert!(
		findings.iter().any(|f| f.check == "no_cap_drop_all"),
		"absent cap_drop must fire: {findings:#?}"
	);
	// cap_drop without ALL is still bad, only the literal `ALL` zeroes
	// the baseline.
	let yaml_other = r#"
services:
  web:
    image: alpine:3.20
    cap_drop: [NET_RAW, MKNOD]
"#;
	let findings = report_for(yaml_other);
	assert!(
		findings.iter().any(|f| f.check == "no_cap_drop_all"),
		"cap_drop without ALL must fire: {findings:#?}"
	);
}

#[test]
fn audit_no_cap_drop_all_passes_when_all_present() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
    cap_drop: [ALL]
"#;
	let findings = report_for(yaml);
	assert!(
		!findings.iter().any(|f| f.check == "no_cap_drop_all"),
		"cap_drop: [ALL] must pass: {findings:#?}"
	);
}

// ---------------------------------------------------------------------------
// no_new_privileges_off
// ---------------------------------------------------------------------------

#[test]
fn audit_no_new_privileges_off_flags_when_missing() {
	let yaml = r#"
services:
  web:
    image: alpine:3.20
"#;
	let findings = report_for(yaml);
	assert!(
		findings.iter().any(|f| f.check == "no_new_privileges_off"),
		"absent security_opt must fire: {findings:#?}"
	);
}

#[test]
fn audit_no_new_privileges_off_passes_in_both_spellings() {
	// Podman spells it `no-new-privileges` (no `:true`); the docker form
	// carries `:true`. Both must count.
	for entry in ["no-new-privileges", "no-new-privileges:true"] {
		let yaml = format!(
			"services:\n  web:\n    image: alpine:3.20\n    security_opt:\n      - {entry}\n"
		);
		let findings = report_for(&yaml);
		assert!(
			!findings.iter().any(|f| f.check == "no_new_privileges_off"),
			"`{entry}` must pass the no-new-privileges-off check: {findings:#?}"
		);
	}
}

/// `CAP_SYS_ADMIN`, `sys_admin` and `SYS_ADMIN` are the same capability, and
/// compose files carry all three spellings; the check has to read them alike.
#[test]
fn audit_dangerous_capability_reads_every_spelling() {
	for spelling in ["CAP_SYS_ADMIN", "sys_admin", "cap_all", "ALL"] {
		let report = report_for(&format!(
			"services:\n  web:\n    image: alpine:3.20\n    cap_add: [{spelling}]\n"
		));
		assert!(
			report.iter().any(|f| f.check == "dangerous_capability"),
			"{spelling} was not read as dangerous"
		);
	}
}

/// `cap_drop: [all]` and `cap_drop: [CAP_ALL]` satisfy the check like `ALL`.
#[test]
fn audit_no_cap_drop_all_accepts_every_spelling() {
	for spelling in ["all", "CAP_ALL", "ALL"] {
		let report = report_for(&format!(
			"services:\n  web:\n    image: alpine:3.20\n    cap_drop: [{spelling}]\n"
		));
		assert!(
			!report.iter().any(|f| f.check == "no_cap_drop_all"),
			"{spelling} was not read as ALL"
		);
	}
}

/// `no-new-privileges:false` is the option switched off, not on.
#[test]
fn audit_no_new_privileges_off_flags_an_explicit_false() {
	let report = report_for(
		"services:\n  web:\n    image: alpine:3.20\n    security_opt: [\"no-new-privileges:false\"]\n",
	);
	assert!(report.iter().any(|f| f.check == "no_new_privileges_off"));
}

/// `security_opt: [no-new-privileges:true, no-new-privileges:false]`: the
/// engine last-wins (resolves to disabled), the unfixed audit iterates and
/// finds the first entry matches (resolves to enabled). The two must agree.
#[test]
fn audit_no_new_privileges_off_flags_contradictory_entries() {
	let report = report_for(
		"services:\n  web:\n    image: alpine:3.20\n    security_opt:\n      - no-new-privileges:true\n      - no-new-privileges:false\n",
	);
	assert!(
		report.iter().any(|f| f.check == "no_new_privileges_off"),
		"contradictory security_opt entries where the engine disables the \
		 protection must fire: {report:#?}"
	);
}

/// The runtime honours exactly what was asked for in `cap_add:`. The audit
/// must reject the curated dangerous list (`SYS_MODULE` and friends)
/// regardless of what the engine does with them.
#[test]
fn audit_dangerous_capability_flags_the_curated_list() {
	let report = report_for(
		"services:\n  web:\n    image: alpine:3.20\n    cap_add: [SYS_MODULE, DAC_READ_SEARCH, SYS_RAWIO]\n",
	);
	assert!(
		report.iter().any(|f| f.check == "dangerous_capability"),
		"SYS_MODULE / DAC_READ_SEARCH / SYS_RAWIO must fire: {report:#?}"
	);
	// Every entry in the curated list is its own finding, so the count
	// pins the list rather than just any-of.
	assert_eq!(
		report
			.iter()
			.filter(|f| f.check == "dangerous_capability")
			.count(),
		3,
		"three distinct dangerous_capability findings expected: {report:#?}"
	);
}
