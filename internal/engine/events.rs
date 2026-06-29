//! `events` — stream Podman events scoped to the project (`docker compose
//! events`). Filters the libpod event stream by the `podup.project` label.

use futures_util::StreamExt;
use serde_json::Value;

use crate::error::{ComposeError, Result};
use crate::libpod::{urlencoded, API_PREFIX};

use super::Engine;

/// Options for [`Engine::stream_events`], mirroring `docker compose events`
/// (`--since`, `--until`, `--filter`).
#[derive(Debug, Clone, Default)]
pub struct EventsOptions {
	/// Only events at or after this timestamp/relative time (`--since`).
	pub since: Option<String>,
	/// Only events up to this timestamp/relative time (`--until`).
	pub until: Option<String>,
	/// Extra `KEY=VALUE` event filters (`--filter`, e.g. `event=start`).
	pub filters: Vec<String>,
}

impl Engine {
	/// Stream events for this project's containers until interrupted. With
	/// `json`, each event is printed as a compact JSON line; otherwise as
	/// `TYPE ACTION NAME`.
	pub async fn stream_events(&self, json: bool) -> Result<()> {
		self.stream_events_with_options(json, &EventsOptions::default())
			.await
	}

	/// [`Engine::stream_events`] with `docker compose events`-style `--since`,
	/// `--until`, and `--filter` options.
	pub async fn stream_events_with_options(&self, json: bool, opts: &EventsOptions) -> Result<()> {
		let filters = build_event_filters(&self.project, &opts.filters);
		let mut path = format!(
			"{API_PREFIX}/events?stream=true&filters={}",
			urlencoded(&filters.to_string()),
		);
		if let Some(since) = &opts.since {
			path.push_str(&format!("&since={}", urlencoded(since)));
		}
		if let Some(until) = &opts.until {
			path.push_str(&format!("&until={}", urlencoded(until)));
		}
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

/// Build the libpod events `filters` object: always scope to this project's
/// `podup.project` label, then merge each user `KEY=VALUE` predicate (appending
/// to that key's value array). A predicate with no `=` is skipped. Pure so the
/// merge is unit-tested.
fn build_event_filters(project: &str, user_filters: &[String]) -> Value {
	use serde_json::{Map, Value};
	let mut map: Map<String, Value> = Map::new();
	map.insert(
		"label".to_string(),
		Value::Array(vec![Value::String(format!("podup.project={project}"))]),
	);
	for f in user_filters {
		let Some((key, value)) = f.split_once('=') else {
			tracing::warn!("events: ignoring malformed filter '{f}' (expected KEY=VALUE)");
			continue;
		};
		match map
			.entry(key.to_string())
			.or_insert_with(|| Value::Array(Vec::new()))
		{
			Value::Array(arr) => arr.push(Value::String(value.to_string())),
			other => *other = Value::Array(vec![Value::String(value.to_string())]),
		}
	}
	Value::Object(map)
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
	use super::{build_event_filters, format_event};
	use serde_json::json;

	#[test]
	fn build_event_filters_scopes_to_project_label() {
		let f = build_event_filters("demo", &[]);
		assert_eq!(f, json!({ "label": ["podup.project=demo"] }));
	}

	#[test]
	fn build_event_filters_merges_user_predicates() {
		let f = build_event_filters(
			"demo",
			&[
				"event=start".to_string(),
				"event=die".to_string(),
				"type=container".to_string(),
				"bogus".to_string(),
			],
		);
		assert_eq!(
			f,
			json!({
				"label": ["podup.project=demo"],
				"event": ["start", "die"],
				"type": ["container"],
			})
		);
	}

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
