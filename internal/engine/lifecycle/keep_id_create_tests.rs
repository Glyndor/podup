//! #1955: `keep-id` creates are issued one at a time, everything else stays
//! concurrent. Measured against a fake libpod socket whose handlers hold each
//! request open and record how many were in flight at once.
//!
//! The handlers block with `std::thread::sleep`, so the cases run on a
//! multi-thread runtime: on the default single-thread one a blocked handler
//! would stall every other request and the count could never exceed 1, with
//! or without the lock. The control case is what shows the harness can see
//! concurrency at all.

#[cfg(unix)]
use super::*;
#[cfg(unix)]
use crate::engine::fake_podman;
#[cfg(unix)]
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(unix)]
use std::sync::Arc;
#[cfg(unix)]
use std::time::Duration;

/// Requests in flight right now, and the most seen at once.
#[cfg(unix)]
#[derive(Default)]
struct InFlight {
	now: AtomicUsize,
	max: AtomicUsize,
}

#[cfg(unix)]
impl InFlight {
	fn hold(&self, d: Duration) {
		let n = self.now.fetch_add(1, Ordering::SeqCst) + 1;
		self.max.fetch_max(n, Ordering::SeqCst);
		std::thread::sleep(d);
		self.now.fetch_sub(1, Ordering::SeqCst);
	}
}

/// `up` a three-replica service with the given `userns_mode` and return the
/// most creates and the most starts that were in flight at once. Starts are
/// held longer than creates so that, with creates in series, the three
/// starts still overlap.
#[cfg(unix)]
async fn max_in_flight(userns_mode: Option<&str>) -> (usize, usize) {
	let creates = Arc::new(InFlight::default());
	let starts = Arc::new(InFlight::default());
	let (c, s) = (creates.clone(), starts.clone());
	let fake = fake_podman::start(move |method, target| {
		if method == "GET" && target.contains("/containers/json") {
			(200, "[]".to_string())
		} else if method == "POST" && target.contains("/images/pull") {
			(200, String::new())
		} else if method == "POST" && target.contains("/containers/create") {
			c.hold(Duration::from_millis(100));
			(200, "{}".to_string())
		} else if method == "POST" && target.contains("/start") {
			s.hold(Duration::from_millis(400));
			(200, String::new())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let engine = Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir());
	let userns = userns_mode
		.map(|m| format!("    userns_mode: \"{m}\"\n"))
		.unwrap_or_default();
	let file = crate::parse_str(&format!(
		"services:\n  web:\n    image: img\n    scale: 3\n{userns}"
	))
	.unwrap();
	engine
		.up_with_options(&file, false, &[], &[], false, false, false, false)
		.await
		.expect("up against the fake must succeed");
	let seen = fake.requests.lock().unwrap();
	let count = |path: &str| {
		seen.iter()
			.filter(|r| r.contains("POST") && r.contains(path))
			.count()
	};
	assert_eq!(count("/containers/create"), 3, "{seen:?}");
	assert_eq!(count("/start"), 3, "{seen:?}");
	(
		creates.max.load(Ordering::SeqCst),
		starts.max.load(Ordering::SeqCst),
	)
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn keep_id_creates_run_one_at_a_time() {
	for mode in ["keep-id", "keep-id:uid=1000,gid=1000"] {
		let (creates, starts) = max_in_flight(Some(mode)).await;
		assert_eq!(creates, 1, "{mode}: creates in flight at once");
		assert!(
			starts > 1,
			"{mode}: starts must stay concurrent, saw {starts}"
		);
	}
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn creates_without_keep_id_stay_concurrent() {
	for mode in [None, Some("auto"), Some("host")] {
		let (creates, starts) = max_in_flight(mode).await;
		assert!(
			creates > 1,
			"{mode:?}: creates must stay concurrent, saw {creates}"
		);
		assert!(
			starts > 1,
			"{mode:?}: starts must stay concurrent, saw {starts}"
		);
	}
}
