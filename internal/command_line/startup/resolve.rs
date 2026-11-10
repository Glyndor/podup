//! Resolution of compose files, base directory, and project name.

use std::path::{Path, PathBuf};

use podup::ComposeError;

// The project name is also needed while the compose file is read, because the
// file may interpolate it, so its resolution lives with the parser.
pub(crate) use podup::compose::{resolve_base_dir, resolve_project_name};

/// Validate an explicit `--project-directory`: it must exist and be a directory.
/// A `None` (unset) directory is always fine: it is derived from the compose
/// file's location. Matches `docker compose`, which errors on a missing working
/// directory instead of silently accepting it.
pub(crate) fn validate_project_directory(dir: Option<&Path>) -> podup::Result<()> {
	if let Some(dir) = dir {
		if !dir.is_dir() {
			return Err(ComposeError::Invalid(format!(
				"--project-directory {dir:?} does not exist or is not a directory"
			)));
		}
	}
	Ok(())
}

/// Compose-spec file-name precedence, highest first.
const COMPOSE_FILE_CANDIDATES: [&str; 4] = [
	"compose.yaml",
	"compose.yml",
	"docker-compose.yaml",
	"docker-compose.yml",
];

/// Resolve which compose file(s) to load. Explicit `--file` flags win; then the
/// `COMPOSE_FILE` environment variable (a path-separator-delimited list);
/// otherwise probe the compose-spec precedence list in the current directory,
/// falling back to `docker-compose.yml` so a missing-file error names a
/// sensible path. Multiple files are merged in order, later overriding earlier.
/// Whether the compose files came from the operator (`-f` or `COMPOSE_FILE`)
/// rather than from the directory lookup. A named file that is not there is
/// an error even for the commands that can run without any file (#1687).
pub(crate) fn compose_files_were_named(explicit: &[PathBuf]) -> bool {
	!explicit.is_empty() || std::env::var("COMPOSE_FILE").is_ok_and(|v| !v.is_empty())
}

/// The separator of the `COMPOSE_FILE` list: `COMPOSE_PATH_SEPARATOR` when it is
/// set and not empty, otherwise `:` (`;` on Windows).
pub(crate) fn path_separator(custom: Option<&str>) -> String {
	match custom {
		Some(sep) if !sep.is_empty() => sep.to_string(),
		_ if cfg!(windows) => ";".to_string(),
		_ => ":".to_string(),
	}
}

pub(crate) fn resolve_compose_files(explicit: &[PathBuf]) -> Vec<PathBuf> {
	if !explicit.is_empty() {
		return explicit.to_vec();
	}
	if let Ok(env) = std::env::var("COMPOSE_FILE") {
		if !env.is_empty() {
			let custom = std::env::var("COMPOSE_PATH_SEPARATOR").ok();
			let sep = path_separator(custom.as_deref());
			return env
				.split(sep.as_str())
				.filter(|part| !part.is_empty())
				.map(PathBuf::from)
				.collect();
		}
	}
	let present = candidates_in(Path::new(""));
	if let Some(first) = present.first() {
		static WARNED: std::sync::Once = std::sync::Once::new();
		if present.len() > 1 {
			// Two of the supported names side by side: the one used is the first
			// by precedence, and the person who edited the other one would not
			// otherwise find out. docker compose says so too.
			WARNED.call_once(|| tracing::warn!("{}", multiple_candidates_message(&present)));
		}
		let mut files = vec![first.clone()];
		files.extend(override_for(first));
		return files;
	}
	vec![PathBuf::from("docker-compose.yml")]
}

/// The compose files of the precedence list that exist in `dir`, highest first.
pub(super) fn candidates_in(dir: &Path) -> Vec<PathBuf> {
	COMPOSE_FILE_CANDIDATES
		.iter()
		.map(|name| dir.join(name))
		.filter(|path| path.is_file())
		.collect()
}

/// The warning for more than one candidate present: what was found, and which
/// of them is used.
pub(super) fn multiple_candidates_message(present: &[PathBuf]) -> String {
	let names: Vec<String> = present.iter().map(|p| p.display().to_string()).collect();
	format!(
		"found several compose files with supported names ({}); using {}",
		names.join(", "),
		names[0]
	)
}

/// Override-file names, in the compose-spec precedence order. Only the first one
/// present is used; docker compose does not merge two overrides.
const OVERRIDE_FILE_CANDIDATES: [&str; 4] = [
	"compose.override.yaml",
	"compose.override.yml",
	"docker-compose.override.yaml",
	"docker-compose.override.yml",
];

/// The override file to merge on top of an auto-discovered `base`, if one sits
/// beside it.
///
/// Base file plus `docker-compose.override.yml` is how nearly every repository
/// separates dev from prod, and docker compose merges it automatically whenever
/// no explicit `-f` is given. podup ran the base alone and said nothing: wrong
/// image tags, wrong published ports, missing dev bind mounts, exit 0, about a
/// file the user never named on the command line, so nothing in the invocation
/// hinted at what went wrong.
///
/// Discovery is deliberately limited to the auto-discovery path. An explicit
/// `-f` means the caller is choosing the file set themselves, and `COMPOSE_FILE`
/// is that same choice by another name; docker compose skips the override in
/// both cases too.
pub(super) fn override_for(base: &Path) -> Option<PathBuf> {
	let dir = base.parent().unwrap_or(Path::new(""));
	OVERRIDE_FILE_CANDIDATES
		.iter()
		.map(|name| dir.join(name))
		.find(|path| path.is_file())
}

/// Make a compose-file path absolute for recording on a container label.
///
/// The label is read back by `ls` from a different working directory than the
/// one the project was started in, so a relative path there would be
/// meaningless. Falls back to the path as given when the filesystem cannot
/// resolve it; a best-effort label must never fail the command that sets it.
pub(crate) fn absolute(path: &Path) -> PathBuf {
	std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Anchor relative `--env-file` paths at `cwd`.
///
/// Like every other path given on the command line, an `--env-file` is
/// relative to where the command is run, not to the compose file's directory,
/// so `cd envs && podup --env-file x.env -f ../compose.yaml config` reads
/// `envs/x.env`. Absolute paths are left as they are.
pub(crate) fn anchor_env_files(files: &[String], cwd: &Path) -> Vec<String> {
	files
		.iter()
		.map(|f| {
			let path = Path::new(f);
			if path.is_absolute() {
				f.clone()
			} else {
				cwd.join(path).to_string_lossy().into_owned()
			}
		})
		.collect()
}
