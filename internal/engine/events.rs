//! `events` — stream Podman events scoped to the project (`docker compose
//! events`). Filters the libpod event stream by the `podup.project` label.

use futures_util::StreamExt;
use serde_json::Value;

use crate::error::{ComposeError, Result};
use crate::libpod::{urlencoded, API_PREFIX};

use super::Engine;

impl Engine {
	/// Stream events for this project's containers until interrupted. With
	/// `json`, each event is printed as a compact JSON line; otherwise as
	/// `TYPE ACTION NAME`.
	pub async fn stream_events(&self, json: bool) -> Result<()> {
		let filters = serde_json::json!({
			"label": [format!("podup.project={}", self.project)],
		});
		let path = format!(
			"{API_PREFIX}/events?stream=true&filters={}",
			urlencoded(&filters.to_string()),
		);
		let resp = self
			.client
			.get_stream(&path)
			.await
			.map_err(ComposeError::Podman)?;
		let mut stream = crate::libpod::parse_json_lines::<Value>(resp.into_body());
		while let Some(event) = stream.next().await {
			match event {
				Ok(value) => println!("{}", format_event(&value, json)),
				Err(e) => tracing::warn!("events: {e}"),
			}
		}
		Ok(())
	}
}

/// Render one event. `json` emits the raw object as a compact line; otherwise a
/// `TYPE ACTION NAME` summary, tolerant of both the docker-compat shape
/// (`Type`/`Action`/`Actor.Attributes.name`) and the libpod-native one
/// (`status`/`id`).
fn format_event(value: &Value, json: bool) -> String {
	if json {
		return serde_json::to_string(value).unwrap_or_default();
	}
	let typ = value.get("Type").and_then(Value::as_str).unwrap_or("");
	let action = value
		.get("Action")
		.or_else(|| value.get("status"))
		.and_then(Value::as_str)
		.unwrap_or("");
	let name = value
		.pointer("/Actor/Attributes/name")
		.or_else(|| value.get("id"))
		.and_then(Value::as_str)
		.unwrap_or("");
	format!("{typ} {action} {name}").trim().to_string()
}

#[cfg(test)]
mod tests {
	use super::format_event;
	use serde_json::json;

	#[test]
	fn formats_docker_compat_shape() {
		let ev = json!({
			"Type": "container",
			"Action": "start",
			"Actor": { "Attributes": { "name": "web-1" } },
		});
		assert_eq!(format_event(&ev, false), "container start web-1");
	}

	#[test]
	fn formats_libpod_native_shape() {
		let ev = json!({ "Type": "container", "status": "die", "id": "abc123" });
		assert_eq!(format_event(&ev, false), "container die abc123");
	}

	#[test]
	fn json_mode_emits_raw_object() {
		let ev = json!({ "Type": "container", "Action": "start" });
		let out = format_event(&ev, true);
		assert!(out.contains("\"Type\":\"container\""));
		assert!(out.contains("\"Action\":\"start\""));
	}
}
