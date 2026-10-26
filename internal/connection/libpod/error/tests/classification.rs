use super::*;

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
fn is_timeout_matches_synthetic_timeout_error() {
	// Client-side timeouts carry status 0 and a "timed out" message; lifecycle
	// stop escalation keys off this.
	assert!(PodmanError::Api {
		status: 0,
		message: "timed out after 40s waiting for the Podman socket to respond".into(),
	}
	.is_timeout());
	// A real HTTP error (non-zero status) is not a timeout.
	assert!(!PodmanError::Api {
		status: 500,
		message: "boom".into(),
	}
	.is_timeout());
	// A status-0 error without the timeout marker is not a timeout.
	assert!(!PodmanError::Api {
		status: 0,
		message: "invalid API path".into(),
	}
	.is_timeout());
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
fn image_in_use_accepts_409_and_500_with_message() {
	// A 409 on image delete is always an in-use conflict.
	assert!(PodmanError::Api {
		status: 409,
		message: "image is in use by 1 container".into(),
	}
	.is_image_in_use());
	// Some Podman versions report it as a 500 naming the cause.
	assert!(PodmanError::Api {
		status: 500,
		message: "image used by a container: image in use".into(),
	}
	.is_image_in_use());
	// An unrelated 500 still propagates.
	assert!(!PodmanError::Api {
		status: 500,
		message: "internal error".into(),
	}
	.is_image_in_use());
	assert!(!PodmanError::Api {
		status: 404,
		message: "no such image".into(),
	}
	.is_image_in_use());
}

#[test]
fn state_conflict_recognises_pause_unpause_mismatches() {
	for msg in [
		"container abc is already paused: container state improper",
		"container abc is not paused: container state improper",
		"cannot pause container abc: container abc is not running",
		"unpausing container: container state improper",
	] {
		assert!(
			PodmanError::Api {
				status: 500,
				message: msg.into(),
			}
			.is_state_conflict(),
			"should treat {msg:?} as a state conflict"
		);
	}
	// A genuine failure is not a state conflict.
	assert!(!PodmanError::Api {
		status: 500,
		message: "internal error".into(),
	}
	.is_state_conflict());
	assert!(!PodmanError::Api {
		status: 404,
		message: "no such container".into(),
	}
	.is_state_conflict());
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
	assert!(!PodmanError::Json(serde_json::from_str::<u8>("bad").unwrap_err()).is_already_exists());
}

#[test]
fn is_status_does_not_match_field() {
	// The `is_status` predicate keys on the `Api` variant, so a `Field`
	// rejection is never mistaken for a generic API error.
	let e = PodmanError::Field {
		service: "web".into(),
		field: "pid".into(),
		value: "evil".into(),
		message: "x".into(),
	};
	assert!(!e.is_status(400));
	assert!(!e.is_status(500));
}
