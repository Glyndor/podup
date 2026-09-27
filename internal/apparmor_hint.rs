//! Hint printed under the libpod error that means AppArmor's `pasta` profile
//! is refusing a signal from `podman`.

/// The text libpod puts in the error when AppArmor refuses the signal.
pub(crate) const PASTA_SIGNAL_DENIED: &str =
	"rootless netns: kill network process: permission denied";

/// The rule the `pasta` profile needs.
pub(crate) const PASTA_RULE: &str = "signal (receive) peer=podman,";

/// The line after which the one-liner inserts the rule. Must match the sed
/// pattern in `sed_command` exactly (two leading spaces).
const ANCHOR: &str = "  include <abstractions/pasta>";

/// Outcome of inspecting `/etc/apparmor.d` for a profile named `pasta` that
/// does or does not allow the signal Podman needs.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PastaProfile {
	/// No file under the directory defines a profile named `pasta`.
	NotFound,
	/// The file defining `pasta` already allows the signal.
	HasRule,
	/// The file defining `pasta` lacks the rule. `has_anchor` says whether it
	/// contains a line exactly equal to ANCHOR.
	Missing {
		path: std::path::PathBuf,
		has_anchor: bool,
	},
}

/// Whether `contents` defines an AppArmor profile whose name is `pasta`.
///
/// True when some line, after `trim_start()`, starts with `profile pasta` and
/// the next character is whitespace or `{`. A comment starts with `#`, so it
/// can never match.
pub(crate) fn defines_pasta(contents: &str) -> bool {
	for raw in contents.lines() {
		let line = raw.trim_start();
		let Some(after) = line.strip_prefix("profile pasta") else {
			continue;
		};
		let next = after.chars().next();
		if matches!(next, Some(' ' | '\t' | '{')) {
			return true;
		}
	}
	false
}

/// Whether a non-comment line of `contents` grants `signal (receive)` to
/// `podman` (or, if no `peer=` is given, to every peer, which still unblocks
/// the failed call). A comment starts with `#`, so it can never match.
pub(crate) fn has_rule(contents: &str) -> bool {
	for raw in contents.lines() {
		let line = raw.trim_start();
		let Some(after) = line.strip_prefix("signal") else {
			continue;
		};
		// The character after `signal` must be whitespace, `{`, `,`, or `(`,
		// otherwise `signals`/`signaler` would match. `(`, `{`, and `,` are
		// valid AppArmor syntax starts for a bare rule; whitespace is the
		// usual form (`signal (receive) ...`).
		let Some(next) = after.chars().next() else {
			continue;
		};
		if !matches!(next, ' ' | '\t' | '(' | '{' | ',') {
			continue;
		}
		if line.contains("peer=podman") || !line.contains("peer=") {
			return true;
		}
	}
	false
}

/// Classify `contents` against the three states a file can be in relative to
/// the missing AppArmor rule.
pub(crate) fn classify(path: &std::path::Path, contents: &str) -> Option<PastaProfile> {
	if !defines_pasta(contents) {
		return None;
	}
	if has_rule(contents) {
		return Some(PastaProfile::HasRule);
	}
	let has_anchor = contents.lines().any(|l| l == ANCHOR);
	Some(PastaProfile::Missing {
		path: path.to_path_buf(),
		has_anchor,
	})
}

/// Read `dir`, looking for the first regular file that defines a profile
/// named `pasta`. Files that cannot be read (not UTF-8, permission denied)
/// are skipped; on any directory error, or when no candidate matches,
/// returns `NotFound`. Entries are sorted by name so the result does not
/// depend on the filesystem order.
pub(crate) fn find_pasta_profile(dir: &std::path::Path) -> PastaProfile {
	let entries = match std::fs::read_dir(dir) {
		Ok(it) => it,
		Err(_) => return PastaProfile::NotFound,
	};
	// `metadata` follows symlinks, so a link to a profile file counts and a
	// link to a directory (or `abstractions/` itself) does not.
	let mut files: Vec<std::path::PathBuf> = entries
		.flatten()
		.map(|entry| entry.path())
		.filter(|path| std::fs::metadata(path).is_ok_and(|m| m.is_file()))
		.collect();
	files.sort();
	for path in files {
		let Ok(contents) = std::fs::read_to_string(&path) else {
			continue;
		};
		if let Some(profile) = classify(&path, &contents) {
			return profile;
		}
	}
	PastaProfile::NotFound
}

/// Build the one-line `sed` command that inserts [`PASTA_RULE`] after
/// [`ANCHOR`]. Returns `None` when `path` has a character outside
/// `[A-Za-z0-9._/-]`: the path is printed unquoted, so anything a shell could
/// read differently gets the manual instructions instead.
pub(crate) fn sed_command(path: &std::path::Path) -> Option<String> {
	let s = path.to_str().filter(|s| !s.is_empty())?;
	if !s
		.bytes()
		.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'/' | b'-'))
	{
		return None;
	}
	Some(format!(
		"sudo sed -i 's|^{ANCHOR}$|&\\n  {PASTA_RULE}|' {s}"
	))
}

const CAUSE: &str = "hint: AppArmor's `pasta` profile does not let Podman stop the rootless \
	network (https://bugs.debian.org/1100135).";

/// The hint for `profile`, or `None` when the profile already allows the
/// signal and the failure must have another cause.
pub(crate) fn render_hint(profile: &PastaProfile) -> Option<String> {
	match profile {
		PastaProfile::HasRule => None,
		PastaProfile::Missing { path, has_anchor } => {
			let reload = format!("sudo apparmor_parser -r {}", path.display());
			match has_anchor.then(|| sed_command(path)).flatten() {
				Some(sed) => Some(format!(
					"{CAUSE} Allow it once, then retry:\n    {sed}\n    {reload}"
				)),
				None => Some(format!(
					"{CAUSE} Add the line `{PASTA_RULE}` inside the `pasta` profile in {}, \
					 then reload it with `{reload}`.",
					path.display()
				)),
			}
		}
		PastaProfile::NotFound => Some(format!(
			"hint: this usually means an AppArmor profile named `pasta` refuses the signal \
			 Podman sends to stop the rootless network (https://bugs.debian.org/1100135). \
			 On the host running Podman, allow `{PASTA_RULE}` in that profile and reload it."
		)),
	}
}

/// The hint for `error_text` when it is the libpod error this module is about,
/// read against the profiles on this host. Any other error returns `None`
/// without touching the filesystem.
pub(crate) fn hint_for(error_text: &str) -> Option<String> {
	#[cfg(target_os = "linux")]
	let dir = Some(std::path::Path::new("/etc/apparmor.d"));
	#[cfg(not(target_os = "linux"))]
	let dir = None;
	hint_in(error_text, dir)
}

/// [`hint_for`] with the profile directory as a parameter; `None` means the
/// platform has no AppArmor to look at.
pub(crate) fn hint_in(error_text: &str, dir: Option<&std::path::Path>) -> Option<String> {
	if !error_text.contains(PASTA_SIGNAL_DENIED) {
		return None;
	}
	render_hint(&dir.map_or(PastaProfile::NotFound, find_pasta_profile))
}

#[cfg(test)]
#[path = "apparmor_hint_tests.rs"]
mod tests;
