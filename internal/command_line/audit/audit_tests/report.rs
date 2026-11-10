use super::*;

// ---------------------------------------------------------------------------
// The report and how it groups its findings.
// ---------------------------------------------------------------------------

/// `has_findings` is what `--strict` reads; a mutation sweep replaced it with
/// a constant and only the binary tests noticed.
#[test]
fn audit_report_has_findings_follows_the_list() {
	let clean = report_for_file("services:\n  web:\n    image: alpine:3.20\n    read_only: true\n    cap_drop: [ALL]\n    security_opt: [no-new-privileges:true]\n    pids_limit: 64\n    mem_limit: 64m\n    memswap_limit: 64m\n    init: true\n    restart: unless-stopped\n    cpus: \"1\"\n    healthcheck:\n      test: [\"CMD\", \"true\"]\n      x-podman-on-failure: restart\n    userns_mode: auto\n");
	assert!(!clean.has_findings());
	let dirty = report_for_file("services:\n  web:\n    image: alpine\n");
	assert!(dirty.has_findings());
}

/// `by_service` hands each service exactly its own findings, in file order,
/// including a service with none.
#[test]
fn audit_report_by_service_keeps_file_order_and_ownership() {
	let file = parse_str(
		"services:\n  zeta:\n    image: alpine:3.20\n    privileged: true\n  alpha:\n    image: alpine:3.20\n    read_only: true\n    cap_drop: [ALL]\n    security_opt: [no-new-privileges:true]\n    pids_limit: 64\n    mem_limit: 64m\n    memswap_limit: 64m\n    init: true\n    restart: unless-stopped\n    cpus: \"1\"\n    healthcheck:\n      test: [\"CMD\", \"true\"]\n      x-podman-on-failure: restart\n    userns_mode: auto\n",
	)
	.unwrap();
	let report = audit_file(&file, &[]);
	let services = audited_containers(&file);
	assert_eq!(
		services.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
		vec!["zeta", "alpha"]
	);
	let grouped = report.by_service(&services);
	assert_eq!(grouped.len(), 2);
	assert_eq!(grouped[0].0, "zeta");
	assert!(grouped[0].1.iter().all(|f| f.service == "zeta"));
	assert!(grouped[0].1.iter().any(|f| f.check == "privileged"));
	assert_eq!(grouped[1].0, "alpha");
	assert!(grouped[1].1.is_empty());
}
