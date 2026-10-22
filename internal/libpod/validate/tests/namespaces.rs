use super::*;

/// The modes every slot shares.
#[test]
fn namespace_modes_accept_allow_list() {
	for field in [PID_FIELD, IPC_FIELD, UTS_FIELD, USERNS_FIELD, CGROUP_FIELD] {
		assert!(is_valid_namespace_mode(field, "host"), "{field}");
		assert!(is_valid_namespace_mode(field, "private"), "{field}");
		assert!(is_valid_namespace_mode(field, "pod"), "{field}");
		assert!(is_valid_namespace_mode(field, "container:abc"), "{field}");
		assert!(
			is_valid_namespace_mode(field, "ns:/run/netns/foo"),
			"{field}"
		);
	}
}

#[test]
fn namespace_modes_reject_unknown() {
	assert!(!is_valid_namespace_mode(PID_FIELD, "evil"));
	assert!(!is_valid_namespace_mode(PID_FIELD, ""));
	assert!(!is_valid_namespace_mode(PID_FIELD, "HOST"));
	assert!(!is_valid_namespace_mode(PID_FIELD, "container:"));
	assert!(!is_valid_namespace_mode(PID_FIELD, "ns:"));
}

/// A one-character path is still a path. The old check measured every
/// prefix against `"container:".len()`, so `ns:/x` failed for being short
/// rather than for being wrong.
#[test]
fn a_short_ns_path_is_accepted() {
	assert!(is_valid_namespace_mode(PID_FIELD, "ns:/x"));
}

/// Measured against podman 5.7.0: `shareable` and `none` parse for `ipc`
/// and are rejected by every other slot with "unrecognized namespace
/// mode". `shareable` is the one compose files reach for, and podup used
/// to refuse it.
#[test]
fn ipc_takes_shareable_and_none_but_no_other_slot_does() {
	assert!(is_valid_namespace_mode(IPC_FIELD, "shareable"));
	assert!(is_valid_namespace_mode(IPC_FIELD, "none"));
	for field in [PID_FIELD, UTS_FIELD, USERNS_FIELD, CGROUP_FIELD] {
		assert!(!is_valid_namespace_mode(field, "shareable"), "{field}");
		assert!(!is_valid_namespace_mode(field, "none"), "{field}");
	}
}

/// Measured against podman 5.7.0. `keep-id` is the standard rootless
/// answer to a file-ownership mismatch, and podup rejected it outright
/// (#1463). The option-carrying forms parse too.
#[test]
fn userns_takes_its_own_vocabulary() {
	for mode in [
		"keep-id",
		"auto",
		"nomap",
		"keep-id:uid=1000,gid=1000",
		"auto:size=65536",
	] {
		assert!(is_valid_namespace_mode(USERNS_FIELD, mode), "{mode}");
	}
	// And nowhere else.
	for field in [PID_FIELD, IPC_FIELD, UTS_FIELD, CGROUP_FIELD] {
		assert!(!is_valid_namespace_mode(field, "keep-id"), "{field}");
		assert!(!is_valid_namespace_mode(field, "auto"), "{field}");
		assert!(!is_valid_namespace_mode(field, "nomap"), "{field}");
	}
	// The option form still needs an option after the colon.
	assert!(!is_valid_namespace_mode(USERNS_FIELD, "keep-id:"));
	assert!(!is_valid_namespace_mode(USERNS_FIELD, "auto:"));
}

#[test]
fn userns_options_do_not_admit_unknown_modes_or_options_on_plain_modes() {
	for mode in [
		"unknown:size=65536",
		"host:uid=1000",
		"private:size=10",
		"nomap:size=10",
	] {
		let service = crate::compose::types::Service {
			userns_mode: Some(mode.into()),
			..Default::default()
		};
		let error = pre_validate_spec("web", &service).unwrap_err();
		let ComposeError::Podman(PodmanError::Field {
			service,
			field,
			message,
			..
		}) = error
		else {
			panic!("unknown modes must retain the field error");
		};
		assert_eq!(service, "web");
		assert_eq!(field, USERNS_FIELD);
		assert_eq!(
			message,
			format!(
				"namespace mode {mode:?} is not recognised; must be {}",
				allowed_namespace_modes(USERNS_FIELD)
			)
		);
	}
	for field in [PID_FIELD, IPC_FIELD, UTS_FIELD, CGROUP_FIELD] {
		assert!(!is_valid_namespace_mode(field, "auto:size=65536"));
		assert!(!is_valid_namespace_mode(field, "keep-id:uid=1000,gid=1000"));
	}
}

/// The error text has to name the modes the slot in hand accepts, not a
/// union that would send the reader after a value their slot rejects.
#[test]
fn the_error_lists_the_modes_for_that_slot() {
	let userns = allowed_namespace_modes(USERNS_FIELD);
	assert!(userns.contains("keep-id"), "{userns}");
	assert!(!userns.contains("shareable"), "{userns}");

	let ipc = allowed_namespace_modes(IPC_FIELD);
	assert!(ipc.contains("shareable"), "{ipc}");
	assert!(!ipc.contains("keep-id"), "{ipc}");

	let pid = allowed_namespace_modes(PID_FIELD);
	assert!(!pid.contains("keep-id"), "{pid}");
	assert!(!pid.contains("shareable"), "{pid}");

	// The list reads as a list: a separator between items and none before
	// the first. A separator emitted on the wrong side survived every
	// `contains` above, since the words were all still there.
	for list in [&userns, &ipc, &pid] {
		assert!(list.starts_with("one of "), "{list}");
		assert!(
			!list.starts_with("one of ,"),
			"separator before the first item: {list}"
		);
		assert!(
			!list.contains(",,") && !list.contains(", ,"),
			"doubled separator: {list}"
		);
		assert!(list.contains(", "), "no separator between items: {list}");
	}
}

#[test]
fn first_invalid_namespace_returns_first_failure() {
	let slots: Vec<(&str, Option<&str>)> = vec![
		(PID_FIELD, Some("host")),
		(IPC_FIELD, Some("evil")),
		(UTS_FIELD, Some("private")),
	];
	let (field, value, allowed) = first_invalid_namespace(&slots).unwrap();
	assert_eq!(field, IPC_FIELD);
	assert_eq!(value, "evil");
	assert!(allowed.contains("`host`"));
	assert!(allowed.contains("`private`"));
	assert!(allowed.contains("`container:<id-or-path>`"));
}

#[test]
fn first_invalid_namespace_passes_when_all_valid() {
	let slots: Vec<(&str, Option<&str>)> = vec![
		(PID_FIELD, Some("host")),
		(IPC_FIELD, Some("container:web")),
		(UTS_FIELD, None),
	];
	assert!(first_invalid_namespace(&slots).is_none());
}
