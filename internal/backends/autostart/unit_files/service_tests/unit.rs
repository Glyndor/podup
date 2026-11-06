use super::*;

// --- what the unit file says ---------------------------------------------

#[test]
fn renders_single_file_unit() {
	let s = render_service_unit(&opts_single());
	assert!(s.contains("Description=podup app"));
	assert!(s.contains("Type=oneshot"));
	assert!(s.contains("RemainAfterExit=yes"));
	assert!(s.contains("WorkingDirectory=/srv/app"));
	assert!(s.contains("WantedBy=default.target"));
	assert!(
		s.contains(
			"ExecStart=/usr/local/bin/podup -f /srv/app/docker-compose.yml -p app up -d --no-build --pull never"
		)
	);
	assert!(s.contains("ExecStop=/usr/local/bin/podup -f /srv/app/docker-compose.yml -p app stop"));
}

#[test]
fn boot_neither_builds_nor_destroys() {
	// The contract, pinned. `--build` on ExecStart puts an image build on the
	// boot path of an unattended machine: it needs the network and a briefly
	// unreachable registry leaves the stack down. A pull at boot has the same
	// failure, and `--no-build` / `--pull never` are what keep either off the
	// boot path (#1951). `down` on ExecStop removes the containers, so a clean
	// shutdown would delete the stack and every boot would recreate it. None
	// of these may come back without this test being deleted on purpose.
	let s = render_service_unit(&opts_single());
	assert!(
		!s.contains("--build"),
		"a boot must not depend on a build:\n{s}"
	);
	assert!(
		s.contains("--pull never"),
		"a boot must not pull an image:\n{s}"
	);
	assert!(
		!s.contains(" down"),
		"ExecStop must stop, not remove the containers:\n{s}"
	);
}

/// The whole line, not a substring: `contains("--no-build")` would still pass
/// with the flag on ExecStop.
#[test]
fn boot_exec_start_is_exactly_pinned() {
	let s = render_service_unit(&opts_single());
	let exec_start_line = s
		.lines()
		.find(|l| l.starts_with("ExecStart="))
		.expect("ExecStart line is present");
	assert_eq!(
		exec_start_line,
		"ExecStart=/usr/local/bin/podup -f /srv/app/docker-compose.yml -p app up -d --no-build --pull never"
	);
}

/// #1616: service mode writes the final unit, so it inherits none of the
/// ordering Quadlet's generator adds on its own. What has to hold is that the
/// unit waits for the network, and that it waits through Podman's user-scope
/// shim rather than through `network-online.target`, which is inert in the
/// `--user` instance.
///
/// The assertion this replaced only forbade the string `network-online.target`,
/// which a unit carrying no ordering at all also satisfies. That is the state
/// this test exists to rule out, so it asserts the keys that must be present
/// first and the spelling that must not appear second.
#[test]
fn orders_against_the_user_scope_network_shim() {
	let s = render_service_unit(&opts_single());
	assert!(
		s.contains("Wants=podman-user-wait-network-online.service"),
		"the unit must pull in the network shim:\n{s}"
	);
	assert!(
		s.contains("After=podman-user-wait-network-online.service"),
		"wanting the shim without ordering after it starts both at once, \
		 which is the same as not waiting:\n{s}"
	);
	for key in ["Wants=", "After=", "Requires=", "BindsTo=", "PartOf="] {
		assert!(
			!s.contains(&format!("{key}network-online.target")),
			"a `--user` unit must not depend on the system target directly, \
			 since it never activates there:\n{s}"
		);
	}
}

#[test]
fn ends_with_newline() {
	assert!(render_service_unit(&opts_single()).ends_with("WantedBy=default.target\n"));
}
