//! Resolution of compose files, base directory, and project name.

use std::path::{Path, PathBuf};

use podup::ComposeError;

/// Validate an explicit `--project-directory`: it must exist and be a directory.
/// A `None` (unset) directory is always fine — it is derived from the compose
/// file's location. Matches `docker compose`, which errors on a missing working
/// directory instead of silently accepting it.
pub(crate) fn validate_project_directory(dir: Option<&Path>) -> podup::Result<()> {
	if let Some(dir) = dir {
		if !dir.is_dir() {
			return Err(ComposeError::Unsupported(format!(
				"--project-directory {} does not exist or is not a directory",
				dir.display()
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
pub(crate) fn resolve_compose_files(explicit: &[PathBuf]) -> Vec<PathBuf> {
	if !explicit.is_empty() {
		return explicit.to_vec();
	}
	if let Ok(env) = std::env::var("COMPOSE_FILE") {
		if !env.is_empty() {
			let sep = if cfg!(windows) { ';' } else { ':' };
			return env.split(sep).map(PathBuf::from).collect();
		}
	}
	for candidate in COMPOSE_FILE_CANDIDATES {
		if Path::new(candidate).is_file() {
			return vec![PathBuf::from(candidate)];
		}
	}
	vec![PathBuf::from("docker-compose.yml")]
}

/// Resolve the base directory for relative-path resolution. An explicit
/// `--project-directory` wins; otherwise it is the directory containing the
/// compose file (compose-spec default), or the current directory when the
/// compose file has no parent component.
pub(crate) fn resolve_base_dir(project_directory: Option<&Path>, file: &Path) -> PathBuf {
	if let Some(dir) = project_directory {
		return dir.to_path_buf();
	}
	match file.parent() {
		Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
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
pub(crate) fn resolve_project_name(
	explicit: Option<String>,
	compose_name: Option<&str>,
	base_dir: &Path,
) -> String {
	if let Some(name) = explicit {
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
pub(crate) fn sanitize_project_name(raw: &str) -> String {
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

#[cfg(test)]
mod tests {
	use super::{
		resolve_base_dir, resolve_compose_files, resolve_project_name, sanitize_project_name,
		validate_project_directory,
	};
	use std::path::{Path, PathBuf};

	#[test]
	fn validate_project_directory_accepts_none_and_existing_dir() {
		validate_project_directory(None).unwrap();
		let dir = std::env::temp_dir();
		validate_project_directory(Some(&dir)).unwrap();
	}

	#[test]
	fn validate_project_directory_rejects_missing_and_file() {
		let missing = std::env::temp_dir().join(format!("podup-pd-{}-nope", std::process::id()));
		assert!(validate_project_directory(Some(&missing)).is_err());

		let file = std::env::temp_dir().join(format!("podup-pd-{}.tmp", std::process::id()));
		std::fs::write(&file, b"x").unwrap();
		assert!(validate_project_directory(Some(&file)).is_err());
		let _ = std::fs::remove_file(&file);
	}

	#[test]
	fn explicit_compose_files_win() {
		let p = resolve_compose_files(&[PathBuf::from("custom.yml")]);
		assert_eq!(p, vec![PathBuf::from("custom.yml")]);
	}

	#[test]
	fn multiple_explicit_compose_files_preserved() {
		let p = resolve_compose_files(&[PathBuf::from("a.yml"), PathBuf::from("b.yml")]);
		assert_eq!(p, vec![PathBuf::from("a.yml"), PathBuf::from("b.yml")]);
	}

	#[test]
	fn missing_compose_file_falls_back_to_default_name() {
		// In a directory with no candidate files, the default name is returned
		// so the resulting error names a sensible path.
		let dir = std::env::temp_dir().join(format!("podup-cf-{}", std::process::id()));
		std::fs::create_dir_all(&dir).unwrap();
		let prev = std::env::current_dir().unwrap();
		std::env::set_current_dir(&dir).unwrap();
		// Scope COMPOSE_FILE to "unset" race-free so a value set in the test
		// environment cannot leak in and `temp-env` restores it afterwards.
		let p = temp_env::with_var_unset("COMPOSE_FILE", || resolve_compose_files(&[]));
		std::env::set_current_dir(prev).unwrap();
		let _ = std::fs::remove_dir_all(&dir);
		assert_eq!(p, vec![PathBuf::from("docker-compose.yml")]);
	}

	#[test]
	fn project_directory_override_wins() {
		let base = resolve_base_dir(
			Some(Path::new("/srv/app")),
			Path::new("/etc/compose/docker-compose.yml"),
		);
		assert_eq!(base, PathBuf::from("/srv/app"));
	}

	#[test]
	fn defaults_to_compose_file_parent() {
		let base = resolve_base_dir(None, Path::new("/etc/compose/docker-compose.yml"));
		assert_eq!(base, PathBuf::from("/etc/compose"));
	}

	#[test]
	fn bare_filename_resolves_to_current_dir() {
		// A bare filename has no directory component, so the base directory must
		// fall back to the working directory — never an empty path, which would
		// leave a relative `file:` source to be resolved against the Podman
		// service's working directory ($HOME) instead of the project directory.
		let base = resolve_base_dir(None, Path::new("docker-compose.yml"));
		assert_eq!(base, std::env::current_dir().unwrap());
		assert!(base.is_absolute());
	}

	#[test]
	fn explicit_project_name_wins() {
		let name = resolve_project_name(
			Some("explicit".to_string()),
			Some("from-compose"),
			Path::new("/srv/myapp"),
		);
		assert_eq!(name, "explicit");
	}

	#[test]
	fn compose_name_used_when_no_explicit() {
		let name = resolve_project_name(None, Some("from-compose"), Path::new("/srv/myapp"));
		assert_eq!(name, "from-compose");
	}

	#[cfg(unix)]
	#[test]
	fn falls_back_to_directory_basename() {
		let dir = std::env::temp_dir().join(format!("podup-pn-{}", std::process::id()));
		std::fs::create_dir_all(&dir).unwrap();
		let name = resolve_project_name(None, None, &dir);
		let _ = std::fs::remove_dir_all(&dir);
		// The basename is sanitized; the temp dir name is already lowercase
		// alphanumeric with hyphens, so it survives unchanged.
		assert_eq!(
			name,
			dir.file_name().unwrap().to_string_lossy().to_lowercase()
		);
	}

	#[test]
	fn sanitize_lowercases_and_drops_invalid_chars() {
		assert_eq!(sanitize_project_name("My App!"), "myapp");
	}

	#[test]
	fn sanitize_keeps_underscore_and_hyphen() {
		assert_eq!(sanitize_project_name("web_service-1"), "web_service-1");
	}

	#[test]
	fn sanitize_strips_leading_separators() {
		assert_eq!(sanitize_project_name("__leading"), "leading");
		assert_eq!(sanitize_project_name("--dash"), "dash");
	}

	#[test]
	fn sanitize_empty_result_falls_back_to_podup() {
		assert_eq!(sanitize_project_name("!!!"), "podup");
		assert_eq!(sanitize_project_name(""), "podup");
	}
}
