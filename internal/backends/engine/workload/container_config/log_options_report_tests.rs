//! What `logging:` says when it drops or cannot honour an option. The
//! translation itself is covered in `log_config_tests.rs`; this reads the
//! warnings, which is the only place a dropped key is visible to the user.

use super::*;
use crate::compose::types::LoggingConfig;

#[derive(Clone, Default)]
struct Capture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Capture {
	fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
		self.0.lock().unwrap().extend_from_slice(bytes);
		Ok(bytes.len())
	}
	fn flush(&mut self) -> std::io::Result<()> {
		Ok(())
	}
}

/// The warnings `build_log_config` gives for a `logging:` block.
fn warnings(driver: Option<&str>, options: &[(&str, &str)]) -> String {
	let logging = LoggingConfig {
		driver: driver.map(str::to_string),
		options: options
			.iter()
			.map(|(k, v)| ((*k).to_string(), (*v).to_string()))
			.collect(),
	};
	let capture = Capture::default();
	let writer = capture.clone();
	let subscriber = tracing_subscriber::fmt()
		.without_time()
		.with_ansi(false)
		.with_writer(move || writer.clone())
		.finish();
	tracing::subscriber::with_default(subscriber, || {
		build_log_config("web", Some(&logging)).unwrap();
	});
	let bytes = capture.0.lock().unwrap().clone();
	String::from_utf8(bytes).unwrap()
}

#[test]
fn a_path_under_journald_is_reported_as_ignored() {
	let text = warnings(Some("journald"), &[("path", "/var/log/a.log")]);
	assert!(
		text.contains("logging.options.path is ignored") && text.contains("'journald'"),
		"{text}"
	);
}

#[test]
fn a_path_under_a_file_driver_says_nothing() {
	for driver in [Some("k8s-file"), Some("json-file"), None] {
		let text = warnings(driver, &[("path", "/var/log/a.log")]);
		assert!(text.is_empty(), "{driver:?}: {text}");
	}
}

#[test]
fn an_option_the_generator_never_reads_is_reported_by_name() {
	let text = warnings(Some("journald"), &[("labels", "app"), ("tag", "t")]);
	assert!(text.contains("logging.options.labels is ignored"), "{text}");
	assert!(!text.contains("logging.options.tag"), "{text}");
}

#[test]
fn the_options_the_generator_reads_say_nothing() {
	let text = warnings(Some("k8s-file"), &[("max-size", "10m"), ("path", "/a")]);
	assert!(text.is_empty(), "{text}");
}
