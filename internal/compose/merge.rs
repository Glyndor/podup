use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};

/// Upper bound on YAML alias references in a document that uses anchors, and on
/// the size of such a document. serde_yaml_ng already aborts deeply *nested*
/// alias expansion (its repetition limit), but a flat document with many alias
/// references to a non-trivial anchor expands *linearly* while the `Value` tree
/// is built and can exhaust memory — a ~46 KB file can allocate gigabytes. Real
/// compose files use a handful of anchors, so these caps never trigger in
/// practice; the worst-case expansion they allow (refs × doc size) stays bounded.
const MAX_ALIAS_REFS: usize = 100;
const MAX_ALIAS_DOC_BYTES: usize = 512 * 1024;

/// Upper bound on flow-style nesting depth (`[`/`{`). serde_yaml_ng's own
/// recursion cap eventually rejects pathological nesting, but its tokenizer is
/// O(n^2) in the depth it scans, so a small file of deeply nested flow
/// collections (`[[[[…]]]]`) can burn quadratic CPU time before that cap fires.
/// Real compose files nest only a handful of levels, so this cheap pre-parse
/// pass bounds the parser's worst-case work without affecting any valid input.
const MAX_FLOW_DEPTH: usize = 100;

pub(super) fn deserialize_with_merge(content: &str) -> Result<ComposeFile> {
	guard_flow_depth(content)?;
	guard_alias_expansion(content)?;
	let mut value: serde_yaml::Value = serde_yaml::from_str(content)?;
	apply_merge_keys(&mut value);
	let file: ComposeFile = serde_yaml::from_value(value)?;
	Ok(file)
}

/// Reject YAML documents whose alias use could amplify into an out-of-memory
/// expansion (a "billion-laughs" linear cousin) before they reach the parser.
fn guard_alias_expansion(content: &str) -> Result<()> {
	let refs = count_alias_refs(content);
	if refs == 0 {
		return Ok(());
	}
	if content.len() > MAX_ALIAS_DOC_BYTES {
		return Err(ComposeError::Unsupported(format!(
			"compose document uses YAML aliases and is {} bytes; documents using anchors/aliases \
			 must be at most {MAX_ALIAS_DOC_BYTES} bytes — inline the repeated content instead",
			content.len()
		)));
	}
	if refs > MAX_ALIAS_REFS {
		return Err(ComposeError::Unsupported(format!(
			"compose document uses {refs} YAML alias references; at most {MAX_ALIAS_REFS} are \
			 allowed — inline the repeated content instead"
		)));
	}
	Ok(())
}

/// Reject documents whose flow-style nesting (`[`/`{`) exceeds [`MAX_FLOW_DEPTH`]
/// before they reach the O(n^2) tokenizer. Brackets inside single/double-quoted
/// scalars and after a `#` comment are ignored, mirroring [`count_alias_refs`]'s
/// conservative scan; the count never fully parses YAML, it only bounds work.
fn guard_flow_depth(content: &str) -> Result<()> {
	let mut depth: usize = 0;
	for line in content.lines() {
		let (mut in_single, mut in_double) = (false, false);
		for c in line.chars() {
			match c {
				'\'' if !in_double => in_single = !in_single,
				'"' if !in_single => in_double = !in_double,
				'#' if !in_single && !in_double => break,
				'[' | '{' if !in_single && !in_double => {
					depth += 1;
					if depth > MAX_FLOW_DEPTH {
						return Err(ComposeError::Unsupported(format!(
							"compose document nests flow collections more than {MAX_FLOW_DEPTH} \
							 levels deep; flatten the structure"
						)));
					}
				}
				']' | '}' if !in_single && !in_double => depth = depth.saturating_sub(1),
				_ => {}
			}
		}
	}
	Ok(())
}

/// Count YAML alias references (`*anchor`) outside quoted scalars and comments.
///
/// A heuristic — it does not fully parse YAML — but it only needs to bound a
/// DoS and it is conservative: `*` inside single/double quotes or after `#` is
/// ignored, and an alias is counted only when `*` sits at a node position and is
/// followed by an anchor-name character.
fn count_alias_refs(content: &str) -> usize {
	let mut count = 0;
	for line in content.lines() {
		let mut chars = line.chars().peekable();
		let (mut in_single, mut in_double) = (false, false);
		let mut prev: Option<char> = None;
		while let Some(c) = chars.next() {
			match c {
				'\'' if !in_double => in_single = !in_single,
				'"' if !in_single => in_double = !in_double,
				'#' if !in_single && !in_double => break,
				'*' if !in_single && !in_double => {
					let at_node = matches!(prev, None | Some(' ' | '\t' | '[' | '{' | ',' | ':'));
					let next_ok = chars
						.peek()
						.is_some_and(|n| n.is_ascii_alphanumeric() || *n == '_' || *n == '-');
					if at_node && next_ok {
						count += 1;
					}
				}
				_ => {}
			}
			prev = Some(c);
		}
	}
	count
}

/// Recursively resolve YAML merge keys (`<<: *anchor`) in a `Value` tree.
///
/// serde_yaml_ng does not expose `apply_merge()` — this replaces it.
/// Merge semantics: keys from the anchor fill in only where the child has no value.
fn apply_merge_keys(value: &mut serde_yaml::Value) {
	match value {
		serde_yaml::Value::Mapping(mapping) => {
			for v in mapping.values_mut() {
				apply_merge_keys(v);
			}
			let merge_key = serde_yaml::Value::String("<<".to_string());
			if let Some(merge_val) = mapping.remove(&merge_key) {
				let bases: Vec<serde_yaml::Mapping> = match merge_val {
					serde_yaml::Value::Mapping(m) => vec![m],
					serde_yaml::Value::Sequence(seq) => seq
						.into_iter()
						.filter_map(|v| match v {
							serde_yaml::Value::Mapping(m) => Some(m),
							_ => None,
						})
						.collect(),
					_ => vec![],
				};
				for base in bases {
					for (k, v) in base {
						if !mapping.contains_key(&k) {
							mapping.insert(k, v);
						}
					}
				}
			}
		}
		serde_yaml::Value::Sequence(seq) => {
			for v in seq.iter_mut() {
				apply_merge_keys(v);
			}
		}
		_ => {}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	// alias-expansion guard

	#[test]
	fn count_alias_refs_ignores_quotes_comments_and_globs() {
		assert_eq!(count_alias_refs("a: &x 1\nb: *x\n"), 1);
		assert_eq!(count_alias_refs("c: [*x, *x, *x]\n"), 3);
		// `*` in quoted strings, comments, and globs (`*.txt`, `**`) are not aliases.
		assert_eq!(
			count_alias_refs("cmd: \"rm *x\"\nd: 1 # *x\ng: ['*.txt', '**']\n"),
			0
		);
	}

	#[test]
	fn guard_allows_normal_anchored_file() {
		// A handful of merge-key aliases in a small file is fine.
		let yaml = "x: &d {a: 1}\nweb: {<<: *d}\napi: {<<: *d}\n";
		assert!(guard_alias_expansion(yaml).is_ok());
	}

	#[test]
	fn guard_rejects_linear_alias_amplification() {
		// Many references to one anchor — the OOM vector serde_yaml_ng does not bound.
		let mut yaml = String::from("anchor: &a [x, y, z]\nlist:\n");
		for _ in 0..(MAX_ALIAS_REFS + 50) {
			yaml.push_str("  - *a\n");
		}
		let err = guard_alias_expansion(&yaml).unwrap_err();
		assert!(format!("{err}").contains("alias references"));
	}

	// flow-depth guard

	#[test]
	fn guard_flow_depth_allows_shallow_nesting() {
		// A handful of nested flow collections (typical compose) is fine.
		assert!(guard_flow_depth("a: [[1, 2], [3, 4]]\nb: {x: {y: 1}}\n").is_ok());
	}

	#[test]
	fn guard_flow_depth_rejects_pathological_nesting() {
		let deep = format!(
			"a: {}{}\n",
			"[".repeat(MAX_FLOW_DEPTH + 5),
			"]".repeat(MAX_FLOW_DEPTH + 5)
		);
		let err = guard_flow_depth(&deep).unwrap_err();
		assert!(format!("{err}").contains("flow collections"));
	}

	#[test]
	fn guard_flow_depth_ignores_brackets_in_quotes_and_comments() {
		// Brackets inside quoted scalars or after `#` do not count toward depth.
		let yaml = format!("cmd: \"{}\"  # {}\n", "[".repeat(200), "{".repeat(200));
		assert!(guard_flow_depth(&yaml).is_ok());
	}

	#[test]
	fn guard_rejects_large_aliased_document() {
		let mut yaml = String::from("anchor: &a 1\nb: *a\n");
		yaml.push_str(&format!("pad: \"{}\"\n", "p".repeat(MAX_ALIAS_DOC_BYTES)));
		let err = guard_alias_expansion(&yaml).unwrap_err();
		assert!(format!("{err}").contains("at most"));
	}
}
