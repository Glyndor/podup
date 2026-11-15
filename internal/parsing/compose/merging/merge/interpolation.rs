use std::collections::{HashMap, HashSet};

use crate::error::Result;

use super::super::textual;
use super::guards::guard_alias_expansion;

/// Recursively interpolate every string scalar of a parsed YAML `value`.
///
/// Mapping values and sequence items are coerced through [`interpolate_scalar`]
/// (so an interpolated numeric/boolean keeps its YAML type, matching
/// docker-compose's typed fields); mapping keys are left as they are written,
/// because Docker Compose does not interpolate them.
pub(super) fn interpolate_value(
	value: &mut serde_yaml::Value,
	vars: &HashMap<String, String>,
	spent: &mut usize,
	warned: &mut HashSet<String>,
) -> Result<()> {
	// String-only fields are selected by their direct schema context.
	let serde_yaml::Value::Mapping(root) = value else {
		return interpolate_in(value, vars, spent, warned, false, None, "");
	};
	if !mapping_needs_interp(root) {
		return Ok(());
	}
	let taken = std::mem::take(root);
	let mut rebuilt = serde_yaml::Mapping::with_capacity(taken.len());
	for (key, mut section) in taken {
		let path = key_path("", &key);
		warn_if_key_looks_interpolated(&key, warned);
		let string_key: Option<fn(&str) -> bool> = match key.as_str() {
			Some("services" | "jobs") => Some(textual::is_string_key),
			Some("configs" | "secrets") => Some(textual::is_resource_string_key),
			_ => None,
		};
		// `services: !override {...}` is a tagged mapping; look through the tag.
		let mut inner = &mut section;
		while let serde_yaml::Value::Tagged(tagged) = inner {
			inner = &mut tagged.value;
		}
		match (inner, string_key) {
			(serde_yaml::Value::Mapping(entries), Some(string_key)) => {
				let taken_entries = std::mem::take(entries);
				let mut rebuilt_entries = serde_yaml::Mapping::with_capacity(taken_entries.len());
				for (name, mut entry) in taken_entries {
					warn_if_key_looks_interpolated(&name, warned);
					let entry_path = key_path(&path, &name);
					interpolate_entry(&mut entry, vars, spent, warned, string_key, &entry_path)?;
					rebuilt_entries.insert(name, entry);
				}
				*entries = rebuilt_entries;
			}
			(other, _) => interpolate_in(other, vars, spent, warned, false, None, &path)?,
		}
		rebuilt.insert(key, section);
	}
	*root = rebuilt;
	Ok(())
}

/// Interpolate an entry using its schema's direct string keys.
fn interpolate_entry(
	entry: &mut serde_yaml::Value,
	vars: &HashMap<String, String>,
	spent: &mut usize,
	warned: &mut HashSet<String>,
	string_key: fn(&str) -> bool,
	path: &str,
) -> Result<()> {
	match entry {
		serde_yaml::Value::Tagged(tagged) => {
			interpolate_entry(&mut tagged.value, vars, spent, warned, string_key, path)
		}
		other => interpolate_in(other, vars, spent, warned, false, Some(string_key), path),
	}
}

/// [`interpolate_value`] with `text` set when the value sits under a key whose
/// schema type is a string: its interpolated result stays text instead of
/// being read back as a number or a boolean (`user: ${UID}` with `UID=1000`
/// is the string `"1000"`, as in docker compose).
fn interpolate_in(
	value: &mut serde_yaml::Value,
	vars: &HashMap<String, String>,
	spent: &mut usize,
	warned: &mut HashSet<String>,
	text: bool,
	string_key: Option<fn(&str) -> bool>,
	path: &str,
) -> Result<()> {
	match value {
		serde_yaml::Value::String(s) if s.contains('$') => {
			*value = if text {
				serde_yaml::Value::String(
					crate::substitute::substitute_budgeted(s, vars, spent, warned)
						.map_err(|e| with_path(e, path))?,
				)
			} else {
				interpolate_scalar(s, vars, spent, warned, path)?
			};
		}
		serde_yaml::Value::Tagged(tagged) => {
			interpolate_in(
				&mut tagged.value,
				vars,
				spent,
				warned,
				text,
				string_key,
				path,
			)?;
		}
		serde_yaml::Value::Sequence(seq) => {
			// Skip the recursion when no element carries a `$`: a 100-service file
			// with no `${VAR}` references pays zero per-sequence iteration cost
			// (#1364). Nested mappings/sequences still need a recursive pre-check
			// so the inner one is not missed.
			if !seq.iter().any(value_needs_interp) {
				return Ok(());
			}
			// A list item has no name of its own, so its path ends in `[]`,
			// the way Docker Compose writes it.
			let item_path = format!("{path}.[]");
			for item in seq.iter_mut() {
				interpolate_in(item, vars, spent, warned, text, None, &item_path)?;
			}
		}
		serde_yaml::Value::Mapping(map) => {
			// Pre-check: only rebuild the mapping when something inside actually
			// carries a `$`. Without this gate every mapping allocates a fresh
			// `serde_yaml::Mapping` and re-inserts every key/value on every parse
			// For a 100-service file with ~10 fields each that is ~10k key/value
			// pairs moved for free (#1364). Scalar strings are already gated on
			// `s.contains('$')`; this mirrors that gate for the parent node.
			if !mapping_needs_interp(map) {
				return Ok(());
			}
			let taken = std::mem::take(map);
			let mut rebuilt = serde_yaml::Mapping::with_capacity(taken.len());
			for (key, mut val) in taken {
				warn_if_key_looks_interpolated(&key, warned);
				let child_path = key_path(path, &key);
				let value_is_text =
					text || string_key.is_some_and(|is_string| key.as_str().is_some_and(is_string));
				interpolate_in(
					&mut val,
					vars,
					spent,
					warned,
					value_is_text,
					None,
					&child_path,
				)?;
				rebuilt.insert(key, val);
			}
			*map = rebuilt;
		}
		_ => {}
	}
	Ok(())
}

/// Whether `map` (or any nested mapping/sequence/scalar it holds) contains a
/// `$`-bearing string that interpolation would touch. Used to gate the parent
/// mapping rebuild on real work (#1364).
fn mapping_needs_interp(map: &serde_yaml::Mapping) -> bool {
	for (k, v) in map {
		if let serde_yaml::Value::String(s) = k {
			if s.contains('$') {
				return true;
			}
		}
		if value_needs_interp(v) {
			return true;
		}
	}
	false
}

/// Whether `value` (a scalar, sequence, or mapping) contains a `$`-bearing
/// string anywhere reachable from it. Pure and total so the gate above is
/// unit-testable without standing up interpolation.
pub(super) fn value_needs_interp(value: &serde_yaml::Value) -> bool {
	match value {
		serde_yaml::Value::String(s) => s.contains('$'),
		serde_yaml::Value::Sequence(seq) => seq.iter().any(value_needs_interp),
		serde_yaml::Value::Mapping(map) => mapping_needs_interp(map),
		serde_yaml::Value::Tagged(tagged) => value_needs_interp(&tagged.value),
		_ => false,
	}
}

/// Interpolate a single string scalar and recover its YAML type.
///
/// An empty expansion stays an empty string (it must never collapse to `null`
/// and drop the owning key). Otherwise the resolved text is re-read as a YAML
/// scalar so `${N}` in a numeric position becomes a number and `${B}` a boolean,
/// matching docker-compose. Crucially, only *scalar* re-parses are adopted: a
/// result that parses into a mapping or sequence (an injected
/// `root\n  privileged: true`, or a trailing-colon `repo:`) is kept verbatim as
/// a string, so an attacker-influenced value can never introduce structure.
///
/// The re-parse goes through the same alias-expansion guard the file-level
/// path uses: a `.env`-supplied value that resolves to many alias references
/// would otherwise have `serde_yaml::from_str` build the full Value tree
/// (allocating memory proportional to refs × anchor size) before we discard
/// the result and keep the raw string.
fn interpolate_scalar(
	s: &str,
	vars: &HashMap<String, String>,
	spent: &mut usize,
	warned: &mut HashSet<String>,
	path: &str,
) -> Result<serde_yaml::Value> {
	let resolved = crate::substitute::substitute_budgeted(s, vars, spent, warned)
		.map_err(|e| with_path(e, path))?;
	if resolved.is_empty() {
		return Ok(serde_yaml::Value::String(String::new()));
	}
	guard_alias_expansion(&resolved)?;
	// The re-parse only exists to adopt a number or a boolean. Neither contains
	// a bracket or a brace, so text that does is a string and never reaches the
	// parser: a value from the environment cannot hand it a deeply nested
	// collection, which the file-level nesting guard never sees.
	if resolved.contains(['[', '{']) {
		return Ok(serde_yaml::Value::String(resolved));
	}
	match serde_yaml::from_str::<serde_yaml::Value>(&resolved) {
		Ok(v @ serde_yaml::Value::Bool(_)) => Ok(v),
		// A number is adopted only when printing it gives back the text that was
		// resolved: `1.10`, `0x1F`, `1e3` and `+1` would otherwise come out as
		// `1.1`, `31`, `1000.0` and `1`, which silently rewrites a value that
		// was meant as text (a version tag, a build id).
		Ok(serde_yaml::Value::Number(n)) if n.to_string() == resolved => {
			Ok(serde_yaml::Value::Number(n))
		}
		_ => Ok(serde_yaml::Value::String(resolved)),
	}
}

/// The path of the value under `key`, the way Docker Compose writes it when
/// it names the place of an interpolation error (`services.a.environment.V`).
fn key_path(parent: &str, key: &serde_yaml::Value) -> String {
	let name = match key {
		serde_yaml::Value::String(s) => s.clone(),
		other => format!("{other:?}"),
	};
	if parent.is_empty() {
		name
	} else {
		format!("{parent}.{name}")
	}
}

/// Name the place an interpolation error was found in. An error that already
/// names a place, as one from a nested expansion does, keeps the place it has.
fn with_path(error: crate::error::ComposeError, path: &str) -> crate::error::ComposeError {
	match error {
		crate::error::ComposeError::InvalidInterpolationFormat { path: at, template }
			if at.is_empty() =>
		{
			crate::error::ComposeError::InvalidInterpolationFormat {
				path: path.to_string(),
				template,
			}
		}
		other => other,
	}
}

/// Warn once per key that looks like it wants to be interpolated.
///
/// Docker Compose does not interpolate mapping keys: `labels: { ${KEY}: v }`
/// keeps the key as it was written, and so does this. The warning says so,
/// because a key that was meant to hold a variable is usually a mistake.
fn warn_if_key_looks_interpolated(key: &serde_yaml::Value, warned: &mut HashSet<String>) {
	let serde_yaml::Value::String(s) = key else {
		return;
	};
	if !key_looks_interpolated(s) {
		return;
	}
	if !crate::substitute::warnings::enabled() {
		return;
	}
	// The dedup set is shared with the unset-variable warnings, so the key is
	// marked with a prefix of its own and cannot collide with a variable name.
	if !warned.insert(format!("key:{s}")) {
		return;
	}
	tracing::warn!(
		"The key '{s}' is not interpolated. Docker Compose does not interpolate mapping keys, so it is used as written."
	);
}

/// Whether a key looks like it wants to be interpolated: it holds a `${…}` or
/// a `$` followed by a name.
fn key_looks_interpolated(key: &str) -> bool {
	if key.contains("${") {
		return true;
	}
	let bytes = key.as_bytes();
	bytes.iter().enumerate().any(|(i, &b)| {
		b == b'$'
			&& bytes
				.get(i + 1)
				.is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_')
	})
}
