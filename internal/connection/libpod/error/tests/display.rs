use super::*;

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
fn conflict_hint_paused_rm_is_not_labelled_running() {
	// Podman's removal refusal for a *paused* container shares the "without
	// force" wording; the hint must say paused, not running.
	let paused = "cannot remove container abc as it is paused - running or paused containers \
			cannot be removed without force: container state improper";
	let hint = conflict_hint(paused).unwrap();
	assert!(hint.contains("paused"), "got {hint:?}");
	assert!(!hint.contains("running"), "must not say running: {hint:?}");
	assert!(hint.contains("-f"));
}

#[test]
fn conflict_hint_covers_kill_exec_and_start() {
	// kill a stopped container.
	assert!(
		conflict_hint("can only kill running containers. abc is in state exited")
			.unwrap()
			.contains("not running")
	);
	// exec into a stopped container.
	assert!(conflict_hint(
		"can only create exec sessions on running containers: container state improper"
	)
	.unwrap()
	.contains("not running"));
	// start a container that is not startable.
	assert!(conflict_hint(
		"unable to start container abc: container must be in Created or Stopped state to be \
			 started"
	)
	.unwrap()
	.contains("cannot be started"));
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
fn display_incompatible_api_version_offers_a_remedy() {
	let e = PodmanError::IncompatibleApiVersion {
		reported: "4.9.3".into(),
	};
	let msg = e.to_string();
	assert!(msg.contains("podman.io/docs/installation"), "got {msg:?}");
	assert!(msg.contains("Upgrade Podman"), "got {msg:?}");
}

#[test]
fn display_incompatible_api_version_keeps_diagnosis_before_remedy() {
	let e = PodmanError::IncompatibleApiVersion {
		reported: "4.9.3".into(),
	};
	let msg = e.to_string();
	let diagnosis = msg.find("4.9.3").expect("diagnosis should be present");
	let remedy = msg
		.find("Upgrade Podman")
		.expect("remedy should be present");
	assert!(
		diagnosis < remedy,
		"remedy must follow the diagnosis it explains: {msg:?}"
	);
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
fn field_error_names_service_and_field() {
	// A `Field` rendered for a container-create failure leads with the
	// service and field, then the value, so the operator sees the
	// offending entry without parsing the libpod body (#1357).
	let e = PodmanError::Field {
		service: "web".into(),
		field: "pid".into(),
		value: "evil".into(),
		message: "namespace not recognised".into(),
	};
	let msg = e.to_string();
	assert!(
		msg.starts_with("service.web: pid: namespace not recognised"),
		"got {msg:?}"
	);
	assert!(msg.contains("evil"));
}

#[test]
fn field_error_for_build_query_skips_service_prefix() {
	// Build queries have no service context, so the `service.X:` prefix
	// is omitted and the field name leads directly.
	let e = PodmanError::Field {
		service: String::new(),
		field: "build.args".into(),
		value: "MALFORMED\\nKEY".into(),
		message: "key is not a valid identifier".into(),
	};
	let msg = e.to_string();
	assert!(
		msg.starts_with("build.args: key is not a valid identifier"),
		"got {msg:?}"
	);
	assert!(!msg.contains("service."));
	assert!(msg.contains("MALFORMED\\nKEY"));
}
