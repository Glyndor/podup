//! Error type for libpod REST API calls.

use std::fmt;

/// Errors from the Podman libpod REST API client.
#[derive(Debug)]
pub enum PodmanError {
	/// I/O or socket connection error.
	Connect(std::io::Error),
	/// Hyper HTTP error.
	Hyper(hyper::Error),
	/// JSON serialization or deserialization error.
	Json(serde_json::Error),
	/// Podman API returned an error response (4xx/5xx).
	Api { status: u16, message: String },
}

impl fmt::Display for PodmanError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Connect(e) => write!(f, "podman socket connection error: {e}"),
			Self::Hyper(e) => write!(f, "http error: {e}"),
			Self::Json(e) => write!(f, "json error: {e}"),
			Self::Api { status, message } => {
				write!(f, "podman API error (HTTP {status}): {message}")
			}
		}
	}
}

impl std::error::Error for PodmanError {
	fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
		match self {
			Self::Connect(e) => Some(e),
			Self::Hyper(e) => Some(e),
			Self::Json(e) => Some(e),
			Self::Api { .. } => None,
		}
	}
}

impl From<std::io::Error> for PodmanError {
	fn from(e: std::io::Error) -> Self {
		Self::Connect(e)
	}
}

impl From<hyper::Error> for PodmanError {
	fn from(e: hyper::Error) -> Self {
		Self::Hyper(e)
	}
}

impl From<serde_json::Error> for PodmanError {
	fn from(e: serde_json::Error) -> Self {
		Self::Json(e)
	}
}

/// Whether an API error has the given HTTP status code.
impl PodmanError {
	/// True if this is an API error carrying the given HTTP status code.
	pub fn is_status(&self, code: u16) -> bool {
		matches!(self, Self::Api { status, .. } if *status == code)
	}

	/// True if this API error reports that the resource already exists: an HTTP
	/// 409 conflict, or an HTTP 500 whose message says so. Podman's libpod
	/// volume-create endpoint returns 500 (not 409) for a duplicate name, so an
	/// idempotent create must accept both to let a re-`up` succeed.
	pub(crate) fn is_already_exists(&self) -> bool {
		match self {
			Self::Api { status: 409, .. } => true,
			Self::Api {
				status: 500,
				message,
			} => message.contains("already exists"),
			_ => false,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::PodmanError;

	#[test]
	fn is_status_matches_code() {
		let e = PodmanError::Api {
			status: 404,
			message: "not found".into(),
		};
		assert!(e.is_status(404));
		assert!(!e.is_status(200));
		assert!(!e.is_status(500));
	}

	#[test]
	fn is_status_false_for_non_api() {
		let e = PodmanError::Json(serde_json::from_str::<u8>("bad").unwrap_err());
		assert!(!e.is_status(404));
	}

	#[test]
	fn already_exists_accepts_409_and_500_with_message() {
		// 409 conflict: always an already-exists.
		assert!(PodmanError::Api {
			status: 409,
			message: "network already used".into(),
		}
		.is_already_exists());
		// 500 carrying the libpod "already exists" cause (Podman's volume-create
		// path) must also count as already-exists for idempotent create.
		assert!(PodmanError::Api {
			status: 500,
			message: "volume with name p_v already exists: volume already exists".into(),
		}
		.is_already_exists());
	}

	#[test]
	fn already_exists_false_for_other_errors() {
		// A 500 that is not an already-exists must still propagate.
		assert!(!PodmanError::Api {
			status: 500,
			message: "internal error".into(),
		}
		.is_already_exists());
		assert!(!PodmanError::Api {
			status: 404,
			message: "no such volume".into(),
		}
		.is_already_exists());
		assert!(
			!PodmanError::Json(serde_json::from_str::<u8>("bad").unwrap_err()).is_already_exists()
		);
	}

	#[test]
	fn display_api_error() {
		let e = PodmanError::Api {
			status: 500,
			message: "internal error".into(),
		};
		assert_eq!(e.to_string(), "podman API error (HTTP 500): internal error");
	}

	#[test]
	fn display_json_error() {
		let e = PodmanError::Json(serde_json::from_str::<u8>("bad").unwrap_err());
		assert!(e.to_string().contains("json error"));
	}

	#[test]
	fn display_connect_error() {
		let e = PodmanError::Connect(std::io::Error::new(
			std::io::ErrorKind::NotFound,
			"no socket",
		));
		assert!(e.to_string().contains("podman socket connection error"));
	}
}
