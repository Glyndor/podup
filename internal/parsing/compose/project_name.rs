//! The name a project runs under, and its exposure to the compose file.
//!
//! The name comes from the first of four sources that supplies one: the `-p`
//! flag, the `COMPOSE_PROJECT_NAME` variable, the top-level `name:` of the
//! file, and the name of the project directory. The file may also refer to the
//! result as `${COMPOSE_PROJECT_NAME}`, so the name has to be settled before
//! the rest of the file is interpolated.

use std::path::{Path, PathBuf};

use super::merge;
use super::parsing::{dir_of, is_stdin};
use crate::substitute;

/// The variable that names the project, and that the file may interpolate.
const PROJECT_NAME_VAR: &str = "COMPOSE_PROJECT_NAME";

/// Resolve the base directory for relative-path resolution. An explicit
/// `--project-directory` wins; otherwise it is the directory containing the
/// compose file (compose-spec default), or the current directory when the
/// compose file has no parent component.
pub fn resolve_base_dir(project_directory: Option<&Path>, file: &Path) -> PathBuf {
	// Always absolute: a relative base would reach the Podman service as a
	// relative bind source and be resolved against the service's own working
	// directory, so `-f deploy/compose.yml` mounted `$HOME/deploy/data`.
	let anchor = |dir: &Path| {
		if dir.is_absolute() {
			dir.to_path_buf()
		} else {
			std::env::current_dir().unwrap_or_default().join(dir)
		}
	};
	if let Some(dir) = project_directory {
		return anchor(dir);
	}
	match file.parent() {
		Some(parent) if !parent.as_os_str().is_empty() => anchor(parent),
		// A bare compose filename (e.g. `docker-compose.yml`) has an empty parent
		// component. Anchor relative paths to the working directory so a relative
		// `file:` secret/config or bind source resolves against the project
		// directory, not the working directory the Podman service later runs in.
		_ => std::env::current_dir().unwrap_or_default(),
	}
}

/// Resolve the project name following the compose-spec precedence: an explicit
/// `-p` / `COMPOSE_PROJECT_NAME` value, then the top-level `name:` field, then
/// the sanitized basename of the project directory. Explicit values are taken
/// verbatim; only the directory basename is sanitized.
pub fn resolve_project_name(
	explicit: Option<String>,
	compose_name: Option<&str>,
	base_dir: &Path,
) -> String {
	// An empty `-p ""` or `COMPOSE_PROJECT_NAME=` is "not set", as in docker
	// compose: a script that exports the variable empty falls through to the
	// `name:` of the file and then the directory, instead of failing on a name
	// that cannot be valid.
	if let Some(name) = explicit.filter(|n| !n.is_empty()) {
		return name;
	}
	if let Some(name) = compose_name {
		return name.to_string();
	}
	// An empty base dir means a bare compose filename in the current directory;
	// canonicalize `.` so the basename comes from the working directory.
	let probe = if base_dir.as_os_str().is_empty() {
		Path::new(".")
	} else {
		base_dir
	};
	let basename = probe
		.canonicalize()
		.unwrap_or_else(|_| probe.to_path_buf())
		.file_name()
		.map(|n| n.to_string_lossy().into_owned())
		.unwrap_or_default();
	sanitize_project_name(&basename)
}

/// Normalize a raw directory name into a valid compose project name: lowercase,
/// keep only `[a-z0-9_-]`, then strip any leading `_`/`-`. Falls back to the
/// `podup` literal when nothing valid remains, so the project name is never
/// empty.
pub fn sanitize_project_name(raw: &str) -> String {
	let kept: String = raw
		.to_lowercase()
		.chars()
		.filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_' || *c == '-')
		.collect();
	let trimmed = kept.trim_start_matches(['_', '-']);
	if trimmed.is_empty() {
		"podup".to_string()
	} else {
		trimmed.to_string()
	}
}

/// Expose the project name to the interpolation of the files in `paths` until
/// the returned scope is dropped. A load that does not interpolate exposes none.
pub(super) fn expose(
	interpolate: bool,
	paths: &[PathBuf],
	env_files: &[String],
	project_dir: Option<&Path>,
	explicit: Option<&str>,
	stdin: Option<&str>,
) -> substitute::project_name::Scope {
	let name = interpolate
		.then(|| exposed_name(paths, env_files, project_dir, explicit, stdin))
		.flatten();
	substitute::project_name::Scope::set(name)
}

/// The name to expose as `${COMPOSE_PROJECT_NAME}` while the files in `paths`
/// are interpolated, or `None` when the variables already carry the right one.
///
/// `explicit` is the `-p` flag or the process variable, which win over
/// everything. Otherwise a value in the project `.env` or in an `--env-file`
/// is left alone, since interpolation already sees it. Otherwise the top-level
/// `name:` does, and the last file that sets one wins, as it does in a merge.
/// That `name:` is itself interpolated, with the variables as they stand
/// before the project name exists: a `${COMPOSE_PROJECT_NAME}` inside it
/// reads as unset, so it needs a default or one of the sources above. Failing
/// that, the directory supplies the name.
pub(super) fn exposed_name(
	paths: &[PathBuf],
	env_files: &[String],
	project_dir: Option<&Path>,
	explicit: Option<&str>,
	stdin: Option<&str>,
) -> Option<String> {
	if let Some(name) = explicit.filter(|n| !n.is_empty()) {
		return Some(name.to_string());
	}
	let first = paths.first()?;
	let vars_dir = project_dir.map_or_else(|| dir_of(first), Path::to_path_buf);
	let vars = if env_files.is_empty() {
		substitute::build_vars(&vars_dir)
	} else {
		substitute::build_vars_with_env_files_strict(&vars_dir, env_files).ok()?
	};
	if vars.get(PROJECT_NAME_VAR).is_some_and(|v| !v.is_empty()) {
		return None;
	}
	// The load that follows reports the unset variables of `name:`; this read
	// only wants the value. It goes through the same guards as that load, so a
	// hostile document is refused there and read as nameless here.
	let _quiet = substitute::warnings::Guard::new(false);
	let mut named = None;
	for path in paths {
		let content = if is_stdin(path) {
			stdin.map(str::to_string)
		} else {
			crate::filesystem::read_to_string_capped(path).ok()
		};
		let raw = content
			.and_then(|c| merge::interpolated_value(&c, None).ok())
			.and_then(|doc| doc.get("name")?.as_str().map(str::to_string));
		if let Some(raw) = raw {
			named = substitute::substitute(&raw, &vars).ok();
		}
	}
	match named {
		Some(name) if name.is_empty() => None,
		Some(name) => Some(name),
		None => Some(resolve_project_name(
			None,
			None,
			&resolve_base_dir(project_dir, first),
		)),
	}
}
