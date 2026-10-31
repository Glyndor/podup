use super::*;

#[test]
fn audit_sensitive_bind_mount_flags_socket_with_stronger_message() {
	// The container runtime socket is qualitatively worse than every
	// other entry on the list: the bind target itself is the attack
	// vector, not a directory of files. Pin both spellings (Docker and
	// Podman, `/var/run` and `/run`) and assert the message uses the
	// stronger wording rather than the directory-prefix wording.
	for src in [
		"/var/run/docker.sock",
		"/run/docker.sock",
		"/var/run/podman/podman.sock",
		"/run/podman/podman.sock",
	] {
		let yaml = format!(
			"services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - {src}:/sock\n"
		);
		let report = report_for(&yaml);
		let f = report
			.iter()
			.find(|f| f.check == "sensitive_bind_mount")
			.unwrap_or_else(|| panic!("{src} must fire sensitive_bind_mount; got {report:#?}"));
		assert!(
			f.reason.contains("container runtime socket"),
			"socket reason must distinguish itself from the directory wording: {f:?}"
		);
		assert!(
			f.reason.contains(src),
			"reason must name the socket path {src}: {f:?}"
		);
		// No sub-directory wording; the exact match wins over the prefix
		// matches even when both would apply (the socket path is a
		// subpath of `/run/podman` / `/run/docker`).
		assert!(
			!f.reason.contains("runtime directory"),
			"the exact-match wording must take priority over the directory-prefix wording: {f:?}"
		);
	}
}

#[test]
fn audit_sensitive_bind_mount_flags_runtime_directory_prefixes() {
	// Mounting the directory rather than the socket file still exposes
	// the socket to the container; a CI recipe that mounts `/run/podman`
	// whole is the more common mistake. Pin each prefix in turn so a
	// regression that drops one is caught by its own test.
	for src in [
		"/run/podman",
		"/run/podman/extra",
		"/var/run/podman",
		"/run/docker",
		"/var/run/docker",
	] {
		let yaml = format!(
			"services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - {src}:/data\n"
		);
		let report = report_for(&yaml);
		let f = report
			.iter()
			.find(|f| f.check == "sensitive_bind_mount")
			.unwrap_or_else(|| panic!("{src} must fire sensitive_bind_mount; got {report:#?}"));
		assert!(
			f.reason.contains(src),
			"reason must name the directory path {src}: {f:?}"
		);
		assert!(
			f.reason.contains("runtime directory"),
			"reason must cite the runtime-directory capability: {f:?}"
		);
	}
}

#[test]
fn audit_sensitive_bind_mount_flags_the_rootless_podman_socket() {
	// The rootless socket is the one a rootless operator has and the one
	// podup connects to by default. It sits under the user's runtime
	// directory, so the uid varies and the exact list cannot hold it.
	// Both spellings, and more than one uid, so a match that hard-coded
	// 1000 would fail here.
	for src in [
		"/run/user/1000/podman/podman.sock",
		"/run/user/0/podman/podman.sock",
		"/var/run/user/1001/podman/podman.sock",
	] {
		let yaml = format!(
			"services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - {src}:/sock\n"
		);
		let report = report_for(&yaml);
		let f = report
			.iter()
			.find(|f| f.check == "sensitive_bind_mount")
			.unwrap_or_else(|| panic!("{src} must fire sensitive_bind_mount; got {report:#?}"));
		assert!(
			f.reason.contains("container runtime socket") && f.reason.contains(src),
			"the rootless socket gets the socket wording and names its path: {f:?}"
		);
	}
}

#[test]
fn audit_sensitive_bind_mount_flags_the_rootless_podman_directory() {
	for src in ["/run/user/1000/podman", "/run/user/1000/podman/extra"] {
		let yaml = format!(
			"services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - {src}:/data\n"
		);
		let report = report_for(&yaml);
		let f = report
			.iter()
			.find(|f| f.check == "sensitive_bind_mount")
			.unwrap_or_else(|| panic!("{src} must fire sensitive_bind_mount; got {report:#?}"));
		assert!(
			f.reason.contains("rootless Podman runtime directory"),
			"the directory names the rootless runtime directory: {f:?}"
		);
	}
}

#[test]
fn audit_sensitive_bind_mount_leaves_the_rest_of_the_user_runtime_dir_alone() {
	// The runtime directory also holds the session bus, the audio server
	// and the display socket, which desktop containers mount on purpose.
	// Only the podman subtree is flagged, and only under an all-digit uid.
	for src in [
		"/run/user/1000/pulse/native",
		"/run/user/1000/bus",
		"/run/user/1000/wayland-0",
		"/run/user/1000",
		"/run/user/me/podman/podman.sock",
		"/run/user/1000/podmanx/podman.sock",
	] {
		let yaml = format!(
			"services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - {src}:/data\n"
		);
		let report = report_for(&yaml);
		assert!(
			!report.iter().any(|f| f.check == "sensitive_bind_mount"),
			"{src} must not fire sensitive_bind_mount; got {report:#?}"
		);
	}
}
