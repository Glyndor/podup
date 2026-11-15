//! Interpolation of the values inside dotenv files.
//!
//! A value of a `.env` or an `--env-file` can refer to the process environment
//! and to the keys the file set before it, as in Docker Compose:
//! `B=${A} World`, `C=$HOME/x`, `D=${X:-fallback}`. A single-quoted value stays
//! literal. The process environment is looked up first, then the keys of the
//! files read so far, so a variable exported by the caller is what a value sees.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::{process_vars, substitute_budgeted};
use crate::error::{ComposeError, Result};

/// A `(key, value, literal)` pair as the dotenv parser returns it: `literal`
/// marks a value that must not be interpolated.
type Pair = (String, String, bool);

/// Expand `pairs` in order. `scope` carries what values can refer to: the
/// process environment on entry, and the keys set so far as the pairs go by.
///
/// With `strict` an unusable reference (`${X:?message}`, a malformed template)
/// is an error that names the file the value was read from and the key it was
/// written under; without it the value is kept as written, for the caller that
/// reads a file it has no way to report about.
fn expand(
	pairs: Vec<Pair>,
	process: &HashMap<String, String>,
	scope: &mut HashMap<String, String>,
	strict: bool,
	source: &str,
) -> Result<Vec<(String, String)>> {
	// Unset names default to empty here without a warning, as Docker does for
	// a dotenv file; the warning is for the compose document.
	let _quiet = super::warnings::Guard::new(false);
	let mut spent = 0usize;
	let mut warned = HashSet::new();
	let mut out = Vec::with_capacity(pairs.len());
	for (key, value, literal) in pairs {
		let value = if literal {
			value
		} else {
			match substitute_budgeted(&value, scope, &mut spent, &mut warned) {
				Ok(expanded) => expanded,
				Err(e) if strict => return Err(env_error(source, &key, e)),
				Err(_) => value,
			}
		};
		if !process.contains_key(&key) {
			scope.insert(key.clone(), value.clone());
		}
		out.push((key, value));
	}
	Ok(out)
}

/// The error of a value that could not be expanded, naming the file it was
/// read from and the key it was written under.
///
/// Docker Compose refuses the same values when it reads the file, and it names
/// the file and the variable in its own message.
fn env_error(source: &str, key: &str, error: ComposeError) -> ComposeError {
	let detail = match error {
		// The message and the template come from the env file: escaped and
		// bounded like any other echoed value.
		ComposeError::RequiredVarNotSet { var, msg } => format!(
			"required variable '{}' is missing a value: {}",
			crate::error::sanitize_name(&var),
			crate::errors::clip::clip_for_message(&msg)
		),
		ComposeError::InvalidInterpolationFormat { template, .. } => format!(
			"invalid template: \"{}\"",
			crate::errors::clip::clip_for_message(&template)
		),
		other => other.to_string(),
	};
	ComposeError::EnvFile(format!(
		"env file {}: value of '{}': {detail}",
		crate::error::sanitize_name(source),
		crate::error::sanitize_name(key)
	))
}

/// The values of the optional `.env` of `dir`. A missing or unreadable file
/// gives an empty layer. With `strict` a malformed value is an error naming
/// the file and the key, as in Docker Compose: an unterminated quoted value,
/// a required variable without a value, or a template that is not a valid
/// interpolation. Without it that value is kept as written, for callers that
/// have no project to report to.
pub(super) fn default_layer(dir: &Path, strict: bool) -> Result<HashMap<String, String>> {
	let process = process_vars();
	let mut scope = process.clone();
	let mut layer = HashMap::new();
	// A `.env` that is a link is read through it, like docker compose does; the
	// file it points to is still checked to be a regular file when it is read.
	// A link that cannot be resolved is read as given and fails there.
	let path = dir.join(".env");
	let path = std::fs::canonicalize(&path).unwrap_or(path);
	if let Ok(content) = crate::filesystem::read_to_string_capped(&path) {
		let name = dir.join(".env").display().to_string();
		let source = crate::error::sanitize_name(&name);
		let pairs = if strict {
			crate::dotenv::parse_strict_marked(&content)
				.map_err(|e| ComposeError::EnvFile(format!("env file {source}: {e}")))?
		} else {
			crate::dotenv::parse_marked(&content)
		};
		layer.extend(expand(pairs, &process, &mut scope, strict, &source)?);
	}
	Ok(layer)
}

/// The values the project's env files set, without the process environment.
///
/// With no `extra` files this is the optional `.env` of `dir`; otherwise the
/// listed files replace it, a later file overriding an earlier one and seeing
/// its keys. A missing or malformed explicit file is an error.
pub fn dotenv_layer(dir: &Path, extra: &[String]) -> Result<HashMap<String, String>> {
	if extra.is_empty() {
		return default_layer(dir, true);
	}
	let process = process_vars();
	let mut scope = process.clone();
	let mut layer = HashMap::new();
	for path in extra {
		let abs = if Path::new(path).is_absolute() {
			std::path::PathBuf::from(path)
		} else {
			dir.join(path)
		};
		let content = crate::filesystem::read_to_string_capped(&abs).map_err(|e| {
			ComposeError::EnvFile(format!(
				"env file not found: {} ({e})",
				crate::error::sanitize_name(&abs.display().to_string())
			))
		})?;
		let pairs = crate::dotenv::parse_strict_marked(&content)?;
		let name = abs.display().to_string();
		let source = crate::error::sanitize_name(&name);
		for (key, value, _) in &pairs {
			// A disallowed control character (e.g. NUL) in a value would be
			// interpolated verbatim into a compose scalar, where it is meaningless
			// at best and corrupts the container's config at worst. Reject it here,
			// at load time, with an error that names the originating env file and
			// key, instead of letting it surface later as a compose-file parse
			// error at a meaningless post-substitution offset.
			if let Some(bad) = super::first_disallowed_control_char(value) {
				return Err(ComposeError::EnvFile(format!(
					"env file {}: value of '{key}' contains a disallowed control \
					 character ({}); remove it before use",
					abs.display(),
					bad.escape_default(),
				)));
			}
		}
		layer.extend(expand(pairs, &process, &mut scope, true, &source)?);
	}
	Ok(layer)
}
