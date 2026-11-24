//! What a URL may show where it is written to a log or a message: the host
//! and the path, never the credentials that can ride along in it.

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
		Some((scheme, rest)) => {
			let (authority, path) = match rest.find('/') {
				Some(slash) => rest.split_at(slash),
				None => (rest, ""),
			};
			let host = authority
				.rsplit_once('@')
				.map_or(authority, |(_, host)| host);
			format!("{scheme}://{host}{path}")
		}
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

/// `text` with every URL in it passed through [`redact_url`]. For the messages
/// Podman sends back, which repeat the URL of a build context they could not
/// clone, together with the query git echoes in its own message. A URL runs
/// from its scheme to the next blank, quote, bracket or comma.
pub(crate) fn redact_urls_in_text(text: &str) -> String {
	let mut out = String::with_capacity(text.len());
	let mut rest = text;
	while let Some(at) = rest.find("://") {
		let scheme_start = rest[..at]
			.rfind(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-')))
			.map_or(0, |i| i + 1);
		let after = at + "://".len();
		let end = rest[after..]
			.find(|c: char| {
				c.is_whitespace() || matches!(c, '"' | '\'' | '`' | '<' | '>' | '(' | ')' | ',')
			})
			.map_or(rest.len(), |i| after + i);
		out.push_str(&rest[..scheme_start]);
		out.push_str(&redact_url(&rest[scheme_start..end]));
		rest = &rest[end..];
	}
	out.push_str(rest);
	out
}

#[cfg(test)]
#[path = "redact_tests.rs"]
mod tests;
