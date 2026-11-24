//! What a URL may show where it is written to a log or a message: the host
//! and the path, never the credentials that can ride along in it.
//!
//! The text that reaches these functions is not trusted: it is a compose
//! value, or a message Podman sends back. Nothing here indexes a string at a
//! position it did not get from the string itself, so no input can split a
//! multibyte character.

use std::sync::{Mutex, OnceLock};

/// The shortest part of a known URL (user, password, query value) that is
/// replaced wherever it appears in a message. A shorter one would garble every
/// message it happens to be a substring of, and the structure of the URL is
/// what hides it.
const MIN_PART_LEN: usize = 4;

/// What stands where a known credential was.
const HIDDEN: &str = "<redacted>";

/// How many remembered URLs are kept; the oldest goes first.
const MAX_KNOWN: usize = 32;

/// Punctuation that ends a sentence and is not worth reading as part of the
/// URL in front of it.
const TRAILING: &[char] = &[
	'.', ',', ';', ':', '!', '?', ')', ']', '}', '\'', '`', '\u{2019}', '\u{201d}',
];

/// The quotation marks of [`TRAILING`]: the one that closes a quoted URL is
/// kept even when the URL had a query, since it cannot be part of a secret
/// worth hiding.
fn is_quote(c: char) -> bool {
	matches!(c, '\'' | '`' | '\u{2019}' | '\u{201d}')
}

/// The user information of `rest`, which is what follows `scheme://` in a URL
/// with no query and no fragment, and what is left after it.
///
/// The user information ends at the last `@` before the first `/`. A password
/// that holds a `/` is told from a path that holds an `@` by what comes before
/// the slash: `user:pa/ss@host` has a colon followed by something that is not a
/// port, `host:8080/a@b` has a port.
fn split_userinfo(rest: &str) -> (Option<&str>, &str) {
	let slash = rest.find('/').unwrap_or(rest.len());
	if let Some(at) = rest[..slash].rfind('@') {
		return (Some(&rest[..at]), &rest[at + 1..]);
	}
	if let Some(at) = rest.rfind('@') {
		let before = &rest[..at];
		if let Some(colon) = before.find(':').filter(|colon| *colon < slash) {
			let between = &rest[colon + 1..slash];
			if !between.is_empty() && !between.bytes().all(|b| b.is_ascii_digit()) {
				return (Some(before), &rest[at + 1..]);
			}
		}
	}
	(None, rest)
}

/// `url` without the user information (`user:token@`) and without the query
/// (`?token=...`), which are where a credential goes in a build context such as
/// `https://user:token@host/repo.git?token=abc#ref`. The fragment, which names
/// a Git reference and folder, stays.
///
/// A value with no `://` is returned as it is, except when it looks like
/// `user:secret@host:path`, an scp-style address with a password in it: the
/// part before the `@` goes. `git@host:org/repo.git` has no password and stays.
pub(crate) fn redact_url(url: &str) -> String {
	let (before_fragment, fragment) = match url.split_once('#') {
		Some((head, tail)) => (head, Some(tail)),
		None => (url, None),
	};
	let without_query = before_fragment
		.split_once('?')
		.map_or(before_fragment, |(head, _)| head);
	let mut out = match without_query.split_once("://") {
		Some((scheme, rest)) => format!("{scheme}://{}", split_userinfo(rest).1),
		None => match without_query.split_once('@') {
			Some((userinfo, rest)) if userinfo.contains(':') && !userinfo.contains('/') => {
				rest.to_string()
			}
			_ => without_query.to_string(),
		},
	};
	if let Some(fragment) = fragment {
		out.push('#');
		out.push_str(fragment);
	}
	out
}

/// The byte index where the scheme of the URL whose `://` sits at `at` starts.
/// Walks characters, so it always lands on a boundary.
fn scheme_start(text: &str, at: usize) -> usize {
	let mut start = at;
	for (i, c) in text[..at].char_indices().rev() {
		if c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-') {
			start = i;
		} else {
			break;
		}
	}
	start
}

/// Redact one run of text that is a URL, or several glued together (a proxy
/// URL that holds another one in its path): each starts where its scheme does.
fn redact_run(run: &str) -> String {
	let mut starts = vec![0];
	let mut from = 0;
	// The first `://` belongs to the first URL; each one after it opens a
	// URL of its own, which starts at its scheme.
	if let Some(first) = run.find("://") {
		from = first + 3;
	}
	while let Some(found) = run[from..].find("://") {
		let at = from + found;
		let start = scheme_start(run, at).max(*starts.last().unwrap_or(&0) + 1);
		if start < at {
			starts.push(start);
		}
		from = at + 3;
	}
	starts.push(run.len());
	starts
		.windows(2)
		.map(|w| redact_url(&run[w[0]..w[1]]))
		.collect()
}

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

/// `value` with its `%XX` escapes decoded, when that is valid UTF-8.
fn percent_decoded(value: &str) -> Option<String> {
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
fn parts_of(url: &str) -> Vec<String> {
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

/// `text` with the URLs remembered by [`remember_url`], and their credentials,
/// replaced.
fn forget_known(text: &str) -> String {
	let known = known()
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
	let mut out = text.to_string();
	for entry in known.iter() {
		out = out.replace(&entry.url, &entry.shown);
	}
	for entry in known.iter() {
		for part in entry
			.parts
			.iter()
			.filter(|part| !entry.shown.contains(part.as_str()))
		{
			out = out.replace(part.as_str(), HIDDEN);
		}
	}
	out
}

/// `text` with every URL in it passed through [`redact_url`], which for the
/// messages Podman sends back means the URL of a build context it could not
/// clone, together with the query git echoes in its own message.
///
/// A URL starts at its scheme and runs to the next blank, `"`, `<` or `>`. `,`,
/// `'`, `(` and `)` are valid in a password and in a token, so they do not end
/// a URL; the punctuation and the quotation marks that close a sentence or a
/// quoted URL are cut off the end and kept. A URL held inside another one, as a
/// proxy is, is cleaned on its own.
pub(crate) fn redact_urls_in_text(text: &str) -> String {
	let text = forget_known(text);
	let mut out = String::with_capacity(text.len());
	let mut rest = text.as_str();
	while let Some(at) = rest.find("://") {
		let start = scheme_start(rest, at);
		let after = at + "://".len();
		let end = rest[after..]
			.find(|c: char| c.is_whitespace() || matches!(c, '"' | '<' | '>'))
			.map_or(rest.len(), |i| after + i);
		let run = &rest[start..end];
		let kept = run.trim_end_matches(TRAILING).len();
		// What was cut off the end goes back. When the URL has a query the cut
		// may be the end of a token, so only what follows the first closing
		// quotation mark comes back: a quote is not part of any token worth
		// hiding, and what comes after it is text.
		let (url, tail) = (&run[..kept], &run[kept..]);
		let tail = if url.contains('?') {
			tail.char_indices()
				.find(|(_, c)| is_quote(*c))
				.map_or("", |(i, _)| &tail[i..])
		} else {
			tail
		};
		out.push_str(&rest[..start]);
		out.push_str(&redact_run(url));
		out.push_str(tail);
		rest = &rest[end..];
	}
	out.push_str(rest);
	out
}

#[cfg(test)]
#[path = "redact_tests.rs"]
mod tests;
