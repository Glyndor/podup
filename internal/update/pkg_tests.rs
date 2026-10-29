use super::*;

/// Paths here use `/` even for the Windows layouts. `Path::components` splits
/// on `\\` as well when it runs on Windows, so the component sequence these
/// assert on is the one the real code sees there; a fixture written with
/// `\\` would simply not parse into components on Linux and the tests would
/// pass by asserting nothing.
/// The configuration on the machine this was written on, verbatim from
/// `apt-config dump`. A covered machine, and the one shape the check would
/// have been accidentally fitted to if it were the only fixture.
const REAL_COVERED: &str = "\
Unattended-Upgrade::Allowed-Origins \"\";
Unattended-Upgrade::Allowed-Origins:: \"${distro_id}:${distro_codename}\";
Unattended-Upgrade::Allowed-Origins:: \"${distro_id}:${distro_codename}-security\";
Unattended-Upgrade::Allowed-Origins:: \"${distro_id}ESMApps:${distro_codename}-apps-security\";
Unattended-Upgrade::Allowed-Origins:: \"${distro_id}ESM:${distro_codename}-infra-security\";
Unattended-Upgrade::Allowed-Origins:: \"Glyndor:stable\";
Unattended-Upgrade::DevRelease \"auto\";
";

#[test]
fn the_configuration_our_keyring_writes_reads_as_permitted() {
	assert_eq!(
		glyndor_auto_update(REAL_COVERED),
		GlyndorAutoUpdate::Permitted
	);
}

/// The case the first draft of this check would have got wrong. The package's
/// own README documents `Allowed-Origins` **or** `Origins-Pattern`, so an
/// operator using the second is covered, and a check that knew only about
/// the first would have told them nothing will ever update them while their
/// machine updated itself fine.
#[test]
fn an_origins_pattern_machine_is_covered_too() {
	let dump = "\
Unattended-Upgrade::Allowed-Origins:: \"${distro_id}:${distro_codename}-security\";
Unattended-Upgrade::Origins-Pattern:: \"origin=Glyndor\";
";
	assert_eq!(glyndor_auto_update(dump), GlyndorAutoUpdate::Permitted);
}

/// Allowed origin and a vetoed package: an allowlist that looks perfect on a
/// machine that will still never update podup.
#[test]
fn a_blacklisted_package_is_blocked_despite_an_allowed_origin() {
	let dump = "\
Unattended-Upgrade::Allowed-Origins:: \"Glyndor:stable\";
Unattended-Upgrade::Package-Blacklist:: \"podup\";
";
	let GlyndorAutoUpdate::Blocked(why) = glyndor_auto_update(dump) else {
		panic!("a blacklisted podup must be Blocked");
	};
	assert!(
		why.contains("Package-Blacklist"),
		"the reason must name the list that is doing it: {why}"
	);
}

/// The machine #1602 is about: unattended-upgrades configured and running,
/// Glyndor in neither list, podup never updated and never told.
#[test]
fn a_machine_with_no_glyndor_rule_is_blocked_and_told_the_remedy() {
	let dump = "\
Unattended-Upgrade::Allowed-Origins:: \"${distro_id}:${distro_codename}\";
Unattended-Upgrade::Allowed-Origins:: \"${distro_id}:${distro_codename}-security\";
";
	let GlyndorAutoUpdate::Blocked(why) = glyndor_auto_update(dump) else {
		panic!("a machine with no Glyndor rule must be Blocked");
	};
	assert!(
		why.contains("glyndor-archive-keyring") && why.contains("apt upgrade"),
		"a warning without the remedy is one the reader can do nothing with: {why}"
	);
}

/// No rule of either kind: unattended-upgrades is not configured, or the
/// output is not what this understands. Saying nothing beats guessing, and
/// `Unknown` is what the caller renders as silence.
#[test]
fn a_machine_with_no_rules_at_all_is_unknown_rather_than_blocked() {
	assert_eq!(
		glyndor_auto_update("APT::Architecture \"amd64\";\n"),
		GlyndorAutoUpdate::Unknown
	);
	assert_eq!(glyndor_auto_update(""), GlyndorAutoUpdate::Unknown);
}

#[test]
fn homebrew_layouts_are_recognised_on_every_prefix() {
	for p in [
		"/opt/homebrew/Cellar/podup/5.4.0/bin/podup",
		"/usr/local/Cellar/podup/5.4.0/bin/podup",
		"/home/linuxbrew/.linuxbrew/Cellar/podup/5.4.0/bin/podup",
		"/somewhere/entirely/custom/Cellar/podup/5.4.0/bin/podup",
	] {
		assert!(homebrew_owns(Path::new(p)), "not detected as Homebrew: {p}");
	}
}

#[test]
fn ordinary_layouts_are_not_mistaken_for_homebrew() {
	for p in [
		"/usr/local/bin/podup",
		"/home/me/.cargo/bin/podup",
		"/usr/bin/podup",
		"/home/me/podup/target/release/podup",
	] {
		assert!(
			!homebrew_owns(Path::new(p)),
			"falsely detected as Homebrew: {p}"
		);
	}
}

#[test]
fn scoop_layouts_are_recognised_without_the_variable() {
	for p in [
		"/c/Users/me/scoop/apps/podup/5.4.0/podup.exe",
		"/c/Users/me/scoop/shims/podup.exe",
		"/c/ProgramData/scoop/apps/podup/current/podup.exe",
	] {
		assert!(
			scoop_owns_under(Path::new(p), None),
			"not detected as Scoop: {p}"
		);
	}
}

#[test]
fn a_relocated_scoop_root_is_recognised_through_the_variable() {
	// The fallback cannot see this one: no component is named `scoop`.
	let exe = Path::new("/d/tools/apps/podup/5.4.0/podup.exe");
	assert!(
		!scoop_owns_under(exe, None),
		"the fallback should not match a root with no `scoop` component"
	);
	assert!(
		scoop_owns_under(exe, Some(Path::new("/d/tools"))),
		"a root named by SCOOP must be honoured"
	);
}

#[test]
fn ordinary_layouts_are_not_mistaken_for_scoop() {
	for p in [
		"/c/Program Files/podup/podup.exe",
		"/home/me/.cargo/bin/podup",
		// `scoop` present but not followed by apps or shims: a checkout of
		// the bucket repository, not an install.
		"/home/me/src/scoop/bucket/podup.json",
	] {
		assert!(
			!scoop_owns_under(Path::new(p), None),
			"falsely detected as Scoop: {p}"
		);
	}
}

#[test]
fn the_error_tells_each_manager_its_own_command() {
	for (pm, cmd) in [
		("apt", "apt upgrade podup"),
		("Homebrew", "brew upgrade podup"),
		("Scoop", "scoop update podup"),
	] {
		let ComposeError::Update(msg) = package_managed_error(pm) else {
			panic!("expected an Update error");
		};
		assert!(
			msg.contains(pm),
			"{pm} is not named in its own error: {msg}"
		);
		assert!(
			msg.contains(cmd),
			"{pm} is told to run something other than {cmd}: {msg}"
		);
	}
}

#[test]
fn package_managed_error_names_the_manager() {
	let e = package_managed_error("apt");
	match e {
		ComposeError::Update(msg) => {
			assert!(msg.contains("apt"));
			assert!(msg.contains("podup update"));
		}
		_ => panic!("expected an Update error"),
	}
}

/// An unavailable query is uncertainty, not proof that apt does not own the file.
#[cfg(all(unix, target_os = "linux"))]
#[test]
fn dpkg_owns_errors_when_dpkg_query_is_missing_from_path() {
	let empty_path = std::path::PathBuf::from("/nonexistent-empty-path-for-dpkg-test");
	let fake_target = Path::new("/usr/bin/podup");
	temp_env::with_var("PATH", Some(empty_path.display().to_string()), || {
		// Refused where a dpkg database exists, allowed where there is no dpkg.
		assert_eq!(
			dpkg_owns(fake_target).is_err(),
			Path::new("/var/lib/dpkg/status").exists()
		);
	});
}

#[cfg(all(unix, target_os = "linux"))]
#[test]
fn dpkg_query_distinguishes_absent_ownership_from_query_failure() {
	use std::os::unix::process::ExitStatusExt;
	use std::process::{ExitStatus, Output};
	for (status, owned) in [
		(0, Some(true)),
		(1 << 8, Some(false)),
		(2 << 8, None),
		(9, None),
	] {
		let result = dpkg_query_result(
			Ok(Output {
				status: ExitStatus::from_raw(status),
				stdout: Vec::new(),
				stderr: b"query diagnostic".to_vec(),
			}),
			true,
		);
		match owned {
			Some(owned) => assert_eq!(result.unwrap(), owned),
			None => assert!(result.is_err()),
		}
	}
	// Denied, or missing where a dpkg database exists: unknown, so refuse.
	for kind in [
		std::io::ErrorKind::NotFound,
		std::io::ErrorKind::PermissionDenied,
	] {
		assert!(dpkg_query_result(Err(std::io::Error::from(kind)), true).is_err());
	}
	assert!(dpkg_query_result(
		Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
		false
	)
	.is_err());
}

/// A machine with no dpkg at all (Fedora, Arch) has no dpkg-managed binary, so
/// the update must not be blocked there.
#[cfg(all(unix, target_os = "linux"))]
#[test]
fn a_machine_without_dpkg_does_not_block_the_update() {
	let missing = Err(std::io::Error::from(std::io::ErrorKind::NotFound));
	assert!(!dpkg_query_result(missing, false).unwrap());
}
