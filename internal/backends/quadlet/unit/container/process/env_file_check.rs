//! What `podman --env-file` reads from a file, next to what `up` reads.
//!
//! `EnvironmentFile=` hands the file to `podman run --env-file`, which takes each
//! line as `KEY=VALUE` as written: no quotes removed, no `export`, no inline
//! comment, no `${VAR}`. `up` reads the same file as dotenv. A file that uses any
//! of that gives the container other values under Quadlet, so the keys that
//! differ are reported (never their values, which may be secrets).
//!
//! Only a key `up` reads in the file is named. A line `podman --env-file` reads
//! as a key of its own (the tail of a multi-line value, the key an `export`
//! prefix leaves behind) is counted and never named: such a key can be part of
//! a value that is a secret.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use crate::compose::types::EnvFileEntry;

/// What the two readers make of one env file.
pub(super) struct FileRead {
	/// The keys `up` reads in the file whose value `podman --env-file` reads
	/// another way, sorted.
	keys: Vec<String>,
	/// The lines `podman --env-file` reads as a key of its own: the ones inside
	/// a multi-line value, and the rest.
	multi_line: usize,
	other: usize,
}

impl FileRead {
	/// Nothing to report about this file.
	fn none() -> Self {
		Self {
			keys: Vec::new(),
			multi_line: 0,
			other: 0,
		}
	}

	/// Whether there is anything to report about this file.
	pub(super) fn is_empty(&self) -> bool {
		self.keys.is_empty() && self.multi_line == 0 && self.other == 0
	}

	/// What the container gets another value for: the keys `up` reads in the
	/// file, and the lines `podman --env-file` reads as a key of its own, which
	/// are counted but never named.
	pub(super) fn detail(&self) -> String {
		let mut parts: Vec<String> = self.keys.clone();
		if self.multi_line > 0 {
			parts.push(format!(
				"{} line(s) that are part of a multi-line value",
				self.multi_line
			));
		}
		if self.other > 0 {
			parts.push(format!(
				"{} line(s) that `podman --env-file` reads under a key of its own",
				self.other
			));
		}
		match parts.len() {
			0 => String::new(),
			1 => parts.remove(0),
			_ => {
				let last = parts.pop().unwrap_or_default();
				format!("{}, and {last}", parts.join(", "))
			}
		}
	}
}

/// What `podman --env-file` reads from each env file of a service, next to what
/// `up` reads from it. One entry per file, in the order of `env_file:`.
///
/// The reading of `up` is the one the container gets: the files before the one
/// being read, the service's own `environment:` and the project environment,
/// resolved the way `up` resolves them.
pub(super) fn reads(
	entries: &[EnvFileEntry],
	service_env: &HashMap<String, Option<String>>,
	base_dir: &Path,
) -> Vec<FileRead> {
	let files: Vec<Option<Scanned>> = entries
		.iter()
		.map(|entry| scan_file(base_dir, entry))
		.collect();
	(0..entries.len())
		.map(|index| read_file(&files, entries, service_env, base_dir, index))
		.collect()
}

/// The difference between what `podman --env-file` reads from the file at
/// `index` and what `up` reads from it.
fn read_file(
	files: &[Option<Scanned>],
	entries: &[EnvFileEntry],
	service_env: &HashMap<String, Option<String>>,
	base_dir: &Path,
	index: usize,
) -> FileRead {
	let Some(scanned) = &files[index] else {
		return FileRead::none();
	};
	// The reading of `up`: the files up to this one, the service's own
	// `environment:` and the project environment, as `up` resolves them.
	let Ok(up) =
		crate::env_file::load_env_file_entries_for(&entries[..=index], base_dir, service_env)
	else {
		return FileRead::none();
	};
	let mut read = FileRead::none();
	let mut keys = BTreeSet::new();
	for key in scanned.up_keys.iter().chain(scanned.bare_keys.iter()) {
		// A bare key is a host import: podman reads no key from that line, so
		// the two readers only disagree when podman reads the same key
		// somewhere else.
		if scanned.bare_keys.contains(key) && !scanned.podman.contains_key(key) {
			continue;
		}
		// A key the service's `environment:` sets, or a later file sets again,
		// reaches the container from there and not from this file, so a
		// difference in this file is not one the container sees.
		if service_env.contains_key(key) || set_after(files, index, key) {
			continue;
		}
		if scanned.podman.get(key) != up.get(key) {
			keys.insert(key.clone());
		}
	}
	read.keys = keys.into_iter().collect();
	for line in &scanned.lines {
		let Some((key, _)) = &line.podman else {
			continue;
		};
		// A key `up` reads in the file is named above, when the two readers
		// disagree about its value.
		if scanned.up_keys.contains(key) || scanned.bare_keys.contains(key) {
			continue;
		}
		if line.continuation {
			read.multi_line += 1;
		} else {
			read.other += 1;
		}
	}
	read
}

/// Whether a file after `index` sets `key`, so the container gets that one's
/// value and not the one of the file at `index`.
fn set_after(files: &[Option<Scanned>], index: usize, key: &str) -> bool {
	files.iter().skip(index + 1).any(|file| {
		file.as_ref()
			.is_some_and(|f| f.up_keys.contains(key) || f.podman.contains_key(key))
	})
}

/// One env file, as the two readers see it.
struct Scanned {
	/// The lines of the file.
	lines: Vec<Line>,
	/// The key and value `podman --env-file` reads from the file, the last
	/// line of a key winning.
	podman: HashMap<String, String>,
	/// The keys `up` reads in the file, the ones a line assigns.
	up_keys: BTreeSet<String>,
	/// The keys `up` reads in the file as a host import (a bare key).
	bare_keys: BTreeSet<String>,
}

/// Read the file of `entry` and take it apart. `None` when the file cannot be
/// read: a missing file is reported where it is loaded, and a file that is not
/// a regular file (a FIFO, say) is left to the loader that refuses it.
fn scan_file(base_dir: &Path, entry: &EnvFileEntry) -> Option<Scanned> {
	let path = base_dir.join(entry.path());
	let text = crate::filesystem::read_to_string_capped(&path).ok()?;
	let lines = scan(&text, is_raw(entry));
	let mut podman = HashMap::new();
	let mut up_keys = BTreeSet::new();
	let mut bare_keys = BTreeSet::new();
	for line in &lines {
		if let Some((key, value)) = &line.podman {
			podman.insert(key.clone(), value.clone());
		}
		if let Some(key) = &line.up_key {
			if line.bare {
				bare_keys.insert(key.clone());
			} else {
				up_keys.insert(key.clone());
			}
		}
	}
	Some(Scanned {
		lines,
		podman,
		up_keys,
		bare_keys,
	})
}

/// One physical line of an env file, as the two readers see it.
struct Line {
	/// The key and value `podman --env-file` reads from the line, if it reads
	/// any.
	podman: Option<(String, String)>,
	/// The key `up` reads from the line, if the line defines one.
	up_key: Option<String>,
	/// The key is a host import: the line names it and the value comes from
	/// the host.
	bare: bool,
	/// The line continues a quoted value that started on an earlier line.
	continuation: bool,
}

/// Read the lines of `text` as the two readers see them: `podman --env-file`
/// takes each line as it is written, `up` reads the file as dotenv, where a
/// quoted value that does not close on its line takes the lines that follow.
///
/// The line structure mirrors the dotenv reader, so the keys `up` reads in the
/// file are the keys of the lines that define one.
fn scan(text: &str, raw: bool) -> Vec<Line> {
	let text = text.strip_prefix('\u{feff}').unwrap_or(text);
	let mut lines = Vec::new();
	let mut rest = text.lines();
	while let Some(text_line) = rest.next() {
		let podman = podman_reads(text_line);
		if raw {
			// The `raw` format keeps every value as written and interpolates
			// nothing, the way `podman --env-file` reads a file, so both
			// readers read the same keys.
			let (up_key, bare) = match &podman {
				Some((key, _)) => (Some(key.clone()), false),
				None => {
					let key = text_line.trim();
					let host = !key.is_empty() && std::env::var(key).is_ok();
					(host.then(|| key.to_string()), host)
				}
			};
			lines.push(Line {
				podman,
				up_key,
				bare,
				continuation: false,
			});
			continue;
		}
		// `up` skips blank lines and comments, and drops the `export ` prefix.
		let trimmed = text_line.trim_start();
		if trimmed.is_empty() || trimmed.starts_with('#') {
			lines.push(Line {
				podman,
				up_key: None,
				bare: false,
				continuation: false,
			});
			continue;
		}
		let line = trimmed
			.strip_prefix("export ")
			.map(str::trim_start)
			.unwrap_or(trimmed);
		let Some((key, value)) = line.split_once('=') else {
			// A line without `=` names a key and imports its value from the
			// host.
			let key = line.trim();
			let bare = !key.is_empty() && std::env::var(key).is_ok();
			lines.push(Line {
				podman,
				up_key: bare.then(|| key.to_string()),
				bare,
				continuation: false,
			});
			continue;
		};
		let up_key = {
			let key = key.trim();
			if key.is_empty() {
				// A line without a name defines nothing, so it takes no value
				// and the lines after it are lines of their own.
				None
			} else {
				Some(key.to_string())
			}
		};
		// A quoted value that does not close on this line takes the lines that
		// follow as its own.
		let quote = up_key.as_ref().and_then(|_| {
			let value = value.trim_start();
			let quote = value.chars().next()?;
			if !matches!(quote, '"' | '\'') {
				return None;
			}
			if closing_quote(&value[quote.len_utf8()..], quote).is_some() {
				return None;
			}
			Some(quote)
		});
		lines.push(Line {
			podman,
			up_key,
			bare: false,
			continuation: false,
		});
		let Some(quote) = quote else {
			continue;
		};
		for next in rest.by_ref() {
			let podman = podman_reads(next);
			lines.push(Line {
				podman,
				up_key: None,
				bare: false,
				continuation: true,
			});
			if closing_quote(next, quote).is_some() {
				break;
			}
		}
	}
	lines
}

/// The key and value `podman --env-file` reads from a line, if it reads any:
/// the line trimmed on the left, skipped when it is empty or a comment, split
/// at its first `=`.
fn podman_reads(line: &str) -> Option<(String, String)> {
	let line = line.trim_start();
	if line.is_empty() || line.starts_with('#') {
		return None;
	}
	let (key, value) = line.split_once('=')?;
	Some((key.to_string(), value.to_string()))
}

/// The byte index of the closing `quote` in `s`, if there is one. In a
/// double-quoted value a backslash escapes the next byte, as in the dotenv
/// reader.
fn closing_quote(s: &str, quote: char) -> Option<usize> {
	let bytes = s.as_bytes();
	let q = quote as u8;
	let mut i = 0;
	while i < bytes.len() {
		let c = bytes[i];
		if quote == '"' && c == b'\\' {
			i += 2;
			continue;
		}
		if c == q {
			return Some(i);
		}
		i += 1;
	}
	None
}

/// Whether the entry asks for the `raw` format, where the value is kept as
/// written and nothing is interpolated.
fn is_raw(entry: &EnvFileEntry) -> bool {
	matches!(entry, EnvFileEntry::Config { format: Some(format), .. } if format == "raw")
}
