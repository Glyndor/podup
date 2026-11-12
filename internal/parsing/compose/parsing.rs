use std::path::{Path, PathBuf};

use super::{
	diagnostics, extends, include, jobs, merge, nested_resets, project_name, tags, types, validate,
};
use crate::error::{ComposeError, Result};
use crate::substitute;
use types::{ComposeFile, ServiceNetworks};

/// Whether a compose-file path is the stdin sentinel `-` (`docker compose -f -`).
pub(super) fn is_stdin(path: &Path) -> bool {
	path == Path::new("-")
}

/// The directory a compose file's own paths resolve against.
///
/// `-f -` reads the document from stdin, which has no directory of its own,
/// so the working directory stands in for it.
pub(super) fn dir_of(path: &Path) -> PathBuf {
	if is_stdin(path) {
		return std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
	}
	let abs = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
	abs.parent().unwrap_or(Path::new(".")).to_path_buf()
}

/// Parse one compose file from disk, applying variable substitution and
/// resolving `extends:` / `include:` directives, with `env_files` (the global
/// `--env-file` flag) loaded into the variable map used for interpolation. These take
/// effect for the top-level file, included files and external extends.
///
/// They **replace** a project `.env` rather than adding to it: when `env_files`
/// is non-empty, `.env` is not read. That is docker-correct, and the opposite
/// of what this comment used to claim, which also reached docs.rs readers. The
/// process environment still takes precedence over both.
pub fn parse_file_with_env_files(path: &Path, env_files: &[String]) -> Result<ComposeFile> {
	parse_file_with_env_files_interp(path, env_files, true)
}

/// Like [`parse_file_with_env_files`] but with explicit control over variable
/// interpolation. `interpolate = false` (the `config --no-interpolate` path)
/// leaves `${VAR}` placeholders literal while still resolving
/// `extends:`/`include:`/merge.
pub fn parse_file_with_env_files_interp(
	path: &Path,
	env_files: &[String],
	interpolate: bool,
) -> Result<ComposeFile> {
	// `-f -` reads the compose document from stdin (like `docker compose`);
	// there is no file to canonicalize, so relative paths and `.env` resolve
	// against the working directory. The stdin bytes are read once here so
	// the parse and the raw nested-key diagnostic can both see them; before
	// this was done in two places, the second of which silently skipped
	// stdin and let a `cat typo.yaml | podup config -f -` lose its warning.
	let stdin = if is_stdin(path) {
		Some(crate::filesystem::read_stdin_to_string_capped().map_err(ComposeError::Io)?)
	} else {
		None
	};
	// This entry point starts a project: the file and everything it pulls in
	// share one interpolation budget, and a load running inside a callback of
	// another one does not borrow that one's count.
	let budget = substitute::project_budget::Scope::open();
	let file = parse_file_with_env_files_interp_with_stdin(
		path,
		env_files,
		interpolate,
		stdin.as_deref(),
		None,
	);
	drop(budget);
	let file = file?;
	// A single file stands alone, so its jobs have to declare their triggers.
	// With several `-f` files the check waits for the merge.
	jobs::check_triggers_declared(&file)?;
	Ok(file)
}

/// [`parse_file_with_env_files_interp`] with the stdin content pre-read.
/// `stdin_content` is consumed only when `path` is the stdin sentinel `-`;
/// for a real path the argument is ignored. The split exists so the
/// integration test can drive the stdin path without redirecting
/// process stdin.
pub(crate) fn parse_file_with_env_files_interp_with_stdin(
	path: &Path,
	env_files: &[String],
	interpolate: bool,
	stdin_content: Option<&str>,
	project_dir: Option<&Path>,
) -> Result<ComposeFile> {
	// `-f -` reads the compose document from stdin (like `docker compose`); there
	// is no file to canonicalize, so relative paths and `.env` resolve against the
	// working directory.
	let (abs, dir) = if is_stdin(path) {
		(PathBuf::from("-"), dir_of(path))
	} else {
		let abs = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
		let dir = abs.parent().unwrap_or(Path::new(".")).to_path_buf();
		(abs, dir)
	};
	// The variables of a project come from the project directory, not from the
	// directory of each file: with several `-f` files in different directories,
	// the `.env` of the project is the one of the first file (or the one
	// `--project-directory` names), and every file is interpolated with it.
	let vars_dir = project_dir.unwrap_or(&dir);
	let mut file =
		parse_file_inner_with_stdin(&abs, vars_dir, env_files, interpolate, stdin_content)
			.map_err(|e| name_file_in_parse_error(e, path))?;

	// The `include:` list of the file, and the list of every file it names in
	// turn: the compose specification applies `include` recursively.
	let included_env_files = include::merge_includes(&mut file, &dir, env_files, interpolate)?;

	let env_files = include::absolute_env_files(&dir, env_files);
	// Jobs and services share a namespace and keep their `extends` to their
	// own kind. Both checks wait until the includes are merged, because a job
	// can arrive from an included file.
	jobs::check_structure(&file)?;
	extends::resolve_all_extends(&mut file, &dir, &env_files, included_env_files, interpolate)?;
	jobs::check_relations(&file)?;
	Ok(file)
}

/// Put the file name on a YAML syntax error. The parser reports only a line and
/// column, so with several `-f` files there was no telling which one held the
/// mistake. The parser's own message stays out, as it echoes the file content.
fn name_file_in_parse_error(error: ComposeError, path: &Path) -> ComposeError {
	let ComposeError::Parse(yaml) = error else {
		return error;
	};
	let shown = if is_stdin(path) {
		"(stdin)".to_string()
	} else {
		crate::error::sanitize_name(&path.display().to_string()).into_owned()
	};
	ComposeError::Invalid(match yaml.location() {
		Some(loc) => format!(
			"failed to parse compose file '{shown}' at line {}, column {}",
			loc.line(),
			loc.column()
		),
		None => format!("failed to parse compose file '{shown}'"),
	})
}

/// Parse and merge multiple compose files (the `-f`/`COMPOSE_FILE` list).
///
/// Files are merged left to right: a later file overrides an earlier one,
/// service by service (per-field, like `extends`), with top-level
/// volumes/networks/secrets/configs replaced on key conflicts. Relative paths
/// resolve against the first file's directory, matching the compose project
/// directory. `env_files` feed interpolation for every file.
pub fn parse_files_with_env_files(paths: &[PathBuf], env_files: &[String]) -> Result<ComposeFile> {
	parse_files_with_env_files_interp(paths, env_files, true)
}

/// Like [`parse_files_with_env_files`] but with explicit interpolation control.
/// `interpolate = false` backs `config --no-interpolate`: `${VAR}` placeholders
/// are left literal across all merged files.
pub fn parse_files_with_env_files_interp(
	paths: &[PathBuf],
	env_files: &[String],
	interpolate: bool,
) -> Result<ComposeFile> {
	parse_files_with_env_files_interp_in(paths, env_files, interpolate, None)
}

/// Like [`parse_files_with_env_files_interp`] but with the project directory
/// the variables are read from.
///
/// The compose specification reads the `.env` of the project, not one per
/// file: with several `-f` files in different directories the variables come
/// from the directory of the first file, or from the directory
/// `--project-directory` names, and every file is interpolated with them.
/// `None` means the directory of the first file.
pub fn parse_files_with_env_files_interp_in(
	paths: &[PathBuf],
	env_files: &[String],
	interpolate: bool,
	project_dir: Option<&Path>,
) -> Result<ComposeFile> {
	parse_files_with_project_in(paths, env_files, interpolate, project_dir, None)
}

/// Like [`parse_files_with_env_files_interp_in`] but with the project name the
/// operator gave (`-p`, or the `COMPOSE_PROJECT_NAME` variable), if any.
///
/// The files may refer to the name of the project as `${COMPOSE_PROJECT_NAME}`.
/// It is the explicit one when there is one, else the top-level `name:`, else
/// the name of the project directory, and it reaches every file the load reads.
pub fn parse_files_with_project_in(
	paths: &[PathBuf],
	env_files: &[String],
	interpolate: bool,
	project_dir: Option<&Path>,
	explicit_project: Option<&str>,
) -> Result<ComposeFile> {
	let mut iter = paths.iter();
	let first = iter
		.next()
		.ok_or_else(|| ComposeError::FileNotFound("no compose file given".to_string()))?;
	// `-f -` reads process stdin; cache the bytes so the same content is
	// visible to the parse and to the raw nested-key diagnostic, otherwise
	// a piped file with a typo would lose its warning because the
	// diagnostic path re-reads from disk and stdin cannot be re-read.
	let stdin = if paths.iter().any(|p| is_stdin(p)) {
		Some(crate::filesystem::read_stdin_to_string_capped().map_err(ComposeError::Io)?)
	} else {
		None
	};
	let _name = project_name::expose(
		interpolate,
		paths,
		env_files,
		project_dir,
		explicit_project,
		stdin.as_deref(),
	);
	let project_dir = project_dir
		.map(Path::to_path_buf)
		.or_else(|| Some(dir_of(first)));
	// One interpolation budget for every `-f` file and what they pull in. It
	// closes before the diagnostics below, which re-read each file on their own.
	let budget = substitute::project_budget::Scope::open();
	let mut merged = parse_file_with_env_files_interp_with_stdin(
		first,
		env_files,
		interpolate,
		stdin.as_deref(),
		project_dir.as_deref(),
	)?;
	for path in iter {
		let other = parse_file_with_env_files_interp_with_stdin(
			path,
			env_files,
			interpolate,
			stdin.as_deref(),
			project_dir.as_deref(),
		)?;
		// `!override`/`!reset` are attached to keys in the raw document and are
		// gone by the time it is a typed `ComposeFile`, so they are collected
		// from the file itself and passed alongside.
		let directives = tags::collect_from_file(path);
		let resets = tags::entry_resets_from_file(path);
		let nested = nested_resets::nested_resets_from_file(path);
		merge_override(&mut merged, other, &directives, &resets, &nested);
	}
	drop(budget);
	// Each file passed on its own; a service in one `-f` file and a job of the
	// same name in another only meet here.
	jobs::check_structure(&merged)?;
	jobs::check_relations(&merged)?;
	jobs::check_triggers_declared(&merged)?;
	normalize_default_network(&mut merged);
	// Semantic validation runs only on the interpolated file: `--no-interpolate`
	// leaves literal `${VAR}` placeholders that cannot be reference- or
	// range-checked. This makes `config`, `up`, and `generate` reject the same
	// contradictory files docker-compose does, at config time.
	if interpolate {
		validate::validate(&merged)?;
	}
	for warning in diagnostics::collect(&merged) {
		diagnostics::emit_diagnostic(&warning);
	}
	// Unknown keys nested inside option blocks (bind/volume/tmpfs mounts, long-form
	// service networks, deploy.resources specs) are dropped by the typed model and
	// so are invisible to `diagnostics::collect`. Re-read each input file's raw,
	// interpolated YAML and diff those blocks directly. This runs per input file
	// (pre-merge): an unknown key in ANY `-f` file should warn, including the
	// stdin compose, whose bytes the parse above already consumed and are
	// re-fed through the same diagnostic path here (#1746 entry 7).
	for warning in diagnostics::collect_raw_nested_warnings(
		paths,
		env_files,
		interpolate,
		stdin.as_deref(),
		project_dir.as_deref(),
	) {
		diagnostics::emit_diagnostic(&warning);
	}
	Ok(merged)
}

/// Re-read `path` and return the interpolated, merge-resolved YAML as text, the
/// same document shape the parser builds before deserializing into a
/// `ComposeFile`. Used only by the raw nested-key diagnostic, which needs the
/// pre-typed document to spot keys the model drops. `interpolate = false` (the
/// `config --no-interpolate` path) leaves `${VAR}` placeholders literal.
pub(super) fn interpolated_yaml_text(
	path: &Path,
	env_files: &[String],
	interpolate: bool,
	project_dir: Option<&Path>,
) -> Result<String> {
	let abs = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
	let dir = abs.parent().unwrap_or(Path::new(".")).to_path_buf();
	let content = crate::filesystem::read_to_string_capped(&abs).map_err(|e| {
		if e.kind() == std::io::ErrorKind::NotFound {
			ComposeError::FileNotFound(abs.display().to_string())
		} else {
			ComposeError::Io(e)
		}
	})?;
	interpolated_yaml_text_from_content(
		&content,
		project_dir.unwrap_or(&dir),
		env_files,
		interpolate,
	)
}

/// [`interpolated_yaml_text`] with the content already in memory, which the
/// stdin path needs because the diagnostic cannot re-read stdin
/// (`stdin()` is one-shot). Splits out so the path and the stdin
/// branches share the same interpolation step, with no risk of
/// drifting (#1746 entry 7).
fn interpolated_yaml_text_from_content(
	content: &str,
	dir: &Path,
	env_files: &[String],
	interpolate: bool,
) -> Result<String> {
	// The parse pass already emitted unset-variable warnings for this
	// document; the diagnostic pass only re-interpolates so the typed model
	// can be diffed against the raw shape. Silence substitute warnings here
	// so every reference to the same missing variable in the file emits one
	// line, not two (#1838), and the audit path that reuses this same code
	// converges to the same count as config.
	let _silence = substitute::warnings::Guard::new(false);
	let value = if interpolate {
		let vars = substitute::build_vars_with_env_files_strict(dir, env_files)?;
		merge::interpolated_value(content, Some(&vars))?
	} else {
		merge::interpolated_value(content, None)?
	};
	Ok(serde_yaml::to_string(&value)?)
}

/// Synthesize the implicit `default` network, matching docker-compose: any
/// service that declares neither `networks:` (or declares it empty) nor `network_mode` is attached to
/// a project `default` network. Without this, such services are created with no
/// network namespace at all: they get no IP and cannot resolve each other by
/// name, silently breaking the common no-`networks:`-block compose file.
///
/// The `default` network is created as `{project}_default` (see
/// `resolve_network_name`) unless the file already defines a top-level
/// `networks.default`, whose configuration is then respected. Idempotent.
pub(crate) fn normalize_default_network(file: &mut ComposeFile) {
	let needs_default = file
		.services
		.values()
		.chain(file.jobs.values().map(|job| &job.service))
		.any(|svc| {
			svc.network_mode.is_none()
				&& (svc.networks.is_unset()
					|| svc.networks.names().iter().any(|net| net == "default"))
		});
	if !needs_default {
		return;
	}
	file.networks.entry("default".to_string()).or_insert(None);
	for svc in file.services_and_jobs_mut() {
		if svc.network_mode.is_none() && svc.networks.is_unset() {
			svc.networks = ServiceNetworks::List(vec!["default".to_string()]);
		}
	}
}

/// Remove the top-level entry a later file reset with `!reset`.
fn drop_entry(target: &mut ComposeFile, section: &str, name: &str) {
	match section {
		"services" => drop(target.services.shift_remove(name)),
		"networks" => drop(target.networks.shift_remove(name)),
		"volumes" => drop(target.volumes.shift_remove(name)),
		"secrets" => drop(target.secrets.shift_remove(name)),
		"configs" => drop(target.configs.shift_remove(name)),
		"models" => drop(target.models.shift_remove(name)),
		_ => {}
	}
}

/// Merge `other` into `target` with `other` winning (compose `-f` override
/// semantics): services and the volumes, networks, secrets and configs they
/// share are merged field by field.
pub(super) fn merge_override(
	target: &mut ComposeFile,
	other: ComposeFile,
	directives: &tags::Directives,
	resets: &tags::EntryResets,
	nested: &nested_resets::NestedResets,
) {
	for (section, name) in resets {
		drop_entry(target, section, name);
	}
	// The project name is a single value, and the last file that sets one wins.
	if other.name.is_some() {
		target.name = other.name.clone();
	}
	for (name, svc) in other.services {
		if let Some(base) = target.services.get_mut(&name) {
			let tagged = directives.get(&name);
			*base = extends::merge_service_tagged(std::mem::take(base), svc, tagged);
			if let Some(paths) = nested.get(&name) {
				nested_resets::apply_nested_resets(base, paths);
			}
		} else {
			target.services.insert(name, svc);
		}
	}
	// A resource both files declare merges key by key, as docker does: the second
	// file's `labels` add to the first file's, and a `driver` it leaves out stays.
	for (k, v) in other.volumes {
		match target.volumes.get_mut(&k) {
			Some(slot) => *slot = extends::merge_block(slot.take(), v, None),
			None => {
				target.volumes.insert(k, v);
			}
		}
	}
	for (k, v) in other.networks {
		match target.networks.get_mut(&k) {
			Some(slot) => *slot = extends::merge_block(slot.take(), v, None),
			None => {
				target.networks.insert(k, v);
			}
		}
	}
	for (k, v) in other.secrets {
		match target.secrets.get_mut(&k) {
			Some(slot) => {
				let merged = extends::merge_block(Some(slot.clone()), Some(v.clone()), None);
				*slot = merged.unwrap_or(v);
			}
			None => {
				target.secrets.insert(k, v);
			}
		}
	}
	for (k, v) in other.configs {
		match target.configs.get_mut(&k) {
			Some(slot) => {
				let merged = extends::merge_block(Some(slot.clone()), Some(v.clone()), None);
				*slot = merged.unwrap_or(v);
			}
			None => {
				target.configs.insert(k, v);
			}
		}
	}
	for (k, v) in other.models {
		target.models.insert(k, v);
	}
	// Jobs merge field by field like services, so an override file can change
	// one key of a job without restating the rest.
	for (name, job) in other.jobs {
		if let Some(base) = target.jobs.get_mut(&name) {
			*base = jobs::merge_job(std::mem::take(base), job, directives.get(&name));
		} else {
			target.jobs.insert(name, job);
		}
	}
}

/// Parse a compose YAML string (no file I/O).
///
/// Variable substitution is applied using only the process environment.
/// `extends: { file: ... }` and `include:` directives are not resolved;
/// use [`parse_file_with_env_files`] for that.
pub fn parse_str(content: &str) -> Result<ComposeFile> {
	// A document of its own: it counts against a budget of its own, not against
	// a project that happens to be loading on this thread when a callback calls
	// this.
	let _budget = substitute::project_budget::Scope::open();
	let vars = substitute::build_vars(Path::new("."));
	let mut file = merge::deserialize_with_merge_interp(content, Some(&vars))?;
	jobs::check_structure(&file)?;
	extends::resolve_extends_same_file(&mut file)?;
	jobs::check_relations(&file)?;
	jobs::check_triggers_declared(&file)?;
	Ok(file)
}

/// Parse raw (already-substituted) YAML into a `ComposeFile` without any
/// post-processing.
pub fn parse_str_raw(content: &str) -> Result<ComposeFile> {
	merge::deserialize_with_merge(content)
}

pub(crate) fn parse_file_inner_with_env(
	path: &Path,
	vars_dir: &Path,
	extra_env_files: &[String],
	interpolate: bool,
) -> Result<ComposeFile> {
	parse_file_inner_with_stdin(path, vars_dir, extra_env_files, interpolate, None)
}

/// [`parse_file_inner_with_env`] with the stdin content pre-read. When
/// `path` is the stdin sentinel `-` and `stdin_content` is `None`, the
/// function reads process stdin; when both are present, the supplied
/// content is used and stdin is left alone. Used by the integration
/// to keep the same byte sequence visible to the parse and to the raw
/// nested-key diagnostic, so a `cat typo.yaml | podup config -f -`
/// surfaces its typo instead of silently dropping the warning
/// (#1746 entry 7).
pub(crate) fn parse_file_inner_with_stdin(
	path: &Path,
	vars_dir: &Path,
	extra_env_files: &[String],
	interpolate: bool,
	stdin_content: Option<&str>,
) -> Result<ComposeFile> {
	let content = if is_stdin(path) {
		match stdin_content {
			Some(c) => c.to_string(),
			None => crate::filesystem::read_stdin_to_string_capped().map_err(ComposeError::Io)?,
		}
	} else {
		crate::filesystem::read_to_string_capped(path).map_err(|e| {
			if e.kind() == std::io::ErrorKind::NotFound {
				ComposeError::FileNotFound(path.display().to_string())
			} else {
				ComposeError::Io(e)
			}
		})?
	};
	// `config --no-interpolate` leaves `${VAR}` placeholders literal; otherwise
	// interpolate against the env/.env/env-file variable map. Interpolation runs
	// on the parsed YAML scalars (see `deserialize_with_merge_interp`), not the
	// raw text, so resolved values cannot alter the document structure.
	if interpolate {
		let vars = substitute::build_vars_with_env_files_strict(vars_dir, extra_env_files)?;
		merge::deserialize_with_merge_interp(&content, Some(&vars))
	} else {
		merge::deserialize_with_merge(&content)
	}
}
