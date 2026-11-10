use super::*;

// ---------------------------------------------------------------------------
// Jobs
// ---------------------------------------------------------------------------

/// A job is a container with the exposure of a service, so the same checks
/// run over it: a privileged job is a finding, reported under the job's name.
#[test]
fn a_privileged_job_is_a_finding_like_a_privileged_service() {
	let file = parse_str(
		"services:\n  web:\n    image: nginx\njobs:\n  risky:\n    image: alpine\n    privileged: true\n    triggers: {manual: true}\n",
	)
	.expect("compose parses");
	let names: Vec<String> = audited_containers(&file)
		.into_iter()
		.map(|c| c.name)
		.collect();
	assert_eq!(
		names,
		["web", "risky"],
		"the job is walked after the services"
	);
	let report = audit_file(&file, &[]);
	assert!(
		report
			.findings
			.iter()
			.any(|f| f.service == "risky" && f.check == "privileged"),
		"{:?}",
		report.findings
	);
}
