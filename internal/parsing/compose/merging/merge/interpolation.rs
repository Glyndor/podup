use std::collections::{HashMap, HashSet};

use crate::error::Result;

use super::super::textual;
use super::guards::guard_alias_expansion;

/// Recursively interpolate every string scalar of a parsed YAML `value`.
///
/// Mapping values and sequence items are coerced through [`interpolate_scalar`]
/// (so an interpolated numeric/boolean keeps its YAML type, matching
/// docker-compose's typed fields); mapping keys are interpolated as plain text.
pub(super) fn interpolate_value(
	value: &mut serde_yaml::Value,
	vars: &HashMap<String, String>,
	spent: &mut usize,
	warned: &mut HashSet<String>,
) -> Result<()> {
	// String-only fields are selected by their direct schema context.
	let serde_yaml::Value::Mapping(root) = value else {
		return interpolate_in(value, vars, spent, warned, false, None);
	};
	if !mapping_needs_interp(root) {
		return Ok(());
	}
	let taken = std::mem::take(root);
	let mut rebuilt = serde_yaml::Mapping::with_capacity(taken.len());
	for (key, mut section) in taken {
		let key = interpolate_key(key, vars, spent, warned)?;
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
					let name = interpolate_key(name, vars, spent, warned)?;
					interpolate_entry(&mut entry, vars, spent, warned, string_key)?;
					rebuilt_entries.insert(name, entry);
				}
				*entries = rebuilt_entries;
			}
			(other, _) => interpolate_in(other, vars, spent, warned, false, None)?,
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
) -> Result<()> {
	match entry {
		serde_yaml::Value::Tagged(tagged) => {
			interpolate_entry(&mut tagged.value, vars, spent, warned, string_key)
		}
		other => interpolate_in(other, vars, spent, warned, false, Some(string_key)),
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
) -> Result<()> {
	match value {
		serde_yaml::Value::String(s) if s.contains('$') => {
			*value = if text {
				serde_yaml::Value::String(crate::substitute::substitute_budgeted(
					s, vars, spent, warned,
				)?)
			} else {
				interpolate_scalar(s, vars, spent, warned)?
			};
		}
		serde_yaml::Value::Tagged(tagged) => {
			interpolate_in(&mut tagged.value, vars, spent, warned, text, string_key)?;
		}
		serde_yaml::Value::Sequence(seq) => {
			// Skip the recursion when no element carries a `$`: a 100-service file
			// with no `${VAR}` references pays zero per-sequence iteration cost
			// (#1364). Nested mappings/sequences still need a recursive pre-check
			// so the inner one is not missed.
			if !seq.iter().any(value_needs_interp) {
				return Ok(());
			}
			for item in seq.iter_mut() {
				interpolate_in(item, vars, spent, warned, text, None)?;
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
				let key = interpolate_key(key, vars, spent, warned)?;
				let value_is_text =
					text || string_key.is_some_and(|is_string| key.as_str().is_some_and(is_string));
				interpolate_in(&mut val, vars, spent, warned, value_is_text, None)?;
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
) -> Result<serde_yaml::Value> {
	let resolved = crate::substitute::substitute_budgeted(s, vars, spent, warned)?;
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

/// Interpolate a mapping key. Keys stay strings (a key is never coerced to a
/// number/boolean) so the document shape is preserved.
///
/// A key spends the same document budget as a value. With its own budget per
/// key, a thousand keys of a megabyte each stayed under every limit and the
/// rebuilt mapping held a gigabyte.
fn interpolate_key(
	key: serde_yaml::Value,
	vars: &HashMap<String, String>,
	spent: &mut usize,
	warned: &mut HashSet<String>,
) -> Result<serde_yaml::Value> {
	match key {
		serde_yaml::Value::String(s) if s.contains('$') => Ok(serde_yaml::Value::String(
			crate::substitute::substitute_budgeted(&s, vars, spent, warned)?,
		)),
		other => Ok(other),
	}
}
