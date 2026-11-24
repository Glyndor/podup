//! What the redaction remembers: the URLs a build was given, and the user
//! information of the image references the engine was given, so that a message
//! which repeats only a part of them is cleaned too.
//!
//! The two are kept apart. A reference is learned only where an image enters,
//! and nothing but another reference can push it out.

use std::sync::{Mutex, OnceLock};

use super::{redact_url, split_userinfo, HIDDEN};

/// The shortest part of a known URL (user, password, query value) that is
/// replaced wherever it appears in a message. A shorter one would garble every
/// message it happens to be a substring of, and the structure of the URL is
/// what hides it.
const MIN_PART_LEN: usize = 4;

/// How many remembered URLs are kept; the oldest goes first.
const MAX_KNOWN: usize = 32;

/// The shortest user name of a reference that is remembered on its own.
const MIN_USER_LEN: usize = 6;

/// How many remembered references are kept; the oldest goes first.
const MAX_REFERENCES: usize = 64;

/// One URL the engine was given in full, with what has to be hidden from any
/// message that repeats it.
struct Known {
	url: String,
	shown: String,
	parts: Vec<String>,
}

fn known() -> &'static Mutex<Vec<Known>> {
	static KNOWN: OnceLock<Mutex<Vec<Known>>> = OnceLock::new();
	KNOWN.get_or_init(|| Mutex::new(Vec::new()))
}

/// The user information of the image references, in a list of its own: a
/// build context or any other value cannot push a reference out of it.
fn references() -> &'static Mutex<Vec<Known>> {
	static REFERENCES: OnceLock<Mutex<Vec<Known>>> = OnceLock::new();
	REFERENCES.get_or_init(|| Mutex::new(Vec::new()))
}

/// `value` with its `%XX` escapes decoded, when that is valid UTF-8.
pub(super) fn percent_decoded(value: &str) -> Option<String> {
	let bytes = value.as_bytes();
	let mut out = Vec::with_capacity(bytes.len());
	let mut i = 0;
	while i < bytes.len() {
		if bytes[i] == b'%' && i + 2 < bytes.len() {
			let byte = std::str::from_utf8(&bytes[i + 1..i + 3])
				.ok()
				.and_then(|hex| u8::from_str_radix(hex, 16).ok());
			if let Some(byte) = byte {
				out.push(byte);
				i += 3;
				continue;
			}
		}
		out.push(bytes[i]);
		i += 1;
	}
	String::from_utf8(out)
		.ok()
		.filter(|decoded| decoded != value)
}

/// The user, the password and every value of the query of `url`, as written
/// and decoded, longest first.
pub(super) fn parts_of(url: &str) -> Vec<String> {
	let no_fragment = url.split('#').next().unwrap_or(url);
	let (head, query) = match no_fragment.split_once('?') {
		Some((head, query)) => (head, Some(query)),
		None => (no_fragment, None),
	};
	let mut found: Vec<&str> = Vec::new();
	if let Some((_, rest)) = head.split_once("://") {
		if let (Some(userinfo), _) = split_userinfo(rest) {
			match userinfo.split_once(':') {
				Some((user, password)) => found.extend([user, password]),
				None => found.push(userinfo),
			}
		}
	}
	for pair in query.into_iter().flat_map(|q| q.split('&')) {
		found.push(pair.split_once('=').map_or(pair, |(_, value)| value));
	}
	let mut parts: Vec<String> = Vec::new();
	for part in found {
		if part.chars().count() < MIN_PART_LEN {
			continue;
		}
		parts.push(part.to_string());
		parts.extend(percent_decoded(part));
	}
	parts.sort_by_key(|part| std::cmp::Reverse(part.len()));
	parts.dedup();
	parts
}

/// Remember a URL the engine was given, so that a message that repeats it
/// exactly, or one of its credentials on its own (a tool that echoes only the
/// password, or the URL with the escapes undone), is cleaned even where the
/// shape of the URL does not say what is a credential.
pub(crate) fn remember_url(url: &str) {
	let shown = redact_url(url);
	if shown == url {
		return;
	}
	let mut known = known()
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
	if known.iter().all(|entry| entry.url != url) {
		// A process builds a handful of contexts; the bound keeps one that
		// builds many from growing without limit.
		if known.len() >= MAX_KNOWN {
			known.remove(0);
		}
		known.push(Known {
			url: url.to_string(),
			shown,
			parts: parts_of(url),
		});
	}
}

/// What stands for a hidden part while the text still has to be read for its
/// URLs: unlike [`HIDDEN`] it holds no `<`, which ends a URL, so a part that
/// sits inside a longer secret cannot cut the rest of that secret loose.
pub(super) const MARK: &str = "\u{0}";

/// [`MARK`] as a character.
pub(super) const MARK_CHAR: char = '\u{0}';

/// `text` with the URLs remembered by [`remember_url`], and their credentials,
/// replaced; each credential by `mark`.
pub(super) fn forget_known(text: &str, mark: &str) -> String {
	let urls_known = known()
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
	let references = references()
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
	let known: Vec<&Known> = urls_known.iter().chain(references.iter()).collect();
	// Longest first: a known value that holds another one is hidden whole, not
	// from where the shorter one starts.
	let mut urls: Vec<(&str, String)> = known
		.iter()
		.map(|entry| {
			// What shows as [`HIDDEN`] is user information. While the text is
			// still to be read, it keeps the `:` the reading looks for.
			let shown = match (entry.shown == HIDDEN, mark == MARK) {
				(true, true) => format!("{MARK}:{MARK}"),
				(true, false) => mark.to_string(),
				(false, _) => entry.shown.clone(),
			};
			(entry.url.as_str(), shown)
		})
		.collect();
	urls.sort_by_key(|(url, _)| std::cmp::Reverse(url.len()));
	let mut parts: Vec<&str> = known
		.iter()
		.flat_map(|entry| {
			entry
				.parts
				.iter()
				.filter(|part| !entry.shown.contains(part.as_str()))
				.map(String::as_str)
		})
		.collect();
	parts.sort_by_key(|part| std::cmp::Reverse(part.len()));
	let mut out = text.to_string();
	for (url, shown) in urls {
		out = out.replace(url, &shown);
	}
	for part in parts {
		out = out.replace(part, mark);
	}
	out
}

/// Remember the user information of a reference, so a message that repeats the
/// user or the password on its own is cleaned too.
pub(crate) fn remember_userinfo(userinfo: &str) {
	let mut parts: Vec<String> = Vec::new();
	// A short user name (`git`, `admin`) is a word of ordinary text, and hiding
	// it everywhere would garble messages that say nothing of a credential. A
	// token alone, with no `:`, is a secret.
	let pieces = match userinfo.split_once(':') {
		Some((user, secret)) => vec![(user, MIN_USER_LEN), (secret, MIN_PART_LEN)],
		None => vec![(userinfo, MIN_PART_LEN)],
	};
	for (part, least) in pieces {
		if part.chars().count() >= least && !part.contains(MARK) {
			parts.push(part.to_string());
			parts.extend(percent_decoded(part));
		}
	}
	if parts.is_empty() {
		return;
	}
	parts.sort_by_key(|part| std::cmp::Reverse(part.len()));
	parts.dedup();
	let mut known = references()
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
	if known.iter().all(|entry| entry.url != userinfo) {
		if known.len() >= MAX_REFERENCES {
			known.remove(0);
		}
		known.push(Known {
			url: userinfo.to_string(),
			shown: HIDDEN.to_string(),
			parts,
		});
	}
}
