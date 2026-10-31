// `mkfifo` is the only way to plant a FIFO node; it operates on the supplied path
// and never opens the FIFO. The opt-out is scoped to this test file.
#![allow(unsafe_code)]

use super::*;

fn unit(name: &str) -> QuadletUnit {
	QuadletUnit {
		filename: name.to_string(),
		contents: String::new(),
	}
}

fn write(dir: &Path, name: &str, owner: &str) {
	std::fs::write(dir.join(name), format!("# podup-owner: {owner}\n[Timer]\n")).unwrap();
}

#[test]
fn names_owned_timers_that_this_run_does_not_write() {
	let dir = tempfile::tempdir().unwrap();
	write(dir.path(), "proj-job-1.timer", "proj");
	write(dir.path(), "proj-job-2.timer", "proj");
	write(dir.path(), "other-job-1.timer", "other");
	std::fs::write(dir.path().join("proj-job-3.timer"), "[Timer]\n").unwrap();
	std::fs::write(
		dir.path().join("proj-web.container"),
		"# podup-owner: proj\n",
	)
	.unwrap();
	let written = [unit("proj-job-1.timer")];
	assert_eq!(
		stale_timers(dir.path(), "proj", &written),
		["proj-job-2.timer"]
	);
}

#[test]
fn a_missing_directory_has_no_stale_timers() {
	let dir = tempfile::tempdir().unwrap();
	assert!(stale_timers(&dir.path().join("absent"), "proj", &[]).is_empty());
}

/// Run `stale_timers` on a thread so a read that blocks fails the test instead
/// of hanging it.
#[cfg(unix)]
fn stale_within_five_seconds(dir: &Path) -> Vec<String> {
	let (tx, rx) = std::sync::mpsc::channel();
	let owned = dir.to_path_buf();
	std::thread::spawn(move || {
		let _ = tx.send(stale_timers(&owned, "proj", &[]));
	});
	rx.recv_timeout(std::time::Duration::from_secs(5))
		.expect("stale_timers must not block on a special file")
}

#[cfg(unix)]
#[test]
fn a_fifo_named_like_a_timer_is_skipped_without_blocking() {
	let dir = tempfile::tempdir().unwrap();
	let fifo = std::ffi::CString::new(
		dir.path()
			.join("proj-job-1.timer")
			.to_string_lossy()
			.into_owned(),
	)
	.unwrap();
	// SAFETY: `fifo` is a valid NUL-terminated path for the whole call.
	assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
	write(dir.path(), "proj-job-2.timer", "proj");
	assert_eq!(stale_within_five_seconds(dir.path()), ["proj-job-2.timer"]);
}

#[cfg(unix)]
#[test]
fn a_symlink_named_like_a_timer_is_never_followed() {
	let dir = tempfile::tempdir().unwrap();
	std::os::unix::fs::symlink("/dev/zero", dir.path().join("proj-job-1.timer")).unwrap();
	let target = dir.path().join("elsewhere");
	std::fs::write(&target, "# podup-owner: proj\n").unwrap();
	std::os::unix::fs::symlink(&target, dir.path().join("proj-job-2.timer")).unwrap();
	assert!(stale_within_five_seconds(dir.path()).is_empty());
}

#[test]
fn an_oversized_file_is_not_read() {
	let dir = tempfile::tempdir().unwrap();
	let mut text = String::from("# podup-owner: proj\n");
	text.push_str(&"x".repeat(MAX_TIMER_BYTES as usize + 1));
	std::fs::write(dir.path().join("proj-job-1.timer"), text).unwrap();
	assert!(stale_timers(dir.path(), "proj", &[]).is_empty());
}
