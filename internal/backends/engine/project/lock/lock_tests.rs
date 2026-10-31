use super::*;
use crate::libpod::Client;

fn engine(project: &str) -> Engine {
	Engine::with_base_dir(
		Client::new("/nonexistent.sock"),
		project.into(),
		std::env::temp_dir(),
	)
}

#[test]
fn lock_acquire_release_reacquire() {
	// The lock lives in a per-user directory shared by every test process of that
	// user, so a fixed name collides with a second `cargo test` running at once.
	let e = engine(&format!("podup-locktest-{}", std::process::id()));
	let first = e.lock_project().expect("first acquire");
	drop(first);
	// Dropping the guard must release the flock so a fresh acquire succeeds.
	let _second = e.lock_project().expect("re-acquire after release");
}

#[test]
fn lock_rejects_unsafe_project_name() {
	assert!(engine("../evil").lock_project().is_err());
	assert!(engine(".hidden").lock_project().is_err());
}

#[test]
fn second_holder_blocks_until_first_releases() {
	use std::sync::atomic::{AtomicBool, Ordering};
	use std::sync::{Arc, Barrier};
	use std::thread;

	// Two engines contend for the same project lock. The first holds it; the
	// second must take the blocking `LOCK_EX` path in `acquire` and only
	// succeed after the first guard is dropped. `released` proves the order.
	let project = format!("podup-lock-contention-{}", std::process::id());
	let project = project.as_str();
	let held = engine(project).lock_project().expect("first acquire");

	let released = Arc::new(AtomicBool::new(false));
	let flag = Arc::clone(&released);
	// A rendezvous removes the need for a timing sleep: both threads clear the
	// barrier, then the waiter immediately enters the blocking acquire path.
	let barrier = Arc::new(Barrier::new(2));
	let waiter_barrier = Arc::clone(&barrier);
	let waiter = thread::spawn(move || {
		waiter_barrier.wait();
		let _guard = engine(project).lock_project().expect("second acquire");
		assert!(
			flag.load(Ordering::SeqCst),
			"second holder acquired the lock before the first released it"
		);
	});

	// The lock is exclusive, so the waiter cannot acquire until `held` is
	// dropped; the store is sequenced before the drop, so the waiter is
	// guaranteed to observe `released == true`. This is deterministic: it
	// never depends on how long the waiter takes to reach `flock`.
	barrier.wait();
	released.store(true, Ordering::SeqCst);
	drop(held);

	waiter.join().expect("waiter thread panicked");
}

#[cfg(unix)]
mod open_hostile {
	use super::super::open_lock_file;
	use std::ffi::CString;
	use std::os::unix::ffi::OsStrExt;
	use std::time::{Duration, Instant};

	fn dir(name: &str) -> std::path::PathBuf {
		let d = std::env::temp_dir().join(format!("podup-lock-{name}-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&d);
		std::fs::create_dir_all(&d).unwrap();
		d
	}

	#[test]
	fn a_regular_file_opens_and_keeps_its_mode() {
		let d = dir("regular");
		let path = d.join("a.lock");
		assert!(open_lock_file(&path).is_ok());
		assert!(
			open_lock_file(&path).is_ok(),
			"reopening an existing lock works"
		);
		std::fs::remove_dir_all(&d).unwrap();
	}

	#[test]
	fn a_fifo_with_no_reader_fails_fast_instead_of_blocking() {
		let d = dir("fifo");
		let path = d.join("a.lock");
		let c = CString::new(path.as_os_str().as_bytes()).unwrap();
		// SAFETY: `c` is a valid NUL-terminated path for the call.
		assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
		let started = Instant::now();
		assert!(open_lock_file(&path).is_err(), "a FIFO is no lock file");
		assert!(started.elapsed() < Duration::from_secs(5));
		std::fs::remove_dir_all(&d).unwrap();
	}

	#[test]
	fn a_symlink_is_refused_and_its_target_left_alone() {
		let d = dir("link");
		let victim = d.join("victim");
		std::fs::write(&victim, b"keep").unwrap();
		let path = d.join("a.lock");
		std::os::unix::fs::symlink(&victim, &path).unwrap();
		assert!(open_lock_file(&path).is_err());
		assert_eq!(std::fs::read(&victim).unwrap(), b"keep");
		let dev = d.join("b.lock");
		std::os::unix::fs::symlink("/dev/null", &dev).unwrap();
		assert!(
			open_lock_file(&dev).is_err(),
			"a link to a device is refused too"
		);
		std::fs::remove_dir_all(&d).unwrap();
	}

	#[test]
	fn a_directory_is_refused() {
		let d = dir("isdir");
		let path = d.join("a.lock");
		std::fs::create_dir(&path).unwrap();
		assert!(open_lock_file(&path).is_err());
		std::fs::remove_dir_all(&d).unwrap();
	}
}
