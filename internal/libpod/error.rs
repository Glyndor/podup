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
	pub fn is_status(&self, code: u16) -> bool {
		matches!(self, Self::Api { status, .. } if *status == code)
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
