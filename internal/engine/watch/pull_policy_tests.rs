//! Watch must reject invalid pull intent before watcher or rebuild work.

use crate::engine::Engine;
use crate::libpod::Client;

fn engine() -> Engine {
	Engine::new(Client::new("/nonexistent.sock"), "p".into())
}

fn invalid(field: &str) -> crate::compose::types::ComposeFile {
	crate::parse_str(&format!(
		"services:\n  web: {{image: shared, {field}: nonsense}}\n"
	))
	.unwrap()
}

fn rejected(result: crate::Result<()>, field: &str) {
	let error = result.unwrap_err().to_string();
	assert!(
		error.contains(field) && error.contains("nonsense"),
		"{error}"
	);
}

#[tokio::test]
async fn watch_rejects_pull_fields_before_watcher_setup() {
	for field in ["pull_policy", "pull_refresh_after"] {
		rejected(engine().watch(&invalid(field)).await, field);
	}
}

#[tokio::test]
async fn watch_rebuild_rejects_pull_fields_before_replacement() {
	for field in ["pull_policy", "pull_refresh_after"] {
		rejected(
			engine().watch_rebuild(&invalid(field), "absent").await,
			field,
		);
	}
}
