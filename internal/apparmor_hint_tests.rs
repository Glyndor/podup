use super::{
	classify, defines_pasta, find_pasta_profile, has_rule, hint_for, hint_in, render_hint,
	PastaProfile,
};
use std::fs;
use std::path::Path;

const UBUNTU_PROFILE: &str = "\
abi <abi/4.0>,\n\
\n\
include <tunables/global>\n\
\n\
profile pasta /usr/bin/pasta{,.avx2} flags=(attach_disconnected) {\n\
\x20\x20include <abstractions/pasta>\n\
\n\
\x20\x20/tmp/**\t\t\t\trw,\n\
\x20\x20owner @{HOME}/**\t\t\tw,\n\
}\n";

#[test]
fn defines_pasta_matches_real_ubuntu_line() {
	assert!(defines_pasta(
		"profile pasta /usr/bin/pasta{,.avx2} flags=(attach_disconnected) {"
	));
}

#[test]
fn defines_pasta_rejects_similar_profile_names() {
	assert!(!defines_pasta("profile pastabar /x {"));
}

#[test]
fn defines_pasta_rejects_commented_out_definition() {
	assert!(!defines_pasta("# profile pasta /x {"));
}

#[test]
fn has_rule_accepts_rule_for_podman() {
	assert!(has_rule("signal (receive) peer=podman,"));
}

#[test]
fn has_rule_accepts_rule_without_peer() {
	assert!(has_rule("signal,"));
}

#[test]
fn has_rule_rejects_rule_for_other_peer() {
	assert!(!has_rule("signal (receive) peer=unconfined,"));
}

#[test]
fn has_rule_rejects_commented_rule() {
	assert!(!has_rule("# signal (receive) peer=podman,"));
}

#[test]
fn classify_returns_none_when_profile_not_defined() {
	let contents = "# no pasta here\n  signal (receive) peer=podman,\n";
	assert_eq!(classify(Path::new("/etc/apparmor.d/x"), contents), None);
}

fn write(dir: &Path, name: &str, contents: &str) -> std::path::PathBuf {
	let path = dir.join(name);
	fs::write(&path, contents).expect("write fixture");
	path
}

#[test]
fn find_pasta_profile_returns_missing_with_anchor() {
	let dir = tempfile::tempdir().expect("tempdir");
	// Unrelated file.
	write(dir.path(), "aaa", "profile aaa /bin/aaa {\n}\n");
	// Subdirectory containing a profile named pasta must be skipped.
	let sub = dir.path().join("abstractions");
	fs::create_dir(&sub).expect("mkdir abstractions");
	write(&sub, "pasta", UBUNTU_PROFILE);
	// The real file defines pasta but lacks the rule and has the anchor.
	let profile_path = write(dir.path(), "usr.bin.pasta", UBUNTU_PROFILE);
	let got = find_pasta_profile(dir.path());
	assert_eq!(
		got,
		PastaProfile::Missing {
			path: profile_path,
			has_anchor: true,
		}
	);
}

#[test]
fn find_pasta_profile_returns_has_rule_after_appending_rule() {
	let dir = tempfile::tempdir().expect("tempdir");
	let mut contents = UBUNTU_PROFILE.to_string();
	contents.push_str("  signal (receive) peer=podman,\n");
	write(dir.path(), "usr.bin.pasta", &contents);
	assert_eq!(find_pasta_profile(dir.path()), PastaProfile::HasRule);
}

#[test]
fn find_pasta_profile_returns_not_found_for_missing_dir() {
	let dir = tempfile::tempdir().expect("tempdir");
	let missing = dir.path().join("does-not-exist");
	assert_eq!(find_pasta_profile(&missing), PastaProfile::NotFound);
}

#[test]
fn render_hint_for_missing_with_anchor_emits_one_liner() {
	let path = Path::new("/etc/apparmor.d/usr.bin.pasta");
	let expected = "hint: AppArmor's `pasta` profile does not let Podman stop the rootless \
		 network (https://bugs.debian.org/1100135). Allow it once, then retry:\n\
		 \x20\x20\x20\x20sudo sed -i 's|^  include <abstractions/pasta>$|&\\n  signal \
		 (receive) peer=podman,|' /etc/apparmor.d/usr.bin.pasta\n\
		 \x20\x20\x20\x20sudo apparmor_parser -r /etc/apparmor.d/usr.bin.pasta"
		.to_string();
	assert_eq!(
		render_hint(&PastaProfile::Missing {
			path: path.to_path_buf(),
			has_anchor: true,
		}),
		Some(expected),
	);
}

#[test]
fn render_hint_for_missing_without_anchor_omits_sed() {
	let path = Path::new("/etc/apparmor.d/usr.bin.pasta");
	let hint = render_hint(&PastaProfile::Missing {
		path: path.to_path_buf(),
		has_anchor: false,
	})
	.expect("hint");
	assert!(hint.contains("/etc/apparmor.d/usr.bin.pasta"));
	assert!(hint.contains("signal (receive) peer=podman,"));
	assert!(!hint.contains("sudo sed"));
}

#[test]
fn render_hint_rejects_paths_with_spaces() {
	let path = Path::new("/tmp/a b/pasta");
	let hint = render_hint(&PastaProfile::Missing {
		path: path.to_path_buf(),
		has_anchor: true,
	})
	.expect("hint");
	assert!(!hint.contains("sudo sed"));
}

#[test]
fn render_hint_is_none_when_rule_already_present() {
	assert_eq!(render_hint(&PastaProfile::HasRule), None);
}

#[test]
fn hint_for_returns_none_for_unrelated_error() {
	assert_eq!(hint_for("podman API error (HTTP 500): boom"), None);
}

// Linux only: the hint prints GNU sed syntax, and BSD `sed -i` reads the
// script as a backup suffix. AppArmor is Linux-only anyway.
#[test]
#[cfg(target_os = "linux")]
fn sed_one_liner_actually_adds_the_rule() {
	use super::{sed_command, ANCHOR};

	let dir = tempfile::tempdir().expect("tempdir");
	let profile_path = write(dir.path(), "usr.bin.pasta", UBUNTU_PROFILE);
	let sed = sed_command(&profile_path).expect("sed command");
	// Run what the user is told to run, minus the privilege escalation.
	let status = std::process::Command::new("sh")
		.arg("-c")
		.arg(sed.strip_prefix("sudo ").expect("printed with sudo"))
		.status()
		.expect("spawn sh");
	assert!(status.success(), "sed one-liner failed: {status}");
	let after = fs::read_to_string(&profile_path).expect("read back");
	assert!(
		has_rule(&after),
		"rule was not added by the printed command"
	);
	assert!(defines_pasta(&after), "the profile definition was lost");
	assert!(after.contains(ANCHOR));
	assert_eq!(
		find_pasta_profile(dir.path()),
		PastaProfile::HasRule,
		"after the printed fix the hint must go quiet"
	);
}

/// The error as `podup restart` printed it on the host where #1941 was measured.
const LIVE_ERROR: &str = "podman error: podman API error (HTTP 500): 1 error occurred:\n\
	\t* rootless netns: kill network process: permission denied\n\n";

// Linux only: a temporary directory elsewhere (`C:\Users\RUNNER~1\...` on
// Windows) has characters the one-liner will not print unquoted, so the hint
// correctly falls back to the manual text there.
#[test]
#[cfg(target_os = "linux")]
fn hint_in_names_the_profile_for_the_live_error() {
	let dir = tempfile::tempdir().expect("tempdir");
	let profile_path = write(dir.path(), "usr.bin.pasta", UBUNTU_PROFILE);
	let hint = hint_in(LIVE_ERROR, Some(dir.path())).expect("hint for the live error");
	assert!(
		hint.contains(&format!(
			"sudo apparmor_parser -r {}",
			profile_path.display()
		)),
		"{hint}"
	);
	assert!(hint.contains("sudo sed -i"), "{hint}");
}

#[test]
fn hint_in_is_silent_when_the_rule_is_already_there() {
	let dir = tempfile::tempdir().expect("tempdir");
	write(
		dir.path(),
		"usr.bin.pasta",
		&format!("{UBUNTU_PROFILE}  signal (receive) peer=podman,\n"),
	);
	assert_eq!(hint_in(LIVE_ERROR, Some(dir.path())), None);
}

#[test]
fn hint_in_without_apparmor_explains_without_a_command() {
	let hint = hint_in(LIVE_ERROR, None).expect("hint");
	assert!(hint.contains("1100135"), "{hint}");
	assert!(!hint.contains("sudo"), "{hint}");
}

#[test]
fn hint_in_ignores_other_errors_even_with_a_profile_present() {
	let dir = tempfile::tempdir().expect("tempdir");
	write(dir.path(), "usr.bin.pasta", UBUNTU_PROFILE);
	assert_eq!(
		hint_in(
			"podman API error (HTTP 500): permission denied",
			Some(dir.path())
		),
		None
	);
}
