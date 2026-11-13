use super::super::tests::report_for;

// ---------------------------------------------------------------------------
// unconfined_security_opt
// ---------------------------------------------------------------------------

/// The reasons of the `unconfined_security_opt` findings for one entry.
fn unconfined(entry: &str) -> Vec<String> {
	let yaml =
		format!("services:\n  web:\n    image: alpine:3.20\n    security_opt: [\"{entry}\"]\n");
	report_for(&yaml)
		.into_iter()
		.filter(|f| f.check == "unconfined_security_opt")
		.map(|f| f.reason)
		.collect()
}

#[test]
fn audit_unconfined_security_opt_flags_each_layer_in_both_spellings() {
	for entry in [
		"seccomp=unconfined",
		"seccomp:unconfined",
		"apparmor=unconfined",
		"apparmor:unconfined",
		"label=disable",
		"label:disable",
		"systempaths=unconfined",
		"systempaths:unconfined",
	] {
		let reasons = unconfined(entry);
		assert_eq!(reasons.len(), 1, "{entry}: {reasons:#?}");
		assert!(reasons[0].contains(entry), "{entry}: {reasons:#?}");
	}
}

#[test]
fn audit_unconfined_security_opt_passes_on_a_profile_or_other_option() {
	for entry in [
		"seccomp=/etc/podman/seccomp.json",
		"seccomp:/etc/podman/seccomp.json",
		"apparmor=my-profile",
		"label=type:spc_t",
		"label=level:s0:c100,c200",
		"no-new-privileges:true",
		"unconfined",
	] {
		let reasons = unconfined(entry);
		assert!(reasons.is_empty(), "{entry}: {reasons:#?}");
	}
}
