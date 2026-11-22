//! Status-code and version-string helpers: [`parse_error_message`](Client::parse_error_message),
//! [`check_status`](Client::check_status), and the [`meets_minimum`] predicate
//! that gates [`MIN_PODMAN_VERSION`](crate::libpod::client::MIN_PODMAN_VERSION).
//!
//! Split from `mod.rs` so the status-decoding pair lives next to its JSON
//! shape description and the version-string test surface it backs.

use hyper::StatusCode;

use super::{Client, PodmanError, Result};

/// Read a `Libpod-API-Version` string as `(major, minor, patch)`.
///
/// One leading `v` and surrounding whitespace are tolerated. The three parts
/// must be plain digit runs: a sign, an empty part, a missing part or a fourth
/// part makes the string unreadable. A pre-release or build suffix after the
/// patch (`6.3.0-dev`, `6.2.0-rc1`, `6.2.0+build`) is dropped, so a release
/// candidate is judged by the release it leads up to.
fn parse_version(version: &str) -> Option<(u64, u64, u64)> {
	let version = version.trim();
	let version = version.strip_prefix('v').unwrap_or(version);
	// What follows the first `-` or `+` is a pre-release or build suffix.
	let core = version.split(['-', '+']).next()?;
	let mut parts = core.split('.');
	let major = parse_part(parts.next()?)?;
	let minor = parse_part(parts.next()?)?;
	let patch = parse_part(parts.next()?)?;
	if parts.next().is_some() {
		return None;
	}
	Some((major, minor, patch))
}

/// One numeric part. `parse` refuses an empty string, a space and anything
/// that is not a digit; a sign cannot reach it, because `-` and `+` end the
/// version before the parts are split.
fn parse_part(part: &str) -> Option<u64> {
	part.parse().ok()
}

/// Whether a `Libpod-API-Version` string (e.g. `"6.2.0"`, `"6.3.1"`) is at or
/// above [`MIN_PODMAN_VERSION`](crate::libpod::client::MIN_PODMAN_VERSION).
/// A string that cannot be read is below the floor.
pub(crate) fn meets_minimum(version: &str) -> bool {
	meets_floor(version, crate::libpod::client::MIN_PODMAN_VERSION)
}

/// [`meets_minimum`] against an explicit floor. Major, minor and patch compare
/// as numbers, most significant first, so `6.10.0` is above `6.2.0`. The patch
/// part decides only when the floor's patch is not zero, which is why the
/// tests reach it through this function rather than through the real floor.
pub(crate) fn meets_floor(version: &str, floor: (u64, u64, u64)) -> bool {
	parse_version(version).is_some_and(|v| v >= floor)
}

impl Client {
	/// Extract the human-readable message from a libpod error body. The body is
	/// JSON shaped like `{"message": "...", "cause": "..."}`; prefer `message`,
	/// fall back to `cause`, and to the raw body when the JSON is malformed (a
	/// proxy or a 502 from a fronting process can return plain text). Pure, so
	/// `check_status` shares it without duplication.
	pub(crate) fn parse_error_message(body: &[u8]) -> String {
		#[derive(serde::Deserialize)]
		struct ApiError {
			cause: Option<String>,
			message: Option<String>,
		}

		if let Ok(e) = serde_json::from_slice::<ApiError>(body) {
			e.message
				.or(e.cause)
				.unwrap_or_else(|| String::from_utf8_lossy(body).into_owned())
		} else {
			String::from_utf8_lossy(body).into_owned()
		}
	}

	/// Check status code; on error parse the Podman error message.
	pub(in crate::connection::libpod::client) fn check_status(
		status: StatusCode,
		body: &[u8],
	) -> Result<()> {
		if status.is_success() {
			return Ok(());
		}
		Err(PodmanError::Api {
			status: status.as_u16(),
			message: Self::parse_error_message(body),
		})
	}
}
