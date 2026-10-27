//! Resource ownership and teardown failure regressions over the fake socket.

use crate::compose::types::ComposeFile;
use crate::engine::{fake_podman, Engine};

fn engine(fake: &fake_podman::FakePodman) -> Engine {
	Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir())
}

fn empty_lists(method: &str, target: &str) -> (u16, String) {
	if method == "GET" && (target.contains("/json?") || target.ends_with("/json")) {
		(200, "[]".into())
	} else {
		(200, "{}".into())
	}
}

fn assert_no_delete(fake: &fake_podman::FakePodman) {
	let seen = fake.requests.lock().unwrap();
	assert!(!seen.iter().any(|r| r.starts_with("DELETE ")), "{seen:?}");
}

mod cleanup;
mod containers;
mod networks;
mod pods;
mod volumes;
