use super::*;

#[test]
fn warns_on_unmapped_build_fields() {
	let msgs = diagnostics_for(
			"services:\n  web:\n    build:\n      context: .\n      privileged: true\n      isolation: chroot\n",
		);
	assert!(msgs.iter().any(|m| m.contains("build.privileged")));
	assert!(msgs.iter().any(|m| m.contains("build.isolation")));
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
