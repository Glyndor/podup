//! What `--no-warn` silences on the engine path: the advisory warnings, and not
//! the security ones.

use std::sync::{Arc, Mutex};

use crate::engine::Engine;

struct Sink(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Sink {
	fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
		self.0.lock().unwrap().extend_from_slice(bytes);
		Ok(bytes.len())
	}
	fn flush(&mut self) -> std::io::Result<()> {
		Ok(())
	}
}

/// Run `up` for a privileged service and return the log it wrote.
async fn log_of_up(no_warn: bool) -> String {
	let fake = crate::engine::fake_podman::start(|method, target| {
		if method == "GET" && target.contains("/containers/json") {
			(200, "[]".to_string())
		} else if method == "POST" && target.contains("/images/pull") {
			(200, String::new())
		} else if method == "POST" && target.contains("/containers/create") {
			(200, "{}".to_string())
		} else if method == "POST" && target.contains("/start") {
			(200, String::new())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let mut engine = Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir());
	engine.no_warn = no_warn;
	let file =
		crate::parse_str("services:\n  web:\n    image: img\n    privileged: true\n").unwrap();

	let bytes = Arc::new(Mutex::new(Vec::new()));
	let writer = bytes.clone();
	let subscriber = tracing_subscriber::fmt()
		.without_time()
		.with_ansi(false)
		.with_writer(move || Sink(writer.clone()))
		.finish();
	let _guard = tracing::subscriber::set_default(subscriber);
	engine
		.up_with_options(&file, false, &[], &[], false, false, false, false)
		.await
		.expect("up succeeds");
	let text = bytes.lock().unwrap().clone();
	String::from_utf8(text).unwrap()
}

const SECURITY: &str = "grants every Linux capability";
const ADVISORY: &str = "reduced effect under rootless Podman";

#[tokio::test]
async fn without_no_warn_both_the_security_and_the_advisory_warning_show() {
	let log = log_of_up(false).await;
	assert!(log.contains(SECURITY), "{log}");
	assert!(log.contains(ADVISORY), "{log}");
}

/// A privileged container is a security warning: it is printed on every run
/// whatever `--no-warn` says. The note that rootless Podman reduces its effect
/// is advice, and `--no-warn` silences it.
#[tokio::test]
async fn no_warn_keeps_the_security_warning_and_drops_the_advisory_one() {
	let log = log_of_up(true).await;
	assert!(
		log.contains(SECURITY),
		"the security warning is gone: {log}"
	);
	assert!(
		!log.contains(ADVISORY),
		"the advisory warning stayed: {log}"
	);
}
