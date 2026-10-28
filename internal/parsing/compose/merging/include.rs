//! `include:` directive: merging externally included compose files.
//!
//! Included files are merged into the parent: services, volumes, networks,
//! secrets, configs, and models from the included file are added only if the
//! key does not already exist in the parent (parent wins on conflict).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::super::types::{ComposeFile, IncludeConfig};
use super::anchor;
use crate::error::{ComposeError, Result};

/// How many `include:` levels deep the parser goes. A project that nests
/// includes deeper than this is a mistake, and the cap keeps a file that
/// includes itself from being read forever.
const MAX_INCLUDE_DEPTH: usize = 16;

/// Read and parse a file referenced by `include:`, wrapping every failure in
/// [`ComposeError::Include`] so a consumer matching on the variant can tell the
/// failure originated from an included file rather than the top-level compose
/// file. A missing included file is not the same as a missing main file, and
/// an invalid-YAML included file is not the same as a malformed main file:
/// the variant lets a handler branch on the difference, and the message names
/// the included path so the operator can find it.
pub(in super::super) fn parse_included_file(
	path: &Path,
	dir: &Path,
	env_files: &[String],
	interpolate: bool,
) -> Result<ComposeFile> {
	let display = path.display().to_string();
	super::super::parse_file_inner_with_env(path, dir, env_files, interpolate).map_err(
		|e| match e {
			ComposeError::FileNotFound(p) => {
				ComposeError::Include(format!("included compose file not found: {p}"))
			}
			ComposeError::Parse(yaml_err) => {
				let msg = match yaml_err.location() {
					Some(loc) => format!(
						"failed to parse included compose file '{display}' at line {}, column {}",
						loc.line(),
						loc.column()
					),
					None => format!("failed to parse included compose file '{display}'"),
				};
				ComposeError::Include(msg)
			}
			ComposeError::Io(io_err) => ComposeError::Include(format!(
				"io error reading included compose file '{display}': {io_err}"
			)),
			other => ComposeError::Include(format!("included compose file '{display}': {other}")),
		},
	)
}

/// Anchor interpolation inputs before a different compose directory can reinterpret them.
pub(in super::super) fn absolute_env_files(dir: &Path, env_files: &[String]) -> Vec<String> {
	env_files
		.iter()
		.map(|path| dir.join(path).to_string_lossy().into_owned())
		.collect()
}

/// Merge `other` into `target`.
///
/// Services / volumes / networks / secrets / configs / models / jobs from
/// `other` are added; existing entries in `target` win on conflict (parent
/// file overrides included content).
pub(in super::super) fn merge_compose_file(target: &mut ComposeFile, other: ComposeFile) {
	for (k, v) in other.services {
		target.services.entry(k).or_insert(v);
	}
	for (k, v) in other.volumes {
		target.volumes.entry(k).or_insert(v);
	}
	for (k, v) in other.networks {
		target.networks.entry(k).or_insert(v);
	}
	for (k, v) in other.secrets {
		target.secrets.entry(k).or_insert(v);
	}
	for (k, v) in other.configs {
		target.configs.entry(k).or_insert(v);
	}
	for (k, v) in other.models {
		target.models.entry(k).or_insert(v);
	}
	for (k, v) in other.jobs {
		target.jobs.entry(k).or_insert(v);
	}
}

/// Merge the files an `include:` names into `file`.
///
/// The include list of an included file is honoured too: the compose
/// specification applies `include` recursively, so a file that is included
/// brings its own includes with it. Every file is resolved against its own
/// directory, and the relative paths of what it brings are anchored to that
/// directory before it is merged.
///
/// A file that includes itself, directly or through other files, is refused
/// instead of being read forever.
///
/// Returns the services the includes contributed, with the env files of the
/// file each one came from: a service that extends a file of its own reads
/// that file with those env files.
pub(in super::super) fn merge_includes(
	file: &mut ComposeFile,
	dir: &Path,
	env_files: &[String],
	interpolate: bool,
) -> Result<HashMap<String, Vec<String>>> {
	let mut added: HashMap<String, Vec<String>> = HashMap::new();
	let mut chain: Vec<PathBuf> = Vec::new();
	merge_includes_in(file, dir, env_files, interpolate, &mut added, 0, &mut chain)?;
	Ok(added)
}

/// [`merge_includes`] over one file: its `include:` list, and the includes of
/// every file it names, in turn.
fn merge_includes_in(
	file: &mut ComposeFile,
	dir: &Path,
	env_files: &[String],
	interpolate: bool,
	added: &mut HashMap<String, Vec<String>>,
	depth: usize,
	chain: &mut Vec<PathBuf>,
) -> Result<()> {
	if depth > MAX_INCLUDE_DEPTH {
		return Err(ComposeError::Include(format!(
			"include: more than {MAX_INCLUDE_DEPTH} levels deep; the files include each other"
		)));
	}
	let includes = std::mem::take(&mut file.include);
	for inc in includes {
		let (extra_env_files, project_dir_override) = match &inc {
			IncludeConfig::Long {
				env_file,
				project_directory,
				..
			} => (
				env_file.as_ref().map(|ef| ef.to_list()).unwrap_or_default(),
				project_directory.as_ref().map(|pd| dir.join(pd)),
			),
			_ => (vec![], None),
		};
		for rel in inc.paths() {
			let rel_path = Path::new(&rel);
			// The Compose Specification resolves `include` paths relative to the
			// including file and treats `../` as canonical (monorepos routinely use
			// `include: ../shared/compose.yaml`). An absolute path is used as given.
			// This matches docker-compose and the trusted-input policy already
			// applied to `extends.file` and `env_file`: the compose file is
			// trusted input, like a Makefile.
			let inc_path = if rel_path.is_absolute() {
				rel_path.to_path_buf()
			} else {
				dir.join(&rel)
			};
			let inc_dir = project_dir_override.clone().unwrap_or_else(|| {
				inc_path
					.parent()
					.map(|p| p.to_path_buf())
					.unwrap_or_else(|| dir.to_path_buf())
			});
			let mut combined_env_files = env_files.to_vec();
			combined_env_files.extend(extra_env_files.iter().cloned());
			// A file that is already being read is a cycle: `a.yml` includes
			// `b.yml`, which includes `a.yml` again.
			let key = inc_path.canonicalize().unwrap_or_else(|_| inc_path.clone());
			if chain.contains(&key) {
				return Err(ComposeError::Include(format!(
					"circular include: {} is already being included",
					inc_path.display()
				)));
			}
			chain.push(key);
			let mut included =
				parse_included_file(&inc_path, &inc_dir, &combined_env_files, interpolate)?;
			// The names this file declares, before its own includes add more.
			let own_names: Vec<String> = included
				.services
				.keys()
				.chain(included.jobs.keys())
				.cloned()
				.collect();
			// The included file's own includes, merged into it first.
			let mut nested: HashMap<String, Vec<String>> = HashMap::new();
			merge_includes_in(
				&mut included,
				&inc_dir,
				&combined_env_files,
				interpolate,
				&mut nested,
				depth + 1,
				chain,
			)?;
			// The services this file contributes to the parent.
			let mut own: HashMap<String, Vec<String>> = HashMap::new();
			for name in own_names {
				if !file.services.contains_key(&name) && !file.jobs.contains_key(&name) {
					own.insert(name, absolute_env_files(&inc_dir, &combined_env_files));
				}
			}
			// A nested service keeps the env files of the file it came from,
			// unless this file declared the same name: this file's service is
			// the one that survives the merge.
			for (name, files) in nested {
				if !file.services.contains_key(&name) && !file.jobs.contains_key(&name) {
					own.entry(name).or_insert(files);
				}
			}
			anchor::anchor_compose_file(&mut included, &inc_dir);
			merge_compose_file(file, included);
			for (name, files) in own {
				added.entry(name).or_insert(files);
			}
			chain.pop();
		}
	}
	Ok(())
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "include_tests.rs"]
mod tests;
