//! Status-code and version-string helpers: [`parse_error_message`](Client::parse_error_message),
//! [`check_status`](Client::check_status), and the [`meets_minimum`] predicate
//! that gates [`MIN_LIBPOD_API_MAJOR`](crate::libpod::client::MIN_LIBPOD_API_MAJOR).
//!
//! Split from `mod.rs` so the status-decoding pair lives next to its JSON
//! shape description and the version-string test surface it backs.

use hyper::StatusCode;

use super::{Client, PodmanError, Result};

/// Whether a `Libpod-API-Version` string (e.g. `"5.0.0"`, `"4.9.3"`) meets the
/// [`MIN_LIBPOD_API_MAJOR`](crate::libpod::client::MIN_LIBPOD_API_MAJOR).0
/// floor.
pub(crate) fn meets_minimum(version: &str) -> bool {
	version
		.trim()
		.trim_start_matches('v')
		.split('.')
		.next()
		.and_then(|major| major.parse::<u64>().ok())
		.is_some_and(|major| major >= crate::libpod::client::MIN_LIBPOD_API_MAJOR)
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
