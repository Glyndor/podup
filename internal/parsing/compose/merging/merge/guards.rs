use crate::error::{ComposeError, Result};

/// Upper bound on YAML alias references in a document that uses anchors, and on
/// the size of such a document. yaml_serde already aborts deeply *nested*
/// alias expansion (its repetition limit), but a flat document with many alias
/// references to a non-trivial anchor expands *linearly* while the `Value` tree
/// is built and can exhaust memory: a ~46 KB file can allocate gigabytes. Real
/// compose files use a handful of anchors, so these caps never trigger in
/// practice; the worst-case expansion they allow (refs × doc size) stays bounded.
pub(super) const MAX_ALIAS_REFS: usize = 100;
pub(super) const MAX_ALIAS_DOC_BYTES: usize = 512 * 1024;
/// Upper bound on the worst-case size the alias tree can reach in memory when
/// expanded. Each `*anchor` reference can copy up to the whole document, so
/// `refs × content.len()` is the safe upper bound on expansion size; the cap
/// stops a flat document with a small anchor referenced many times
/// (~450 KB / 63 refs reaches ~2.5 GB resident under the old caps) while
/// leaving real compose files untouched.
const MAX_ALIAS_EXPANSION_BYTES: usize = 4 * 1024 * 1024;

/// Upper bound on flow-style nesting depth (`[`/`{`). yaml_serde's own
/// recursion cap eventually rejects pathological nesting, but its tokenizer is
/// O(n^2) in the depth it scans, so a small file of deeply nested flow
/// collections (`[[[[…]]]]`) can burn quadratic CPU time before that cap fires.
/// Real compose files nest only a handful of levels, so this cheap pre-parse
/// pass bounds the parser's worst-case work without affecting any valid input.
pub(super) const MAX_FLOW_DEPTH: usize = 100;

/// Reject YAML documents whose alias use could amplify into an out-of-memory
/// expansion (a "billion-laughs" linear cousin) before they reach the parser.
pub(super) fn guard_alias_expansion(content: &str) -> Result<()> {
	let refs = count_alias_refs(content);
	if refs == 0 {
		// The scan is a lexical estimate, and a plain scalar such as `x 'y` is
		// read by it as an opened quote, which hides every alias after it. The
		// count of replayed nodes is the bound, so it runs whenever an alias
		// could be there.
		if content.contains('*') {
			super::super::alias_budget::within_limit(
				content,
				super::super::alias_budget::MAX_EXPANDED_NODES,
			)?;
		}
		return Ok(());
	}
	if content.len() > MAX_ALIAS_DOC_BYTES {
		return Err(ComposeError::Unsupported(format!(
			"compose document uses YAML aliases and is {} bytes; documents using anchors/aliases \
			 must be at most {MAX_ALIAS_DOC_BYTES} bytes; inline the repeated content instead",
			content.len()
		)));
	}
	if refs > MAX_ALIAS_REFS {
		return Err(ComposeError::Unsupported(format!(
			"compose document uses {refs} YAML alias references; at most {MAX_ALIAS_REFS} are \
			 allowed; inline the repeated content instead"
		)));
	}
	super::super::alias_budget::within_limit(
		content,
		super::super::alias_budget::MAX_EXPANDED_NODES,
	)?;
	let expansion = refs.saturating_mul(content.len());
	if expansion > MAX_ALIAS_EXPANSION_BYTES {
		return Err(ComposeError::Unsupported(format!(
			"compose document uses {refs} YAML alias references across {} bytes; \
			 expanded alias content would reach about {expansion} bytes; \
			 at most {MAX_ALIAS_EXPANSION_BYTES} bytes of expanded alias content \
			 are allowed; inline the repeated content instead",
			content.len()
		)));
	}
	Ok(())
}

/// Reject documents whose flow-style nesting (`[`/`{`) exceeds [`MAX_FLOW_DEPTH`]
/// before they reach the O(n^2) tokenizer. Brackets inside single/double-quoted
/// scalars and after a `#` comment are ignored, mirroring [`count_alias_refs`]'s
/// conservative scan; the count never fully parses YAML, it only bounds work.
pub(super) fn guard_flow_depth(content: &str) -> Result<()> {
	let mut depth: usize = 0;
	for line in content.lines() {
		let (mut in_single, mut in_double) = (false, false);
		let mut prev: Option<char> = None;
		let mut chars = line.chars().peekable();
		while let Some(c) = chars.next() {
			// A quote opens a quoted scalar only where a value can start, the same
			// reading `count_alias_refs` has: an apostrophe inside a plain scalar
			// (`a'b`) is text, and taking it for an opening quote hid every bracket
			// after it. Inside a quoted scalar the escapes are read too, `''` in a
			// single-quoted one and a backslash in a double-quoted one; otherwise
			// the closing quote is found early and the next quote opens a quoted
			// run where there is none. `#` starts a comment only after whitespace.
			let at_value_pos = matches!(prev, None | Some(' ' | '\t' | '[' | '{' | ',' | ':'));
			let after_space = matches!(prev, None | Some(' ' | '\t'));
			prev = Some(c);
			match c {
				'\'' if in_single && chars.peek() == Some(&'\'') => {
					chars.next();
				}
				'\\' if in_double => {
					chars.next();
				}
				'\'' if in_single => in_single = false,
				'"' if in_double => in_double = false,
				'\'' if !in_double && at_value_pos => in_single = true,
				'"' if !in_single && at_value_pos => in_double = true,
				'#' if !in_single && !in_double && after_space => break,
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
/// `yaml_serde`'s event stream is not part of its public API
/// (`Event::Alias` is `pub(crate)` in `yaml_serde::de`), so we hand-roll a
/// state machine that mirrors the YAML 1.2 lexer just closely enough to know
/// where a `*name` could legally be an alias. The previous scanner toggled
/// `in_single` on every `'` regardless of position inside a plain scalar,
/// which broke on `don't, *a *a`: once the toggle flipped, every `*a` on the
/// rest of the line was silently inside "a quoted scalar" and never counted,
/// the guard saw zero refs and returned Ok, and both the reference cap and the
/// document-size cap were bypassed. The fix:
///
///   - `'` and `"` only OPEN a quoted scalar when they sit at a value position
///     (preceded by start-of-input, whitespace, or one of `[`, `{`, `,`, `:`).
///     A `'` in the middle of a plain scalar (`don't`) stays a plain character.
///   - Once open, the matching close toggles unconditionally (the close of a
///     quoted scalar is always at a non-value position).
///   - `#` outside any quoted scalar starts a comment to end-of-line; `#` in
///     the middle of a plain scalar (`foo#x`) is a plain character and does
///     not start a comment.
///   - `*` is counted only at a value position, followed by an anchor-name
///     character (`[A-Za-z0-9_-]`), and never inside a quoted scalar or
///     comment.
///
/// Block scalars (`|`, `>`) are not tracked: real compose files do not put
/// alias references inside a block scalar value, and the file-level guards
/// (doc-size + expansion-size) still bound the worst case if they do.
pub(super) fn count_alias_refs(content: &str) -> usize {
	let mut count = 0;
	let mut chars = content.chars().peekable();
	let mut in_single = false;
	let mut in_double = false;
	let mut prev: Option<char> = None;
	while let Some(c) = chars.next() {
		let at_value_pos = matches!(
			prev,
			None | Some(' ' | '\t' | '\n' | '\r' | '[' | '{' | ',' | ':')
		);
		match c {
			'\'' if in_single => {
				in_single = false;
			}
			// `!in_double` is the half this was missing. Without it a `'`
			// inside a double-quoted scalar opened single-quote state, the
			// closing `"` cleared `in_double` and left `in_single` set for
			// the rest of the document, and every `*alias` after it was
			// counted as quoted text. `count_alias_refs` then returned 0,
			// the `refs == 0` early return fired, and BOTH caps were
			// skipped. Two files 18 bytes apart: without the poisoning
			// line the document is refused, with it accepted.
			'\'' if !in_single && !in_double && at_value_pos => {
				in_single = true;
			}
			'"' if in_double => {
				in_double = false;
			}
			// Symmetrical: a `"` inside a single-quoted scalar is ordinary
			// text and must not open double-quote state either.
			'"' if !in_double && !in_single && at_value_pos => {
				in_double = true;
			}
			'#' if !in_single && !in_double && at_value_pos => {
				for c in chars.by_ref() {
					if c == '\n' {
						break;
					}
				}
				prev = Some('\n');
				continue;
			}
			'*' if !in_single && !in_double && at_value_pos => {
				let next_ok = chars
					.peek()
					.is_some_and(|n| n.is_ascii_alphanumeric() || *n == '_' || *n == '-');
				if next_ok {
					count += 1;
				}
			}
			_ => {}
		}
		prev = Some(c);
	}
	count
}
