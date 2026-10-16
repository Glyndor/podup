//! Output-fidelity normalizers for the `config` render path: resolving relative
//! bind-mount sources to absolute paths, and quoting YAML 1.1 boolean-like
//! scalars. Split out of `config_render` so each file stays within the source
//! line limit. None of this changes runtime behaviour; it only shapes the
//! rendered `config` output to match `docker compose config`.

use std::path::{Component, Path, PathBuf};

use podup::compose::types::{ComposeFile, VolumeMount, VolumeType};

/// Rewrite each service's relative bind-mount source to an absolute path,
/// resolved against the project directory (matching `docker compose config`).
/// Only `.`-prefixed host paths are resolved: absolute paths, `~`, named
/// volumes, and Windows drive paths are left untouched, mirroring how the engine
/// classifies a short-form source. Mount semantics are unchanged; this only
/// affects the rendered output's source field.
pub(super) fn resolve_bind_sources(file: &mut ComposeFile, base_dir: &Path) {
	for svc in file.services.values_mut() {
		for mount in &mut svc.volumes {
			match mount {
				VolumeMount::Short(s) => {
					if let Some(rewritten) = rewrite_short_bind(s, base_dir) {
						*s = rewritten;
					}
				}
				VolumeMount::Long {
					volume_type: VolumeType::Bind,
					source: Some(src),
					..
				} => {
					if let Some(abs) = absolute_bind_source(src, base_dir) {
						*src = abs;
					}
				}
				VolumeMount::Long { .. } => {}
			}
		}
	}
}

/// Resolve the source of a short-form `source:target[:options]` bind mount,
/// returning the rewritten spec when the source is a relative host path. A spec
/// with no `:` separator is a single in-container target (anonymous volume), and
/// a non-`.` source is absolute or a named volume; both are left as-is.
fn rewrite_short_bind(spec: &str, base_dir: &Path) -> Option<String> {
	let (src, rest) = spec.split_once(':')?;
	let abs = absolute_bind_source(src, base_dir)?;
	Some(format!("{abs}:{rest}"))
}

/// Resolve a relative (`.`-prefixed) bind source to a lexically-normalized
/// absolute path against `base_dir`; return `None` for anything else.
fn absolute_bind_source(src: &str, base_dir: &Path) -> Option<String> {
	if !src.starts_with('.') {
		return None;
	}
	let joined = base_dir.join(src);
	let absolute = if joined.is_absolute() {
		joined
	} else {
		std::env::current_dir().unwrap_or_default().join(joined)
	};
	Some(normalize_lexically(&absolute))
}

/// Lexically clean a path: drop `.` components and collapse `..` against the
/// preceding normal component, without touching the filesystem (the path need
/// not exist, and symlinks must not be resolved). Matches docker compose, which
/// produces a cleaned absolute source rather than a canonicalized one.
fn normalize_lexically(path: &Path) -> String {
	let mut out: Vec<Component> = Vec::new();
	for comp in path.components() {
		match comp {
			Component::CurDir => {}
			Component::ParentDir => {
				if matches!(out.last(), Some(Component::Normal(_))) {
					out.pop();
				} else {
					out.push(comp);
				}
			}
			other => out.push(other),
		}
	}
	let mut pb = PathBuf::new();
	for c in out {
		pb.push(c.as_os_str());
	}
	pb.to_string_lossy().into_owned()
}

/// Quote any plain (unquoted) YAML scalar value that a strict YAML 1.1 reader
/// would misparse as a boolean (`yes`/`no`/`on`/`off`/`y`/`n`/`true`/`false`),
/// leaving already-quoted scalars, keys, and everything else untouched. Works on
/// yaml_serde's block output, where each scalar value sits after a `: `
/// mapping separator or a `- ` sequence marker on its own line.
///
/// A multiline string is the exception: it is written as a block scalar (`|`,
/// `>-`), whose lines are the string's content, so they pass through untouched.
pub(super) fn quote_yaml11_booleans(yaml: &str) -> String {
	// Set while inside a block scalar: the column its content is indented past.
	let mut block_parent: Option<usize> = None;
	let lines: Vec<String> = yaml
		.lines()
		.map(|line| {
			if let Some(parent) = block_parent {
				let indent = line.len() - line.trim_start_matches(' ').len();
				if line.trim().is_empty() || indent > parent {
					return line.to_string();
				}
				block_parent = None;
			}
			block_parent = block_scalar_parent(line);
			requote_bool_line(line)
		})
		.collect();
	let mut joined = lines.join("\n");
	// `lines()` drops the trailing newline serde_yaml emits; restore it.
	if yaml.ends_with('\n') {
		joined.push('\n');
	}
	joined
}

/// If `line` opens a block scalar (`key: |`, `- key: >-`, `- |`, `? |`, with an
/// optional tag before the indicator), the column its content is indented
/// past: the key's column for a mapping entry, the last `-` or `?` indicator's
/// for a sequence item or an explicit key. The block ends at the first
/// non-blank line that is not indented past it.
fn block_scalar_parent(line: &str) -> Option<usize> {
	let (head, header) = line.rsplit_once(' ')?;
	let mut indicator = header.chars();
	if !matches!(indicator.next(), Some('|' | '>'))
		|| !indicator.all(|c| c.is_ascii_digit() || c == '+' || c == '-')
	{
		return None;
	}
	let head = match head.rsplit_once(' ') {
		Some((before, tag)) if tag.starts_with('!') => before,
		_ => head,
	};
	if head.ends_with(':') {
		// The key may follow sequence dashes and explicit-key markers
		// (`- key: |`, `? key: |`); its own column is what the content passes.
		let mut key = head.trim_start_matches(' ');
		while let Some(rest) = key.strip_prefix("- ").or_else(|| key.strip_prefix("? ")) {
			key = rest.trim_start_matches(' ');
		}
		return Some(head.len() - key.len());
	}
	if !head.trim().is_empty() && head.split_whitespace().all(|t| t == "-" || t == "?") {
		return Some(head.len() - 1);
	}
	None
}

/// Quote the scalar value on a single block-YAML line if it is an unquoted YAML
/// 1.1 boolean token; otherwise return the line unchanged.
fn requote_bool_line(line: &str) -> String {
	let Some((prefix, value)) = split_block_scalar(line) else {
		return line.to_string();
	};
	if is_quoted(value) || !looks_like_yaml11_bool(value) {
		return line.to_string();
	}
	format!("{prefix}'{value}'")
}

/// Split a block-YAML line into `(prefix, value)` where `prefix` ends with the
/// `: ` mapping separator or the `- ` sequence marker. Returns `None` for lines
/// that carry no inline scalar value (nested keys, blank lines).
fn split_block_scalar(line: &str) -> Option<(&str, &str)> {
	// A mapping entry: the value follows the first `: `. An unquoted scalar value
	// never contains `: ` (serde_yaml would quote it), so the first occurrence is
	// the separator.
	if let Some(idx) = line.find(": ") {
		let (prefix, value) = line.split_at(idx + 2);
		if !value.is_empty() {
			return Some((prefix, value));
		}
	}
	// A scalar sequence item: `<indent>- value`.
	let rest = line.trim_start().strip_prefix("- ")?;
	if rest.is_empty() {
		return None;
	}
	let prefix_len = line.len() - rest.len();
	Some((&line[..prefix_len], rest))
}

/// Whether a scalar is already quoted (single or double), so it must be left
/// as-is rather than re-quoted.
fn is_quoted(s: &str) -> bool {
	s.starts_with('\'') || s.starts_with('"')
}

/// Whether a scalar matches a YAML 1.1 boolean token (case-insensitive) that a
/// 1.1 reader would resolve to true/false. The YAML 1.2 core schema serde_yaml
/// emits leaves these as plain strings.
fn looks_like_yaml11_bool(s: &str) -> bool {
	matches!(
		s.to_ascii_lowercase().as_str(),
		"y" | "yes" | "n" | "no" | "true" | "false" | "on" | "off"
	)
}

#[cfg(test)]
#[path = "config_normalize_tests.rs"]
mod tests;
