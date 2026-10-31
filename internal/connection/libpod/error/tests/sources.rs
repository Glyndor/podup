use super::*;

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

#[test]
fn field_error_has_no_source() {
	// Like the other owned variants, `Field` has no underlying error to
	// chain; the explanation is in the variant's own fields.
	use std::error::Error;
	let e = PodmanError::Field {
		service: "web".into(),
		field: "pid".into(),
		value: "evil".into(),
		message: "namespace not recognised".into(),
	};
	assert!(e.source().is_none());
}
