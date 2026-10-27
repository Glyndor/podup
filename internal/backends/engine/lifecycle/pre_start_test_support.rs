use crate::compose::types::ComposeFile;
use crate::engine::Engine;
use crate::libpod::client::pre_start_memory::{self, Peer};

/// Control init exit status independently from the existing service state.
pub(super) fn peer(code: i64, running: bool) -> Peer {
	pre_start_memory::start(move |method, path, _| {
		if path.contains("/wait?") {
			(200, code.to_string())
		} else if path.contains("/containers/json") {
			let state = if running { "running" } else { "exited" };
			(
				200,
				format!(r#"[{{"Names":["/proj-web"],"State":"{state}"}}]"#),
			)
		} else if method == "GET" && path.contains("/images/") {
			(200, r#"{"Id":"same-image"}"#.into())
		} else if method == "GET" && path.contains("/containers/") && path.ends_with("/json") {
			// A container the project created carries its label; replacing one
			// without it is refused.
			(
				200,
				r#"{"Config":{"Labels":{"podup.project":"proj"}}}"#.into(),
			)
		} else {
			(200, "{}".into())
		}
	})
}

/// Use the same project prefix as the recorded existing container.
pub(super) fn engine(peer: &Peer) -> Engine {
	Engine::with_base_dir(peer.client(), "proj".into(), std::env::temp_dir())
}

/// Share a service with one observable init step across entry points.
pub(super) fn file() -> ComposeFile {
	crate::parse_str("services:\n  web:\n    image: base\n    pre_start:\n      - command: init\n")
		.unwrap()
}

/// Inspect actual wire specifications in creation order.
pub(super) fn created(peer: &Peer) -> Vec<serde_json::Value> {
	peer.requests
		.lock()
		.unwrap()
		.iter()
		.filter(|(path, _)| path.contains("POST") && path.ends_with("/containers/create"))
		.map(|(_, body)| body.clone())
		.collect()
}
