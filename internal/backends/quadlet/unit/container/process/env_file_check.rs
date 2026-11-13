//! What `podman --env-file` reads from a file, next to what `up` reads.
//!
//! `EnvironmentFile=` hands the file to `podman run --env-file`, which takes each
//! line as `KEY=VALUE` as written: no quotes removed, no `export`, no inline
//! comment, no `${VAR}`. `up` reads the same file as dotenv. A file that uses any
//! of that gives the container other values under Quadlet, so the keys that
//! differ are reported (never their values, which may be secrets).

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use crate::compose::types::EnvFileEntry;

/// The variables `podman --env-file` sets from `text`: a line is trimmed on the
/// left, skipped when empty or starting with `#`, and split at its first `=`.
/// A bare key imports the host's value, which the unit cannot know, so it is
/// left out of the comparison.
fn podman_reads(text: &str) -> HashMap<String, String> {
	text.lines()
		.map(|line| line.trim_start())
		.filter(|line| !line.is_empty() && !line.starts_with('#'))
		.filter_map(|line| line.split_once('='))
		.map(|(key, value)| (key.to_string(), value.to_string()))
		.collect()
}

/// The keys of `entry` that `podman --env-file` would set to another value than
/// `up`, sorted. Empty when the file cannot be read or parsed: a missing or
/// broken file is reported where it is loaded, not here.
pub(super) fn keys_read_differently(
	entry: &EnvFileEntry,
	path: &Path,
	base_dir: &Path,
) -> Vec<String> {
	let Ok(text) = std::fs::read_to_string(path) else {
		return Vec::new();
	};
	let Ok(up) = crate::env_file::load_env_file_entries(std::slice::from_ref(entry), base_dir)
	else {
		return Vec::new();
	};
	let podman = podman_reads(&text);
	let keys: BTreeSet<&String> = up.keys().chain(podman.keys()).collect();
	keys.into_iter()
		.filter(|key| up.get(*key) != podman.get(*key))
		// A bare key in the file is a host import for both; podman's value for
		// it is unknown here, so it is not counted as a difference.
		.filter(|key| podman.contains_key(*key) || !text.lines().any(|l| l.trim() == key.as_str()))
		.cloned()
		.collect()
}
