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
/// With `strict` an unusable reference (`${X:?message}`, a runaway expansion) is
/// an error; without it the value is kept as written, for the optional `.env`
/// that has never been able to fail.
fn expand(
	pairs: Vec<Pair>,
	process: &HashMap<String, String>,
	scope: &mut HashMap<String, String>,
	strict: bool,
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
				Err(e) if strict => return Err(e),
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

/// The values the project's env files set, without the process environment.
///
/// With no `extra` files this is the optional `.env` of `dir`; otherwise the
/// listed files replace it, a later file overriding an earlier one and seeing
/// its keys. A missing or malformed explicit file is an error.
pub fn dotenv_layer(dir: &Path, extra: &[String]) -> Result<HashMap<String, String>> {
	let process = process_vars();
	let mut scope = process.clone();
	let mut layer = HashMap::new();
	if extra.is_empty() {
		// A `.env` that is a link is read through it, like docker compose does; the
		// file it points to is still checked to be a regular file when it is read.
		// A link that cannot be resolved is read as given and fails there.
		let path = dir.join(".env");
		let path = std::fs::canonicalize(&path).unwrap_or(path);
		if let Ok(content) = crate::filesystem::read_to_string_capped(&path) {
			let pairs = crate::dotenv::parse_marked(&content);
			layer.extend(expand(pairs, &process, &mut scope, false)?);
		}
		return Ok(layer);
	}
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
		for (key, value, _) in &pairs {
			// A disallowed control character (e.g. NUL) in a value would be
			// interpolated verbatim into a compose scalar, where it is meaningless
			// at best and corrupts the container's config at worst. Reject it here,
			// at load time, with an error that names the originating env file and
			// key, instead of letting it surface later as a compose-file parse
			// error at a meaningless post-substitution offset. Only the explicit
			// `--env-file`/`env_file:` path errors; the `.env` fallback keeps its
			// historical pass-through behaviour.
			if let Some(bad) = super::first_disallowed_control_char(value) {
				return Err(ComposeError::EnvFile(format!(
					"env file {}: value of '{key}' contains a disallowed control \
					 character ({}); remove it before use",
					abs.display(),
					bad.escape_default(),
				)));
			}
		}
		layer.extend(expand(pairs, &process, &mut scope, true)?);
	}
	Ok(layer)
}
