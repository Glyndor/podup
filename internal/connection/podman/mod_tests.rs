use super::*;

#[test]
fn first_existing_picks_first_match() {
	let dir = tempfile::tempdir().unwrap();
	let hit = dir.path().join("podman.sock");
	std::fs::write(&hit, b"").unwrap();
	let candidates = vec![
		dir.path().join("missing.sock").display().to_string(),
		hit.display().to_string(),
		dir.path().join("later.sock").display().to_string(),
	];
	assert_eq!(first_existing(&candidates), Some(hit.display().to_string()));
}

#[test]
fn first_existing_none_when_no_candidate_exists() {
	let candidates = vec!["/nonexistent/podup-test/podman.sock".to_string()];
	assert_eq!(first_existing(&candidates), None);
}

#[test]
fn runtime_candidates_root_uses_system_socket() {
	let candidates = runtime_candidates(0, Some("/run/user/0"));
	assert_eq!(candidates, vec![ROOT_SOCKET.to_string()]);
}

#[test]
fn runtime_candidates_prefers_xdg_runtime_dir() {
	let candidates = runtime_candidates(1000, Some("/custom/runtime"));
	assert_eq!(
		candidates,
		vec![
			"/custom/runtime/podman/podman.sock".to_string(),
			"/run/user/1000/podman/podman.sock".to_string(),
		]
	);
}

#[test]
fn runtime_candidates_dedupes_default_runtime_dir() {
	let candidates = runtime_candidates(1000, Some("/run/user/1000"));
	assert_eq!(
		candidates,
		vec!["/run/user/1000/podman/podman.sock".to_string()]
	);
}

#[test]
fn runtime_candidates_ignores_empty_runtime_dir() {
	let candidates = runtime_candidates(1000, Some(""));
	assert_eq!(
		candidates,
		vec!["/run/user/1000/podman/podman.sock".to_string()]
	);
}

#[test]
fn machine_candidates_cover_known_layouts() {
	let machine_dir = "/Users/dev/.local/share/containers/podman/machine";
	assert_eq!(
		machine_candidates("/Users/dev"),
		vec![
			format!("{machine_dir}/podman.sock"),
			format!("{machine_dir}/applehv/podman.sock"),
			format!("{machine_dir}/vz/podman.sock"),
			format!("{machine_dir}/qemu/podman.sock"),
			format!("{machine_dir}/podman-machine-default/podman.sock"),
		]
	);
}

#[test]
fn connect_strips_unix_scheme() {
	let c = connect(Some("unix:///run/user/1000/podman/podman.sock")).unwrap();
	drop(c);
}

#[test]
fn connect_strips_npipe_scheme() {
	let c = connect(Some("npipe:////./pipe/podman")).unwrap();
	drop(c);
}

#[test]
fn connect_passes_plain_path_unchanged() {
	let c = connect(Some("/run/user/1000/podman/podman.sock")).unwrap();
	drop(c);
}

#[test]
fn connect_rejects_remote_schemes() {
	for raw in [
		"tcp://127.0.0.1:2375",
		"ssh://user@host/run/podman.sock",
		"http://localhost:8080",
		"https://localhost:8080",
		"fd://3",
	] {
		assert!(
			matches!(connect(Some(raw)), Err(ComposeError::Unsupported(_))),
			"{raw} should be rejected as unsupported"
		);
	}
}

#[test]
fn remote_scheme_ignores_local_sockets() {
	assert!(remote_scheme("/run/podman.sock").is_none());
	assert!(remote_scheme("unix:///run/podman.sock").is_none());
	assert!(remote_scheme("npipe:////./pipe/podman").is_none());
}

#[cfg(all(unix, not(target_os = "macos")))]
mod runtime_dir_trust {
	use super::super::trusted_runtime_dir;
	use std::os::unix::fs::PermissionsExt;

	fn dir(name: &str, mode: u32) -> std::path::PathBuf {
		let d = std::env::temp_dir().join(format!("podup-xdg-{name}-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&d);
		std::fs::create_dir_all(&d).unwrap();
		std::fs::set_permissions(&d, std::fs::Permissions::from_mode(mode)).unwrap();
		d
	}

	fn me() -> u32 {
		// SAFETY: getuid takes no arguments and cannot fail.
		unsafe { libc::getuid() }
	}

	#[test]
	fn an_own_private_directory_is_trusted() {
		let d = dir("own", 0o700);
		assert!(trusted_runtime_dir(d.to_str().unwrap(), me()));
		std::fs::remove_dir_all(&d).unwrap();
	}

	#[test]
	fn a_directory_others_can_write_is_not() {
		for mode in [0o777, 0o770, 0o707, 0o757] {
			let d = dir("open", mode);
			assert!(
				!trusted_runtime_dir(d.to_str().unwrap(), me()),
				"mode {mode:o}"
			);
			std::fs::remove_dir_all(&d).unwrap();
		}
	}

	#[test]
	fn another_owner_a_relative_path_a_link_or_a_file_is_not() {
		let d = dir("owner", 0o700);
		assert!(!trusted_runtime_dir(d.to_str().unwrap(), me() + 1));
		assert!(!trusted_runtime_dir("relative/runtime", me()));
		let link = d.join("link");
		std::os::unix::fs::symlink(&d, &link).unwrap();
		assert!(!trusted_runtime_dir(link.to_str().unwrap(), me()));
		let file = d.join("file");
		std::fs::write(&file, b"x").unwrap();
		assert!(!trusted_runtime_dir(file.to_str().unwrap(), me()));
		assert!(!trusted_runtime_dir(
			d.join("missing").to_str().unwrap(),
			me()
		));
		std::fs::remove_dir_all(&d).unwrap();
	}
}
