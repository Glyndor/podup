//! Field-by-field assertion that the `SpecGenerator` literal `Engine::create_and_start`
//! builds is sent over the wire to `/containers/create` exactly as the compose
//! fields read.
//!
//! `cargo mutants` once deleted 60 of the literal's fields one at a time and
//! the unit suite stayed green for every one. The live Podman lane was the
//! only thing that noticed a dropped field, and even then only when a scenario
//! happened to exercise the field's effect. This test pins every compose key
//! that maps onto the literal onto the exact JSON value the engine transmits,
//! against the bytes the fake socket receives, so a mutation that drops or
//! renames any one field fails this assertion on the spot.

use super::Engine;
use crate::engine::fake_podman;

/// The fake socket must answer three things the engine calls beyond
/// `/containers/create`: a `GET /containers/json` listing (returns empty so
/// every service is treated as new), a pre-create `DELETE /containers/{name}`
/// (404 is fine, the engine ignores it), and, for any inline
/// `content:`/`file:` secret, the per-name inspect/delete/create dance. The
/// test below uses only `external: true` secrets, so the inspect-then-create
/// branch for owned secrets never runs; we only need to provide a 200 on
/// `/secrets/<name>/json` for each external secret so the engine's
/// `ensure_external_exists` preflight accepts it.
fn fake_routing() -> fake_podman::FakePodman {
	fake_podman::start(|method, target| {
		if method == "GET" && target.contains("/containers/json") {
			(200, "[]".to_string())
		} else if method == "POST" && target.contains("/images/pull") {
			(200, String::new())
		} else if method == "POST" && target.contains("/containers/create") {
			(200, r#"{"Id":"abc","Warnings":[]}"#.to_string())
		} else if method == "POST" && target.contains("/start") {
			(200, String::new())
		} else if method == "DELETE" && target.contains("/containers/") {
			(404, r#"{"message":"no such container"}"#.to_string())
		} else if method == "GET" && target.contains("/secrets/") && target.ends_with("/json") {
			// External secrets exist on the host; respond 200 with a plausible body.
			(200, r#"{"ID":"ext","Spec":{"Labels":{}}}"#.to_string())
		} else {
			(404, r#"{"message":"unexpected request"}"#.to_string())
		}
	})
}

fn engine_for(fake: &fake_podman::FakePodman) -> Engine {
	Engine::with_base_dir(fake.client(), "proj".to_string(), std::env::temp_dir())
}

/// Pull the JSON body of the POST the engine sent to `/containers/create`.
/// `bodies` is parallel to `requests` in the order the requests arrived, so
/// the first non-empty body is the create call (every other call in this
/// scenario, list, pull, delete, start, and secret inspect, carries no body).
fn decode_create_body(bodies: &[Vec<u8>]) -> serde_json::Value {
	let body = bodies
		.iter()
		.find(|b| !b.is_empty())
		.expect("at least one request must carry a body (the create call)");
	serde_json::from_slice(body).expect("create body must be valid JSON")
}

mod one_off;
mod pasta_options;
mod spec_fields;
