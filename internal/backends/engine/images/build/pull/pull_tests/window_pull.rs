//! Warnings are emitted at preflight, independently of pull deduplication.

use crate::compose::types::Service;
use crate::engine::Engine;
use crate::libpod::Client;

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

fn captured(f: impl FnOnce()) -> String {
	let capture = Capture::default();
	let writer = capture.clone();
	let subscriber = tracing_subscriber::fmt()
		.without_time()
		.with_ansi(false)
		.with_writer(move || writer.clone())
		.finish();
	tracing::subscriber::with_default(subscriber, f);
	let bytes = capture.0.lock().unwrap().clone();
	String::from_utf8(bytes).unwrap()
}

fn engine() -> Engine {
	Engine::new(Client::new("/nonexistent.sock"), "p".into())
}

#[test]
fn time_warning_names_the_policy_and_limitation() {
	assert_eq!(crate::engine::build::pull_policy_warning("daily"),
		"pull_policy \"daily\" is treated as \"newer\": podup does not track when an image was last pulled");
}

#[test]
fn time_warning_is_once_per_service() {
	let e = engine();
	let file = crate::parse_str("services:\n  web: {image: shared, pull_policy: daily}\n  worker: {image: shared, pull_policy: weekly}\n").unwrap();
	let output = captured(|| {
		e.pull_preflight(&file).unwrap();
		e.pull_preflight(&file).unwrap();
	});
	assert_eq!(output.matches("does not track").count(), 2);
	assert_eq!(output.matches("service=web").count(), 1);
	assert_eq!(output.matches("service=worker").count(), 1);
	assert_eq!(e.pull_policy_warned.lock().unwrap().len(), 2);
}

#[test]
fn fixed_policy_refresh_interval_has_no_warning() {
	let e = engine();
	let file = crate::parse_str(
		"services:\n  web: {image: shared, pull_policy: missing, pull_refresh_after: 24h}\n",
	)
	.unwrap();
	let output = captured(|| e.pull_preflight(&file).unwrap());
	assert!(!output.contains("does not track"));
	assert!(e.pull_policy_warned.lock().unwrap().is_empty());
}

#[test]
fn registry_autoupdate_still_validates_refresh_interval() {
	let e = engine();
	let service = Service {
		pull_policy: Some("daily".into()),
		pull_refresh_after: Some("bad".into()),
		unknown: [(
			"x-podman-autoupdate".into(),
			serde_yaml::Value::String("registry".into()),
		)]
		.into(),
		..Default::default()
	};
	assert!(e.resolved_pull_policy("web", &service).is_err());
}

#[test]
fn fixed_override_suppresses_time_warning() {
	let e = engine().with_up_overrides(Some("never".into()), false, false);
	let file = crate::parse_str("services:\n  web: {image: shared, pull_policy: daily}\n").unwrap();
	let output = captured(|| e.pull_preflight(&file).unwrap());
	assert!(!output.contains("does not track"), "{output}");
	assert!(e.pull_policy_warned.lock().unwrap().is_empty());
}

#[test]
fn time_override_warning_names_effective_policy() {
	let e = engine().with_up_overrides(Some("weekly".into()), false, false);
	let file = crate::parse_str("services:\n  web: {image: shared, pull_policy: daily}\n").unwrap();
	let output = captured(|| e.pull_preflight(&file).unwrap());
	assert!(
		output.contains("pull_policy \"weekly\" is treated"),
		"{output}"
	);
	assert!(!output.contains("daily"), "{output}");
}

#[test]
fn registry_override_suppresses_time_warning() {
	let e = engine();
	let file = crate::parse_str(
		"services:\n  web: {image: shared, pull_policy: daily, x-podman-autoupdate: registry}\n",
	)
	.unwrap();
	let output = captured(|| e.pull_preflight(&file).unwrap());
	assert!(!output.contains("does not track"), "{output}");
	assert!(e.pull_policy_warned.lock().unwrap().is_empty());
}
