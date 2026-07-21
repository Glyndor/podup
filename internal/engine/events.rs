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
		let filters = build_event_filters(&self.project, &opts.filters)?;
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
		// Warned, never fatal. The parser only yields `Err` on a transport
		// failure or an unparseable frame — but "transport failure" turns out to
		// include how libpod ends a finished stream on some versions:
		// `engine_events_stream_connects` went red on the live lane's Podman
		// 5.8.1 when this was treated as a command failure, while the same suite
		// is green on 5.4.2. Until podup can tell that apart from a socket that
		// genuinely died, reporting it would fail commands that worked.
		while let Some(event) = stream.next().await {
			match event {
				Ok(value) => println!("{}", format_event(&value, json)),
				Err(e) => {
					tracing::warn!("events: stream ended early [{}]: {e}", e.stream_end_kind())
				}
			}
		}
		Ok(())
	}
}

/// Build the libpod events `filters` object: always scope to this project's
/// `podup.project` label, then merge each user `KEY=VALUE` predicate (appending
/// to that key's value array). Pure so the merge is unit-tested.
///
/// A predicate with no `=` is an error rather than a skip. Dropping it scoped
/// the stream to the whole project instead — `events --filter garbage` printed
/// everything, which a caller reads as "these all matched". docker compose
/// errors on a malformed filter too.
fn build_event_filters(project: &str, user_filters: &[String]) -> Result<Value> {
	use serde_json::{Map, Value};
	let mut map: Map<String, Value> = Map::new();
	map.insert(
		"label".to_string(),
		Value::Array(vec![Value::String(format!("podup.project={project}"))]),
	);
	for f in user_filters {
		let Some((key, value)) = f.split_once('=') else {
			return Err(ComposeError::Unsupported(format!(
				"malformed events filter {f:?}: expected KEY=VALUE (e.g. event=start)"
			)));
		};
		match map
			.entry(key.to_string())
			.or_insert_with(|| Value::Array(Vec::new()))
		{
			Value::Array(arr) => arr.push(Value::String(value.to_string())),
			other => *other = Value::Array(vec![Value::String(value.to_string())]),
		}
	}
	Ok(Value::Object(map))
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
	format_event_line(typ, action, name, crate::ui::stdout_colored() && !json)
}

/// Join an event's three fields, tinting the two that carry meaning.
///
/// A `--follow` stream is a wall of near-identical lines; `ACTION` is what
/// distinguishes a `start` from a `die`, and `NAME` is which container it
/// happened to. The type (`container`, `network`) repeats on almost every line
/// and is dimmed so it stops competing.
fn format_event_line(typ: &str, action: &str, name: &str, colour: bool) -> String {
	use crate::ui::{identity_style, paint, Style};
	let typ = paint(Style::new().dimmed(), typ, colour);
	let action = match crate::ui::action_or_status_style(action) {
		Some(style) => paint(style, action, colour),
		None => action.to_string(),
	};
	let name = paint(identity_style(name), name, colour && !name.is_empty());
	format!("{typ} {action} {name}").trim().to_string()
}

#[cfg(test)]
mod tests {
	use super::{build_event_filters, format_event};
	use serde_json::json;

	#[test]
	fn build_event_filters_scopes_to_project_label() {
		let f = build_event_filters("demo", &[]).unwrap();
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
			],
		)
		.unwrap();
		assert_eq!(
			f,
			json!({
				"label": ["podup.project=demo"],
				"event": ["start", "die"],
				"type": ["container"],
			})
		);
	}

	/// #1081: a predicate with no `=` used to be dropped, so `events --filter
	/// garbage` silently scoped to the whole project and printed everything — a
	/// caller reads that back as "these all matched".
	#[test]
	fn malformed_filter_is_rejected_not_dropped() {
		let err = build_event_filters("demo", &["bogus".to_string()])
			.expect_err("a filter with no `=` must not be silently ignored");
		assert!(format!("{err}").contains("bogus"), "got {err}");
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

#[cfg(test)]
mod event_colour_tests {
	use super::format_event_line;

	/// Without a colour sink the line is byte-identical to what it always was,
	/// so `--json`, a pipe and the output contract are untouched.
	#[test]
	fn plain_output_is_unchanged() {
		assert_eq!(
			format_event_line("container", "start", "proj-web-1", false),
			"container start proj-web-1"
		);
	}

	/// The two fields that distinguish one line from the next carry colour; the
	/// type, which repeats on nearly every line, is dimmed rather than absent.
	#[test]
	fn action_and_name_are_tinted_apart() {
		let died = format_event_line("container", "die", "proj-web-1", true);
		let started = format_event_line("container", "start", "proj-web-1", true);
		assert_ne!(
			died,
			started.replace("start", "die"),
			"die and start must differ by more than the verb"
		);
	}

	/// An event with no container name must not emit a stray colour reset.
	#[test]
	fn an_empty_name_is_not_painted() {
		let out = format_event_line("network", "create", "", true);
		assert!(
			out.ends_with("create\u{1b}[0m") || !out.ends_with("\u{1b}[0m "),
			"{out:?}"
		);
	}
}
