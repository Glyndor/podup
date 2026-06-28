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
	/// The reachable Podman server speaks a libpod API version below the minimum
	/// podup supports. Carries the version string the server reported (empty when
	/// the server sent no `Libpod-API-Version` header).
	IncompatibleApiVersion { reported: String },
}

impl fmt::Display for PodmanError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Connect(e) => write!(f, "podman socket connection error: {e}"),
			Self::Hyper(e) => write!(f, "http error: {e}"),
			Self::Json(e) => write!(f, "json error: {e}"),
			Self::Api { status, message } => match conflict_hint(message) {
				Some(hint) => write!(f, "{hint} (podman: {message})"),
				None => write!(f, "podman API error (HTTP {status}): {message}"),
			},
			Self::IncompatibleApiVersion { reported } => {
				let reported = if reported.is_empty() {
					"an unknown version"
				} else {
					reported.as_str()
				};
				write!(
					f,
					"podup requires Podman >= 5.0; this server reports libpod API version {reported}"
				)
			}
		}
	}
}

/// A short, actionable hint for the common Podman container state-conflict
/// errors, so the CLI leads with plain guidance instead of the raw HTTP message
/// (which still follows in parentheses). Returns `None` for anything unrecognised
/// so the original message is shown verbatim. Pure, so it is unit-tested.
fn conflict_hint(message: &str) -> Option<&'static str> {
	let m = message.to_ascii_lowercase();
	if m.contains("without force") || (m.contains("cannot remove") && m.contains("running")) {
		Some("the container is running — stop it first, or pass `-f` to force removal")
	} else if m.contains("already paused") {
		Some("the container is already paused")
	} else if m.contains("not paused") {
		Some("the container is not paused")
	} else if m.contains("not running") || m.contains("can only kill running") {
		Some("the container is not running")
	} else {
		None
	}
}

impl std::error::Error for PodmanError {
	fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
		match self {
			Self::Connect(e) => Some(e),
			Self::Hyper(e) => Some(e),
			Self::Json(e) => Some(e),
			Self::Api { .. } | Self::IncompatibleApiVersion { .. } => None,
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

	/// True if this is the libpod 409 returned when `kill` targets a container
	/// that is not running ("can only kill running containers …"). `docker
	/// compose kill` is best-effort across all targets, so this is treated as an
	/// idempotent no-op rather than a fatal error that aborts the loop. The
	/// message is unique to the kill endpoint, so matching it cannot mask another
	/// op's 409.
	pub(crate) fn is_kill_of_stopped(&self) -> bool {
		matches!(
			self,
			Self::Api { status: 409, message }
				if message.to_ascii_lowercase().contains("can only kill running")
		)
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
	use super::{conflict_hint, PodmanError};

	#[test]
	fn conflict_hint_recognises_common_state_errors() {
		let rm = "cannot remove container abc as it is running - running or paused containers \
			cannot be removed without force: container state improper";
		assert!(conflict_hint(rm).unwrap().contains("-f"));
		assert!(conflict_hint("container abc is already paused")
			.unwrap()
			.contains("already paused"));
		assert!(conflict_hint("container abc is not paused")
			.unwrap()
			.contains("not paused"));
		assert!(
			conflict_hint("cannot kill container abc: container abc is not running")
				.unwrap()
				.contains("not running")
		);
		// libpod's kill-of-stopped 409 message ("can only kill running
		// containers …") gets the same friendly "not running" hint.
		assert!(conflict_hint(
			"can only kill running containers. abc is in state exited: container state improper"
		)
		.unwrap()
		.contains("not running"));
	}

	#[test]
	fn is_kill_of_stopped_matches_only_the_kill_409() {
		let stopped = PodmanError::Api {
			status: 409,
			message: "can only kill running containers. abc is in state exited: \
				container state improper"
				.into(),
		};
		assert!(stopped.is_kill_of_stopped());
		// A different 409 (e.g. already-paused) must not be swallowed by kill.
		let paused = PodmanError::Api {
			status: 409,
			message: "container abc is already paused".into(),
		};
		assert!(!paused.is_kill_of_stopped());
		// Wrong status, even with a matching message, is not a kill-of-stopped.
		let other = PodmanError::Api {
			status: 500,
			message: "can only kill running containers".into(),
		};
		assert!(!other.is_kill_of_stopped());
		assert!(
			!PodmanError::Json(serde_json::from_str::<u8>("bad").unwrap_err()).is_kill_of_stopped()
		);
	}

	#[test]
	fn conflict_hint_none_for_unrecognised() {
		assert!(conflict_hint("some unrelated error").is_none());
		assert!(conflict_hint("no such container: abc").is_none());
	}

	#[test]
	fn api_error_display_leads_with_hint_and_keeps_message() {
		let e = PodmanError::Api {
			status: 409,
			message: "container abc is already paused".into(),
		};
		let s = e.to_string();
		assert!(s.starts_with("the container is already paused"));
		assert!(s.contains("podman: container abc is already paused"));
	}

	#[test]
	fn api_error_display_raw_when_no_hint() {
		let e = PodmanError::Api {
			status: 500,
			message: "boom".into(),
		};
		assert_eq!(e.to_string(), "podman API error (HTTP 500): boom");
	}

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

	#[test]
	fn display_incompatible_api_version_reports_version() {
		let e = PodmanError::IncompatibleApiVersion {
			reported: "4.9.3".into(),
		};
		let msg = e.to_string();
		assert!(msg.contains("Podman >= 5.0"));
		assert!(msg.contains("4.9.3"));
	}

	#[test]
	fn display_incompatible_api_version_handles_missing_header() {
		// An empty reported version (no `Libpod-API-Version` header) renders a
		// readable placeholder rather than a blank.
		let e = PodmanError::IncompatibleApiVersion {
			reported: String::new(),
		};
		let msg = e.to_string();
		assert!(msg.contains("an unknown version"));
	}

	#[test]
	fn source_present_for_wrapped_errors_absent_for_owned() {
		use std::error::Error;
		// Wrapped lower-level errors expose their source...
		let connect = PodmanError::Connect(std::io::Error::new(
			std::io::ErrorKind::NotFound,
			"no socket",
		));
		assert!(connect.source().is_some());
		let json = PodmanError::Json(serde_json::from_str::<u8>("bad").unwrap_err());
		assert!(json.source().is_some());
		// ...while the owned variants have no underlying source.
		assert!(PodmanError::Api {
			status: 500,
			message: "x".into(),
		}
		.source()
		.is_none());
		assert!(PodmanError::IncompatibleApiVersion {
			reported: "4.0.0".into(),
		}
		.source()
		.is_none());
	}

	#[test]
	fn from_io_error_becomes_connect() {
		// The `?`-operator conversion an io error takes when bubbling out of the
		// client maps onto the Connect variant.
		let io = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused");
		let e: PodmanError = io.into();
		assert!(matches!(e, PodmanError::Connect(_)));
		assert!(e.to_string().contains("podman socket connection error"));
	}

	#[test]
	fn from_json_error_becomes_json() {
		let json_err = serde_json::from_str::<u8>("not-json").unwrap_err();
		let e: PodmanError = json_err.into();
		assert!(matches!(e, PodmanError::Json(_)));
		assert!(e.to_string().contains("json error"));
	}
}
