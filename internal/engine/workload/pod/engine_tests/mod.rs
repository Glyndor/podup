//! Unit tests for the `x-podman-pod` engine wiring against the fake Podman
//! socket. Each test stands up a project with the extension set, runs `up`
//! or `down`, and asserts on the wire requests / responses the engine
//! produced. The shared fake-socket helper is below; each test that needs a
//! richer routing rule builds its own.

use crate::compose::parse_str;
use crate::engine::fake_podman;
use crate::engine::Engine;

/// Fake socket answering the requests an `up`/`down` pass makes for a
/// pod-enabled project. Every request gets a 200 with the smallest body the
/// daemon would return; the test then inspects `fake.requests` and
/// `fake.bodies` to assert what the engine actually did.
pub(super) fn pod_up_fake() -> fake_podman::FakePodman {
	fake_podman::start(|method, target| route(method, target, None))
}

/// As [`pod_up_fake`] but reports the pod as already existing with a given
/// hash label. The engine will inspect it, compare against the new hash,
/// and either reuse or recreate.
pub(super) fn pod_up_fake_with_existing(existing_hash: Option<&str>) -> fake_podman::FakePodman {
	let hash_label = existing_hash.unwrap_or("wrong-hash").to_string();
	fake_podman::start(move |method, target| route(method, target, Some(&hash_label)))
}

/// One fake routing table, parameterised by the recorded `podup.pod-config-hash`.
/// `existing_hash == None` means the pod does not yet exist (the engine
/// creates it); `Some(hash)` means the pod exists with that recorded hash.
pub(super) fn route(method: &str, target: &str, existing_hash: Option<&str>) -> (u16, String) {
	if method == "GET" && target.contains("/containers/json") {
		(200, "[]".to_string())
	} else if method == "POST"
		&& (target.contains("/networks/create") || target.contains("/volumes/create"))
	{
		(200, r#"{}"#.to_string())
	} else if method == "POST" && target.contains("/secrets/create") {
		(200, r#"{"ID":"secret-id"}"#.to_string())
	} else if method == "POST" && target.contains("/containers/create") {
		(200, r#"{"Id":"abc","Warnings":[]}"#.to_string())
	} else if method == "POST" && target.contains("/containers/") && target.contains("/wait") {
		(200, r#"{"StatusCode":0}"#.to_string())
	} else if method == "POST" && (target.contains("/start") || target.contains("/images/pull")) {
		(200, String::new())
	} else if method == "GET" && target.contains("/images/") && target.contains("/json") {
		(
			200,
			r#"{"Id":"sha256:0000000000000000000000000000000000000000000000000000000000000000"}"#
				.to_string(),
		)
	} else if method == "DELETE" && target.contains("/containers/") {
		(404, r#"{"message":"no such container"}"#.to_string())
	} else if method == "POST" && target.contains("/pods/create") {
		(200, r#"{"Id":"pod-id"}"#.to_string())
	} else if method == "GET" && target.contains("/pods/") && target.contains("/json") {
		if existing_hash.is_none() {
			// Pod does not exist.
			(404, r#"{"message":"no such pod"}"#.to_string())
		} else {
			let hash_label = existing_hash.unwrap_or("wrong-hash");
			(
				200,
				format!(
					r#"{{"Name":"proj","Labels":{{"podup.pod-config-hash":"{hash_label}"}},"NumContainers":2}}"#
				),
			)
		}
	} else if method == "GET" && target.contains("/pods/") && target.contains("/exists") {
		let hash_label = existing_hash.unwrap_or("wrong-hash");
		(
			200,
			format!(
				r#"{{"Name":"proj","Labels":{{"podup.pod-config-hash":"{hash_label}"}},"NumContainers":2}}"#
			),
		)
	} else if method == "DELETE" && target.contains("/pods/") {
		(200, String::new())
	} else if method == "GET" && target.contains("/networks/json") {
		(200, "[]".to_string())
	} else if method == "GET" && target.contains("/secrets/") && target.contains("/json") {
		// External-secret inspect: pretend every external secret exists.
		(200, r#"{"ID":"ext","Spec":{"Labels":{}}}"#.to_string())
	} else if method == "DELETE" && target.contains("/secrets/") {
		(200, String::new())
	} else {
		(404, r#"{"message":"unexpected"}"#.to_string())
	}
}

/// Decode the JSON body of one of the requests the engine sent. The
/// `SpecGenerator` is the largest payload and is the one most tests care
/// about; `PodSpecGenerator` is the second.
fn decode_body(bodies: &[Vec<u8>], needle: &str) -> serde_json::Value {
	for (i, b) in bodies.iter().enumerate() {
		if b.is_empty() {
			continue;
		}
		if let Ok(v) = serde_json::from_slice::<serde_json::Value>(b) {
			// Cheap discriminator: every other body in these tests is an
			// empty JSON object/array. The `pod` and `portmappings` fields
			// only appear on the bodies we care about.
			if needle == "pod" && v.get("pod").is_some() {
				let _ = i;
				return v;
			}
			if needle == "pods" && v.get("shared_namespaces").is_some() {
				return v;
			}
		}
	}
	panic!("no body matching {needle} in {} bodies", bodies.len())
}

pub(super) fn engine_for(fake: &fake_podman::FakePodman, project: &str) -> Engine {
	Engine::with_base_dir(fake.client(), project.to_string(), std::env::temp_dir())
}

mod down;
mod pod_create_body;
mod pod_hash;
mod ps;
