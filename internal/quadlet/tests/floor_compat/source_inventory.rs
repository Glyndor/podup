//! Source-level companion to `emitted_keys.rs`.
//!
//! The rendered-fixture tests in `emitted_keys.rs` only see keys reachable
//! from those fixtures. A post-floor key in an uncovered branch would
//! slip through. This is the source-level companion: read every emitter
//! file (the `.container` folder is walked at run time, the other units are
//! included by path), collect every `Key=` literal passed to the
//! unit builder, and assert each is floor-compliant. A new emitter that
//! writes a key the fixtures never reach still fails here.
//!
//! Split out into its own file so the small parser can sit alongside the
//! test it serves without pushing `emitted_keys.rs` past the per-file
//! code-line cap.

use std::collections::HashMap;

use crate::quadlet::min_podman::{is_at_floor, UnitType};

/// Every source file the `.container` unit is written from, read as the one
/// blob the parser sees. They are concatenated rather than read one at a time
/// because the `Section::new(...)` bindings live in the orchestrator: read on
/// their own, an emitter file's `svc.add(...)` would resolve no section and be
/// judged against the `.container` key table instead of being recognised as
/// systemd's `[Service]`.
///
/// The folder is walked at run time, subfolders included, not named file by
/// file through `include_str!`: an emitter added later is picked up without
/// anyone remembering to list it here, so it cannot escape the floor check by
/// being new.
fn container_sources() -> String {
	fn collect(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
		for entry in
			std::fs::read_dir(dir).unwrap_or_else(|e| panic!("cannot list {}: {e}", dir.display()))
		{
			let path = entry.expect("directory entry").path();
			if path.is_dir() {
				collect(&path, out);
			} else if path.extension().is_some_and(|ext| ext == "rs") {
				out.push(path);
			}
		}
	}
	let dir =
		std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("internal/quadlet/unit/container");
	let mut paths = Vec::new();
	collect(&dir, &mut paths);
	paths.sort();
	assert!(
		paths.iter().any(|path| path == &dir.join("mod.rs")),
		"the orchestrator mod.rs is missing from {}",
		dir.display()
	);
	// A newline between files, so the end of one can never run into the start
	// of the next and fuse two tokens the parser reads.
	paths
		.iter()
		.map(|path| std::fs::read_to_string(path).expect("emitter source is readable"))
		.collect::<Vec<_>>()
		.join("\n")
}

/// The rendered-fixture tests only see keys reachable from the
/// fixtures. A post-floor key in an uncovered branch would slip through.
/// This is the source-level companion: read every emitter file,
/// collect every `Key=` literal passed to the unit
/// builder, and assert each is at the floor (in 5.0 for the non-build
/// unit types, in 5.2 for `[Build]`) or is a floor-exempt passthrough
/// (`PodmanArgs`/`GlobalArgs`).
///
/// The check is stricter than the rendered-fixture tests' coverage
/// check: this sees every literal key the renderer passes to the
/// builder, regardless of whether it would be routed through
/// `PodmanArgs=` at render time, so a `Retry=` line that the post-5.2
/// table documents as a known 5.5.0 addition still fails here; a
/// renderer must re-route it through `PodmanArgs=` rather than emit it
/// natively.
#[test]
fn source_inventory_lists_every_emitted_key() {
	// Map each emitter file to the unit type it writes to. Files without
	// their own `Section::new(...)` binding (helpers that take a `&mut
	// Section` parameter) fall through to the file's intended unit type
	// when the parser cannot resolve a variable.
	let files: &[(&str, UnitType)] = &[
		("build.rs", UnitType::Build),
		("build_fields.rs", UnitType::Build),
		("container", UnitType::Container),
		("health.rs", UnitType::Container),
		("network.rs", UnitType::Network),
		("pod.rs", UnitType::Pod),
		("pod_fields.rs", UnitType::Pod),
		("security.rs", UnitType::Container),
		("volume.rs", UnitType::Volume),
	];

	for (filename, unit_type) in files {
		let source: String = match *filename {
			"build.rs" => include_str!("../../unit/build.rs").to_string(),
			"build_fields.rs" => include_str!("../../unit/build_fields.rs").to_string(),
			"container" => container_sources(),
			"health.rs" => include_str!("../../unit/health.rs").to_string(),
			"network.rs" => include_str!("../../unit/network.rs").to_string(),
			"pod.rs" => include_str!("../../unit/pod.rs").to_string(),
			"pod_fields.rs" => include_str!("../../unit/pod_fields.rs").to_string(),
			"security.rs" => include_str!("../../unit/security.rs").to_string(),
			"volume.rs" => include_str!("../../unit/volume.rs").to_string(),
			_ => panic!("missing include_str! branch for {filename}"),
		};

		// Build the `var -> section` map from every `Section::new("...")`
		// binding in the file. Variables that do not appear here (function
		// parameters, captured sections) fall through to the file's unit
		// type, since the only way to write to a Quadlet section without
		// a local binding is to take it as a parameter; and the only
		// callers do so explicitly.
		let mut var_to_section: HashMap<String, String> = HashMap::new();
		for (var, section) in extract_section_bindings(&source) {
			var_to_section.insert(var, section);
		}

		for (var, key) in extract_add_keys(&source) {
			let section = var_to_section.get(&var).map(String::as_str);
			// `[Unit]`, `[Install]` and `[Service]` are systemd sections,
			// not Quadlet sections: keys there go straight to systemd and
			// do not gate the floor.
			if matches!(section, Some("Unit" | "Install" | "Service")) {
				continue;
			}
			assert!(
				is_at_floor(*unit_type, &key),
				"internal/quadlet/unit/{filename} writes `{var}.add({key}, ...)` to \
				 {unit_type:?}, which is not in the Podman {} reference for that unit \
				 type; the unit-builder shape forces the native key, but a 5.0 host \
				 drops it. Route the value through PodmanArgs= if the key landed after \
				 the floor, or remove the emitter",
				if matches!(unit_type, UnitType::Build) {
					"5.2"
				} else {
					"5.0"
				}
			);
		}
	}
}

/// Walk `source` looking for `<ident>.add(<maybe-ws>"Key"<maybe-ws>,`. Returns
/// `(variable, key)` for every match whose key is a bare ASCII uppercase
/// identifier (Quadlet keys are CamelCase, systemd/Quadlet keywords are
/// uppercase; lowercase literals here would be a non-key string that
/// slipped past our `.add(` filter and is not worth flagging).
///
/// Handles both the single-line `<ident>.add("Key", ...)` shape and the
/// multi-line `<ident>.add(\n    "Key",\n    ...)` shape: the parser
/// skips whitespace between `(` and the opening `"`.
fn extract_add_keys(source: &str) -> Vec<(String, String)> {
	let mut out: Vec<(String, String)> = Vec::new();
	let needle = ".add(";
	let mut start = 0usize;
	while let Some(rel) = source[start..].find(needle) {
		let dot_pos = start + rel;
		let var = walk_ident_back(source, dot_pos);
		let after_open = dot_pos + needle.len();
		let (key, next) = match read_string_literal(source, after_open) {
			Some(k) => {
				let key_len = k.len();
				(
					k,
					after_open + 1 /* opening quote */ + key_len + 1, /* closing */
				)
			}
			None => {
				start = after_open;
				continue;
			}
		};
		// Skip the comma/whitespace after the key. If the next non-ws char
		// is not `,`, the call shape is `<ident>.add("Key" ...)` without
		// a comma; that is not the unit-builder shape, so skip it.
		let next_non_ws = source[next..]
			.char_indices()
			.find(|(_, c)| !c.is_whitespace())
			.map(|(i, _)| next + i);
		match next_non_ws {
			Some(p) if source.as_bytes()[p] == b',' => {}
			_ => {
				start = next;
				continue;
			}
		}
		if is_quadlet_key_shape(&key) {
			out.push((var, key));
		}
		start = next;
	}
	out
}

/// Walk `source[..pos]` backwards over ASCII alphanumerics and `_`, returning
/// the contiguous identifier (or empty string if the character before `.add`
/// is not part of one; the call is then a method on a literal or macro
/// result, which we cannot resolve and want to ignore).
fn walk_ident_back(source: &str, pos: usize) -> String {
	let prefix = &source[..pos];
	let mut chars: Vec<char> = prefix
		.chars()
		.rev()
		.take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
		.collect();
	chars.reverse();
	chars.into_iter().collect()
}

/// Read a `"..."` string literal starting at `pos`. Skips leading
/// whitespace first, so callers can point at the position right after an
/// opening paren. Returns the inner bytes on success and `None` if the
/// character at the (post-skip) position is not `"`.
fn read_string_literal(source: &str, pos: usize) -> Option<String> {
	let bytes = source.as_bytes();
	let mut j = pos;
	while j < bytes.len() && (bytes[j] as char).is_whitespace() {
		j += 1;
	}
	if j >= bytes.len() || bytes[j] != b'"' {
		return None;
	}
	// Find the closing quote. Quadlet key literals are short ASCII CamelCase
	// with no escapes, so the first `"` after the opening one is the close.
	let rest = &source[j + 1..];
	let end = rest.find('"')?;
	Some(rest[..end].to_string())
}

/// True when `key` looks like a Quadlet key: starts with an uppercase ASCII
/// letter, contains only ASCII alphanumerics. Catches the realistic shape of
/// `<Ident>.add("Foo", ...)` and rejects random string literals that happen
/// to appear after an `.add(` (e.g. `.add(format!("..."))`).
fn is_quadlet_key_shape(key: &str) -> bool {
	let mut chars = key.chars();
	match chars.next() {
		Some(c) if c.is_ascii_uppercase() => {}
		_ => return false,
	}
	chars.all(|c| c.is_ascii_alphanumeric())
}

/// Walk `source` looking for `<ident> = Section::new("Name")`. Returns
/// `(variable, section)` for every match whose section name is a bare
/// ASCII identifier. The variable is the LHS of the `=` in the binding;
/// `let`, `mut`, and intervening whitespace are skipped so the result is
/// the bare identifier (e.g. `unit`, `container`, `svc`).
///
/// Two binding shapes show up in the source: `let mut <var> = Section::new(...)`
/// and `let <var> = Section::new(...)`. Both reduce to the same `(var,
/// section)` tuple here: a later binding with the same variable overwrites
/// the earlier one (which matches Rust scoping; the shadowed binding is
/// dead by the time the new one is in scope).
fn extract_section_bindings(source: &str) -> Vec<(String, String)> {
	let mut out: Vec<(String, String)> = Vec::new();
	let needle = "Section::new(";
	let mut start = 0usize;
	while let Some(rel) = source[start..].find(needle) {
		let kw_pos = start + rel;
		// Find the `=` that introduces the binding. Search up to and
		// including the keyword position, since the `=` in `<var> =
		// Section::new(...)` sits one byte before the needle.
		let before = &source[..=kw_pos];
		let eq_pos = match before.rfind('=') {
			Some(p) if p < kw_pos => p,
			_ => {
				start = kw_pos + needle.len();
				continue;
			}
		};
		let var = extract_binding_var(source, eq_pos);
		let after_open = kw_pos + needle.len();
		match read_string_literal(source, after_open) {
			Some(name) if !var.is_empty() => out.push((var, name)),
			_ => {}
		}
		start = after_open;
	}
	out
}

/// Given the position of the `=` in a `let [mut] <var> = ...` binding,
/// walk back through whitespace and the optional `mut` keyword and return
/// the bare variable name (empty string if the binding is shaped
/// unexpectedly; the source-level test then treats the call site as a
/// non-Quadlet section, which is the conservative answer).
fn extract_binding_var(source: &str, eq_pos: usize) -> String {
	let bytes = source.as_bytes();
	// `eq_pos` points at `=`. The chars immediately before are the variable
	// name; we walk back from there over identifier chars, then optionally
	// skip a `mut` keyword (preceded by whitespace) and a `let` keyword
	// (also preceded by whitespace) to land the cursor at the start of the
	// statement; anything non-identifier between the cursor and `var` is a
	// shape the binding doesn't match.
	let mut end = eq_pos;
	while end > 0 && (bytes[end - 1] as char).is_whitespace() {
		end -= 1;
	}
	let mut start = end;
	while start > 0 {
		let c = bytes[start - 1] as char;
		if c.is_ascii_alphanumeric() || c == '_' {
			start -= 1;
		} else {
			break;
		}
	}
	if start == end {
		return String::new();
	}
	let var = source[start..end].to_string();
	// The identifier may be followed by whitespace and then the `mut`
	// keyword; skip those, then check for `let` as well. Anything else on
	// the LHS of `=` is a binding shape we don't recognise, so bail.
	let mut i = start;
	while i > 0 && (bytes[i - 1] as char).is_whitespace() {
		i -= 1;
	}
	if i >= 3 && &source[i - 3..i] == "mut" {
		i -= 3;
		while i > 0 && (bytes[i - 1] as char).is_whitespace() {
			i -= 1;
		}
	}
	if i >= 3 && &source[i - 3..i] == "let" {
		// Looks like `let [mut] <var> =`. The variable we already extracted
		// is the answer.
		return var;
	}
	// `<var> =` without a `let` keyword is not a binding; return the bare
	// identifier anyway and let the caller decide. (None of the actual
	// emitters bind a Section without `let`, so this branch is defensive.)
	var
}
