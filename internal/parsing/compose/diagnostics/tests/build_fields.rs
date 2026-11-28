use super::*;

#[test]
fn warns_on_unmapped_build_fields() {
	let msgs = diagnostics_for(
		"services:\n  web:\n    build:\n      context: .\n      privileged: true\n",
	);
	assert!(msgs.iter().any(|m| m.contains("build.privileged")));
}

#[test]
fn does_not_report_build_isolation_as_unmapped() {
	// The libpod build endpoint reads `isolation`, so podup forwards it.
	// Reporting it as unmapped would send the reader looking for a workaround
	// that is not needed.
	let msgs = diagnostics_for(
		"services:\n  web:\n    build:\n      context: .\n      isolation: chroot\n",
	);
	assert!(
		!msgs.iter().any(|m| m.contains("build.isolation")),
		"build.isolation is forwarded now and must not be reported as ignored; got: {msgs:?}"
	);
}

#[test]
fn warns_on_build_ssh() {
	let msgs = diagnostics_for(
		"services:\n  web:\n    build:\n      context: .\n      ssh:\n        - default\n",
	);
	assert!(
		msgs.iter().any(|m| m.contains("build.ssh")),
		"got: {msgs:?}"
	);
}

#[test]
fn warns_on_remaining_unmapped_build_fields() {
	let msgs = diagnostics_for(
			"services:\n  web:\n    build:\n      context: .\n      ulimits:\n        nofile: 1024\n      entitlements: [\"security.insecure\"]\n      provenance: true\n      sbom: true\n",
		);
	for field in ["build.entitlements", "build.provenance", "build.sbom"] {
		assert!(
			msgs.iter().any(|m| m.contains(field)),
			"missing {field}; got: {msgs:?}"
		);
	}
	// `build.ulimits` was on this list, wrongly: the libpod build endpoint takes
	// a `ulimits` parameter and applies it. Measured on podman 5.7.0: the same
	// build saw `ulimit -n` of 524288 without it and 1234 with it. Reporting it
	// as unmapped now would send the reader looking for a workaround that is not
	// needed.
	assert!(
		!msgs.iter().any(|m| m.contains("build.ulimits")),
		"build.ulimits is mapped now and must not be reported as ignored; got: {msgs:?}"
	);
}

#[test]
fn says_why_build_sbom_is_not_honored() {
	// The endpoint does have SBOM parameters, but they take a scanner image, a
	// scanner command and an output destination, and the compose value names
	// none of them. Saying "no libpod build-API equivalent" would be wrong: the
	// reason is what the value is missing.
	let msgs =
		diagnostics_for("services:\n  web:\n    build:\n      context: .\n      sbom: true\n");
	let sbom: Vec<_> = msgs.iter().filter(|m| m.contains("build.sbom")).collect();
	assert_eq!(sbom.len(), 1, "got: {msgs:?}");
	assert!(
		!sbom[0].contains("no libpod build-API equivalent"),
		"the reason must name what the value is missing, not deny the API: {sbom:?}"
	);
	assert!(
		sbom[0].contains("sbom-scanner-image") && sbom[0].contains("sbom-output"),
		"the reason must name the parameters the endpoint takes: {sbom:?}"
	);
}

#[test]
fn warns_once_per_service_on_no_cache_filter() {
	let msgs = diagnostics_for(
		"services:\n  web:\n    build:\n      context: .\n      no_cache_filter: builder\n  api:\n    build:\n      context: .\n      no_cache_filter: ''\n",
	);
	let matching: Vec<_> = msgs
		.iter()
		.filter(|message| message.contains("build.no_cache_filter"))
		.collect();
	assert_eq!(matching.len(), 2, "got: {msgs:?}");
	for service in ["web", "api"] {
		assert_eq!(
			matching
				.iter()
				.filter(|message| message.contains(&format!("service '{service}'")))
				.count(),
			1,
			"got: {msgs:?}"
		);
	}
	assert!(
		matching
			.iter()
			.all(|message| message.contains("cannot filter the cache per stage")),
		"got: {msgs:?}"
	);
}
