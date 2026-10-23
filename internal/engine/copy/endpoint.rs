//! The `SERVICE:PATH` grammar of a `cp` argument.
//!
//! One side of a `cp` is a container reference and the other a local path, so
//! every argument has to be classified before any byte moves. The rule is
//! deliberately narrow: a colon splits the service from the container path,
//! and the two shapes a plain host path can look like (a Windows drive, a
//! trailing `/.`) are the exceptions that have to be recognised rather than
//! split on.

use crate::error::{ComposeError, Result};

/// Reject the `cp` endpoint forms podup explicitly does not support, with a
/// clear diagnostic rather than letting them fall through to a local path that
/// happens to be named `-` or `SERVICE:`.
///
/// - `-` (stdin/stdout streaming) is not implemented.
/// - `SERVICE:` (a colon with an empty container path) is a malformed reference.
///
/// A plain local path (no colon, or a colon that is part of an ordinary host
/// path / Windows drive) is left to [`parse_endpoint`].
pub(super) fn check_endpoint(s: &str) -> Result<()> {
	if s == "-" {
		return Err(ComposeError::Unsupported(
			"cp: stdin/stdout ('-') is not supported".into(),
		));
	}
	if let Some((svc, path)) = s.split_once(':') {
		// A Windows drive letter (`C:\...`) is a local path, not a service ref.
		#[cfg(windows)]
		if svc.len() == 1 && svc.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
			return Ok(());
		}
		if !svc.is_empty() && path.is_empty() {
			return Err(ComposeError::Copy(format!(
				"cp: empty container path in '{s}' (expected SERVICE:PATH)"
			)));
		}
	}
	Ok(())
}

pub(super) fn parse_endpoint(s: &str) -> Option<(&str, &str)> {
	if s == "-" {
		return None;
	}
	// `SERVICE:PATH`: colon must not be the first character and path cannot be empty.
	let (svc, path) = s.split_once(':')?;
	if svc.is_empty() || path.is_empty() {
		return None;
	}
	// On Windows, an absolute path like `C:\path` has a single-char drive prefix,
	// treat those as local paths, not service endpoints. This must NOT apply on
	// Unix, where a one-character service name (`c:/path`) is perfectly valid and
	// would otherwise be rejected as a bogus "drive".
	#[cfg(windows)]
	if svc.len() == 1 && svc.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
		return None;
	}
	Some((svc, path))
}

/// Pick the row name for the live board from the `cp` endpoint pair.
///
/// Always the destination side, since the destination is the side the
/// operator was thinking about when they typed the command: the local
/// path for container->host (`svc:/path -> /host/dst`), the service ref
/// for host->container (`/host/src -> svc:/path`). One branch decides,
/// because the endpoint parser has already rejected `SERVICE:PATH` on
/// both sides and `-` on either side.
pub(super) fn cp_row_name(_src: &str, dst: &str) -> String {
	dst.to_string()
}

/// Whether the source string was written as the contents cue: a trailing
/// `/.` (or just `.`) means "copy the directory's contents, not the directory
/// itself". Detected on the original string because `Path::new("payload/.")`
/// has `file_name() == Some("payload")`: by the time the path reaches the
/// packer the cue is gone. The cue is present when, after stripping any
/// trailing characters for which `std::path::is_separator` is true, the
/// source is exactly `.` or ends with a separator followed by `.`. On Unix a
/// backslash is an ordinary filename character, so the cue only fires on
/// the forward-slash shape there; on Windows it fires on both the
/// forward-slash and the backslash shape. `..` and a path whose last
/// component is `..` are not the cue, and an empty source is not the cue.
pub(super) fn has_dot_contents_suffix(src: &str) -> bool {
	let trimmed = src.trim_end_matches(std::path::is_separator);
	trimmed == "."
		|| trimmed
			.strip_suffix('.')
			.is_some_and(|rest| rest.ends_with(std::path::is_separator))
}
