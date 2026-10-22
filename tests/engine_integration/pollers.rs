//! The ways these tests wait for something to become observable, and the one
//! stderr check that has its own self-test.
use super::*;

/// Poll until reading `path` inside `container` yields exactly `expect` once
/// trimmed, or `secs` elapse. Returns whether it matched.
///
/// Reading state back out of the container is how these tests observe an effect
/// that a command's return value cannot show. The usual shape is an entrypoint
/// that appends a line on every start, which makes the file a container-scoped
/// count of how many times the process ran. `/proc/uptime` looks like the
/// obvious alternative and is not one: it is not namespaced, so it reports the
/// host's.
///
/// The comparison is exact rather than a substring, so "started twice" cannot be
/// satisfied by a container that started three times.
/// Poll until a file on the HOST reads exactly `expect` once trimmed, or `secs`
/// elapse. The host side of [`poll_container_file`], for the tests that observe
/// ordering through a bind mount shared by two containers.
pub(crate) async fn poll_host_file(path: std::path::PathBuf, expect: &str, secs: u64) -> bool {
	let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(secs);
	while tokio::time::Instant::now() < deadline {
		if let Ok(out) = std::fs::read_to_string(&path) {
			if out.trim() == expect {
				return true;
			}
		}
		tokio::time::sleep(std::time::Duration::from_millis(100)).await;
	}
	false
}

pub(crate) async fn poll_container_file(
	engine: &Engine,
	container: &str,
	path: &str,
	expect: &str,
	secs: u64,
) -> bool {
	let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(secs);
	while tokio::time::Instant::now() < deadline {
		if let Ok(out) = engine
			.test_exec_capture(container, vec!["cat".into(), path.into()])
			.await
		{
			if out.trim() == expect {
				return true;
			}
		}
		tokio::time::sleep(std::time::Duration::from_millis(100)).await;
	}
	false
}

/// Whether `path` exists inside `container` (as a file, directory, or
/// dangling symlink). `Some(true)` means present, `Some(false)` means
/// absent, `None` means the exec failed or the script produced
/// something other than the expected `present` / `absent` line.
///
/// The previous shape ran `ls <path>` and treated any non-zero exit as
/// "absent", which a transient exec failure (the exec endpoint
/// returning an error, the container being briefly unreachable, the
/// path carrying characters busybox `ls` rejects) reads as a deletion.
/// The script below uses `[ -e "$1" ] || [ -L "$1" ]` for the
/// existence check (covers regular files, directories, and dangling
/// symlinks alike), and the helper treats anything other than an exact
/// `present` / `absent` line as "unknown" rather than silently picking
/// one of the two.
pub(crate) async fn container_path_present(
	engine: &Engine,
	cname: &str,
	path: &str,
) -> Option<bool> {
	let script = "if [ -e \"$1\" ] || [ -L \"$1\" ]; then echo present; else echo absent; fi";
	let out = engine
		.test_exec_capture(
			cname,
			vec![
				"sh".into(),
				"-c".into(),
				script.into(),
				"sh".into(),
				path.into(),
			],
		)
		.await
		.ok()?;
	match out.trim() {
		"present" => Some(true),
		"absent" => Some(false),
		_ => None,
	}
}

/// Poll the test's condition until it holds or `timeout` elapses, while
/// watching the spawned watch task. On every tick the helper checks
/// `JoinHandle::is_finished()`; if the task completed, the helper awaits it
/// and panics with its own result so the failure names the real cause (the
/// `Watch` error returned from `Engine::watch`, e.g. an inotify exhaustion)
/// instead of the poll's deadline message, the misleading "never finished"
/// panic the test would otherwise raise when the watcher died silently.
///
/// `make_poll` is called on each tick; return `true` from its future when the
/// condition holds. Return `false` from the helper when the deadline elapsed
/// with the watch task still running, so the existing assertion can decide
/// whether a timeout is a pass or a fail.
///
/// The handle is `&mut` so the helper can `await` it without the test
/// having to take it back, and so a single borrow covers both the
/// `is_finished` check and the `await`. The closure is called fresh on each
/// tick (its future is not stored across iterations), so it can borrow from
/// the test body freely.
pub(crate) async fn poll_with_watch<F, Fut>(
	handle: &mut tokio::task::JoinHandle<podup::Result<()>>,
	timeout: std::time::Duration,
	mut make_poll: F,
) -> bool
where
	F: FnMut() -> Fut,
	Fut: std::future::Future<Output = bool>,
{
	let deadline = tokio::time::Instant::now() + timeout;
	loop {
		if handle.is_finished() {
			// `is_finished` is the only safe way to detect completion
			// without `await`ing first (which would consume the handle);
			// once it returns true, `await` resolves immediately with the
			// task's result.
			match handle.await {
				// `ComposeError::Watch` already prints `watch error:` in its
				// own `Display`; using the inner text here avoids the
				// double prefix and keeps the failure readable.
				Ok(Err(podup::ComposeError::Watch(s))) => {
					panic!("watch() returned early: {s}")
				}
				Ok(Err(e)) => panic!("watch() returned early: {e}"),
				Ok(Ok(())) => panic!("watch() returned early: Ok(())"),
				Err(e) => panic!("watch task panicked: {e}"),
			}
		}
		if make_poll().await {
			return true;
		}
		if tokio::time::Instant::now() >= deadline {
			return false;
		}
		tokio::time::sleep(std::time::Duration::from_millis(100)).await;
	}
}

/// Self-test for [`poll_with_watch`]: a spawned task that returns an error
/// immediately must make the helper panic with that error's text, not with
/// the poll's timeout message. The failure mode this guards is the watch
/// task dying silently: the test would otherwise wait out the deadline and
/// panic with "did not copy the file" or similar, blaming the wrong step.
#[tokio::test]
#[should_panic(expected = "watch() returned early: simulated watch failure")]
pub(crate) async fn poll_with_watch_surfaces_a_finished_watch_task_error() {
	let mut handle =
		tokio::spawn(async { Err(podup::ComposeError::Watch("simulated watch failure".into())) });
	// The poll never holds; without the helper's `is_finished` check the
	// timeout would expire and the helper would return `false` instead of
	// panicking. The `#[should_panic]` attribute is what binds the test to
	// the helper's panic contract; sabotaging the helper to skip
	// `is_finished` makes this test fail.
	let _ = poll_with_watch(
		&mut handle,
		std::time::Duration::from_millis(50),
		|| async { false },
	)
	.await;
}

/// Fail when `stderr` carries a libpod HTTP error, which podup prints as
/// `podman API error (HTTP <status>): <message>` (`internal/libpod/error/mod.rs`).
///
/// Checking for a bare status number such as "404" is wrong here: every
/// project name carries the test process id (`t<pid>-<tag>`), so a run whose
/// pid contains "404" (`t40415-dnvr` on the lane) prints a clean no-op line
/// that matches it (#1977).
#[track_caller]
pub(crate) fn assert_no_libpod_http_error(stderr: &str) {
	assert!(
		!stderr.contains("podman API error (HTTP"),
		"stderr leaked a libpod HTTP error: {stderr}"
	);
}

/// The lane line that tripped the bare "404" check must pass, and a real
/// libpod error line must still fail. Swapping the helper's check back to
/// `contains("404")` turns the first call red.
#[test]
pub(crate) fn assert_no_libpod_http_error_ignores_the_pid_but_catches_a_real_error() {
	assert_no_libpod_http_error("Network t40415-dnvr_default  Absent\n");
	let real = "podup: error: podman API error (HTTP 404): no such container\n";
	let res = std::panic::catch_unwind(|| assert_no_libpod_http_error(real));
	assert!(
		res.is_err(),
		"the helper must panic on a real `podman API error (HTTP ...)` line; got {real:?}"
	);
}

/// Poll until `cat`-ing `path` in the container yields `expect`, or `secs`
/// elapse. Read-only: used when the trigger already happened (initial_sync).
#[cfg(feature = "test-helpers")]
pub(crate) async fn poll_synced(
	engine: &Engine,
	cname: &str,
	path: &str,
	expect: &str,
	secs: u64,
) -> bool {
	let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(secs);
	while tokio::time::Instant::now() < deadline {
		if poll_synced_once(engine, cname, path, expect).await {
			return true;
		}
		tokio::time::sleep(std::time::Duration::from_millis(100)).await;
	}
	false
}

/// One attempt of the poll loop in [`poll_synced`]. Splits the loop body so a
/// test that drives the watcher through `poll_with_watch` can pass the
/// per-tick check in as a closure without re-implementing the inner
/// `test_exec_capture` call.
#[cfg(feature = "test-helpers")]
pub(crate) async fn poll_synced_once(
	engine: &Engine,
	cname: &str,
	path: &str,
	expect: &str,
) -> bool {
	if let Ok(out) = engine
		.test_exec_capture(cname, vec!["cat".into(), path.into()])
		.await
	{
		out.contains(expect)
	} else {
		false
	}
}
