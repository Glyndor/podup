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

/// The shortest user name of a reference that is remembered on its own.
const MIN_USER_LEN: usize = 6;

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

/// What stands for a hidden part while the text still has to be read for its
/// URLs: unlike [`HIDDEN`] it holds no `<`, which ends a URL, so a part that
/// sits inside a longer secret cannot cut the rest of that secret loose.
const MARK: &str = "\u{0}";

/// [`MARK`] as a character.
const MARK_CHAR: char = '\u{0}';

/// `text` with the URLs remembered by [`remember_url`], and their credentials,
/// replaced; each credential by `mark`.
fn forget_known(text: &str, mark: &str) -> String {
	let known = known()
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
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

/// The characters that end a word of text, as they end a URL.
fn ends_a_word(c: char) -> bool {
	c.is_whitespace() || matches!(c, '"' | '<' | '>')
}

/// The user information of an image reference, or of an address written
/// `user:secret@host/path` with no scheme, and what follows it. The user
/// information is found the way it is in a URL with a scheme, so a password
/// that holds a `/` is told the same way. It holds a `:` with something on
/// each side, and an address (`user@example.com`) has no `:`.
///
/// A reference pinned by digest ends in `@algorithm:hex`. That tail is set
/// aside first, so `name:tag@sha256:...` is not read as user information and
/// `user:secret@host/img@sha256:...` is read for the one that it holds.
fn secret_userinfo(word: &str) -> Option<(&str, &str)> {
	let (base, digest) = match word.rfind('@') {
		Some(at) if is_digest(&word[at + 1..]) => (&word[..at], word.len() - at),
		_ => (word, 0),
	};
	let (Some(userinfo), after) = split_userinfo(base) else {
		return None;
	};
	let (user, secret) = userinfo.split_once(':')?;
	let host = after.split('/').next().unwrap_or(after);
	if user.is_empty() || secret.is_empty() || host.is_empty() {
		return None;
	}
	// What follows the user information in the word, digest included.
	let after = &word[word.len() - after.len() - digest..];
	Some((userinfo, after))
}

/// Whether `text` is `algorithm:hex`, the way an image digest is written.
fn is_digest(text: &str) -> bool {
	text.split_once(':').is_some_and(|(algorithm, hex)| {
		!algorithm.is_empty() && hex.len() >= 32 && hex.bytes().all(|b| b.is_ascii_hexdigit())
	})
}

/// Remember the user information of a reference, so a message that repeats the
/// user or the password on its own is cleaned too.
fn remember_userinfo(userinfo: &str) {
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
	let mut known = known()
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner());
	if known.iter().all(|entry| entry.url != userinfo) {
		if known.len() >= MAX_KNOWN {
			known.remove(0);
		}
		known.push(Known {
			url: userinfo.to_string(),
			shown: HIDDEN.to_string(),
			parts,
		});
	}
}

/// The user information of a value that is one image reference, as a name or a
/// segment of a path is: what comes before the last `@` that is not the one of
/// a digest, whatever it holds (a `/`, a number after a `:`, no `:` at all, a
/// blank). A `scheme://` in front is left out and given back as the prefix.
/// Returns the prefix, the user information, and what follows it.
fn reference_userinfo(value: &str) -> Option<(&str, &str, &str)> {
	let scheme_end = value.find("://").filter(|at| {
		*at > 0
			&& value[..*at]
				.chars()
				.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
	});
	let (prefix, rest) = match scheme_end {
		Some(at) => value.split_at(at + "://".len()),
		None => ("", value),
	};
	if rest.contains("://") {
		return None;
	}
	let base = match rest.rfind('@') {
		Some(at) if is_digest(&rest[at + 1..]) => &rest[..at],
		_ => rest,
	};
	let at = base.rfind('@')?;
	let (userinfo, after) = (&rest[..at], &rest[at + 1..]);
	(!userinfo.is_empty() && !after.is_empty()).then_some((prefix, userinfo, after))
}

/// Remember the credentials of an image reference the configuration or the
/// command line names, before any request carries it: what an error message
/// repeats of it is then hidden whatever path the message takes, and whether or
/// not anything is written about the reference at the log level in use.
pub(crate) fn remember_reference(reference: &str) {
	if reference.contains('@') {
		if let Some((_, userinfo, _)) = reference_userinfo(reference) {
			remember_userinfo(userinfo);
		}
	}
}

/// `text`, which holds no `scheme://`, with the user information of every word
/// that is written `user:secret@host/path` taken out: an image reference with
/// credentials in it, which Podman refuses and repeats in its message.
fn redact_words(text: &str) -> String {
	let mut out = String::with_capacity(text.len());
	let mut rest = text;
	while !rest.is_empty() {
		let blanks = rest.find(|c: char| !ends_a_word(c)).unwrap_or(rest.len());
		out.push_str(&rest[..blanks]);
		rest = &rest[blanks..];
		let end = rest.find(ends_a_word).unwrap_or(rest.len());
		let word = &rest[..end];
		rest = &rest[end..];
		// A mark in front of the reference (a quote, a bracket) is not part of it.
		// The mark of a known part may stand where the reference starts.
		let lead = word
			.find(|c: char| c.is_alphanumeric() || c == MARK_CHAR)
			.unwrap_or(word.len());
		let core = &word[lead..];
		let kept = core.trim_end_matches(TRAILING).len();
		match secret_userinfo(&core[..kept]) {
			Some((_, after)) => {
				out.push_str(&word[..lead]);
				out.push_str(after);
				out.push_str(&core[kept..]);
			}
			None => out.push_str(word),
		}
	}
	out
}

/// A request path as a log line may show it: each segment with its escapes
/// undone and then cleaned, so the reference an image is named by (one segment,
/// its `/` written `%2F`) reads as it was written and what holds credentials
/// is hidden. A segment with nothing to hide is kept as it came.
pub(crate) fn redact_request_path(path: &str) -> String {
	path.split('/')
		.map(|segment| {
			let decoded = percent_decoded(segment);
			let text = decoded.as_deref().unwrap_or(segment);
			let cleaned = redact_text(text, true);
			if cleaned == text {
				segment.to_string()
			} else {
				cleaned
			}
		})
		.collect::<Vec<_>>()
		.join("/")
}

/// `name` as a row or a line of progress may show it: the same cleaning as a
/// message, for the image references and the URLs that name a resource. It
/// formats and does not learn: see [`remember_reference`].
pub(crate) fn redact_label(name: &str) -> String {
	redact_text(name, true)
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
	redact_text(text, false)
}

/// [`redact_urls_in_text`]; with `whole` the text is one reference, read whole
/// before it is read as a message. Nothing is learned here: what a message says
/// is not trusted to teach what to hide, and what to hide is remembered where
/// the configuration enters, by [`remember_reference`].
///
/// What is already known is hidden before the text is read for its URLs, and
/// again after, so a credential written in a shape the reading does not
/// recognise is still hidden.
fn redact_text(text: &str, whole: bool) -> String {
	let text = forget_known(text, MARK);
	// A value that is known to be one reference (a name, a segment of a path) is
	// read whole first: a password may hold a blank, a quote or a bracket, which
	// end a word of a message and would cut it in two.
	if whole {
		if let Some((prefix, _, after)) = reference_userinfo(&text) {
			let shown = format!("{prefix}{after}");
			return forget_known(&shown, HIDDEN).replace(MARK, HIDDEN);
		}
	}
	let mut rest = text.as_str();
	let mut out = String::with_capacity(text.len());
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
		out.push_str(&redact_words(&rest[..start]));
		out.push_str(&redact_run(url));
		out.push_str(tail);
		rest = &rest[end..];
	}
	out.push_str(&redact_words(rest));
	forget_known(&out, HIDDEN).replace(MARK, HIDDEN)
}

#[cfg(test)]
#[path = "redact_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "redact_reference_tests.rs"]
mod reference_tests;
