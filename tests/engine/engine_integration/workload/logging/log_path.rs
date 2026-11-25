//! `logging.options.path` puts the container's log where the file asks.
//!
//! libpod reads the path from `log_configuration.path` and ignores a `path`
//! key inside `options`, so a path sent the second way left the log in
//! Podman's own directory and nothing said so.

use super::*;

async fn log_has(path: &std::path::Path, needle: &str, secs: u64) -> bool {
	let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(secs);
	while tokio::time::Instant::now() < deadline {
		if std::fs::read_to_string(path).is_ok_and(|text| text.contains(needle)) {
			return true;
		}
		tokio::time::sleep(std::time::Duration::from_millis(100)).await;
	}
	false
}

#[cfg(feature = "test-helpers")]
#[tokio::test]
async fn the_log_path_of_a_service_is_where_the_log_is_written() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("logpath");
	let engine = Engine::new(client, proj.clone());
	let dir = tempfile::tempdir().unwrap();
	let log = dir.path().join("web.log");
	let file = parse_str(&format!(
		"services:\n  web:\n    image: busybox:latest\n    command: [\"sh\", \"-c\", \"echo podup-log-path; sleep 300\"]\n    logging:\n      driver: k8s-file\n      options:\n        path: {}\n",
		log.display()
	))
	.unwrap();

	engine.up(&file).await.unwrap();
	let written = log_has(&log, "podup-log-path", 20).await;
	engine.down(&file).await.unwrap();

	assert!(
		written,
		"the log was not written to {}: the path did not reach libpod as a typed field",
		log.display()
	);
}

/// A path with no driver picks the file driver, so the file appears on a host
/// whose default driver is journald.
#[cfg(feature = "test-helpers")]
#[tokio::test]
async fn a_log_path_without_a_driver_still_writes_the_file() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("logpathdef");
	let engine = Engine::new(client, proj.clone());
	let dir = tempfile::tempdir().unwrap();
	let log = dir.path().join("web.log");
	let file = parse_str(&format!(
		"services:\n  web:\n    image: busybox:latest\n    command: [\"sh\", \"-c\", \"echo podup-log-default; sleep 300\"]\n    logging:\n      options:\n        path: {}\n",
		log.display()
	))
	.unwrap();

	engine.up(&file).await.unwrap();
	let written = log_has(&log, "podup-log-default", 20).await;
	engine.down(&file).await.unwrap();

	assert!(written, "the log was not written to {}", log.display());
}
