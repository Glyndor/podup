//! A secret that podup checked is the secret podup removes: a name that is
//! handed to someone else between the check and the removal is left alone.
//!
//! The store below answers like libpod does for a secret: a removal request
//! names the secret by its name or by its ID, and either finds a secret that
//! has it. `swap` runs once, right after the check, and puts an unlabelled
//! secret with a new ID under the same name, the way `podman secret rm` and
//! `podman secret create` run by hand would.

use std::sync::{Arc, Mutex};

use crate::engine::fake_podman;
use crate::engine::secrets::tests_support::{engine_on, file_with_content_secrets};

const NAME: &str = "proj_secret_s1";

#[derive(Clone)]
struct Held {
	id: String,
	name: String,
	project: Option<String>,
}

fn json(s: &Held) -> String {
	let labels = match &s.project {
		Some(p) => format!(r#"{{"podup.project":"{p}"}}"#),
		None => "{}".to_string(),
	};
	format!(
		r#"{{"ID":"{}","Spec":{{"Name":"{}","Labels":{labels}}}}}"#,
		s.id, s.name
	)
}

/// A store holding `old` (ours), that is replaced by `new` (not ours) after
/// the first check of `old`.
struct Store {
	held: Mutex<Vec<Held>>,
	swapped: Mutex<bool>,
}

impl Store {
	fn new() -> Arc<Self> {
		Arc::new(Self {
			held: Mutex::new(vec![Held {
				id: "old".into(),
				name: NAME.into(),
				project: Some("proj".into()),
			}]),
			swapped: Mutex::new(false),
		})
	}

	fn swap(&self) {
		let mut done = self.swapped.lock().unwrap();
		if !*done {
			*done = true;
			*self.held.lock().unwrap() = vec![Held {
				id: "new".into(),
				name: NAME.into(),
				project: None,
			}];
		}
	}

	fn survivor_ids(&self) -> Vec<String> {
		self.held
			.lock()
			.unwrap()
			.iter()
			.map(|s| s.id.clone())
			.collect()
	}

	fn respond(&self, method: &str, target: &str, list_fails: bool) -> (u16, String) {
		let path = target.split('?').next().unwrap_or(target);
		let rest = path.rsplit("/secrets/").next().unwrap_or("");
		match method {
			"GET" if rest == "json" => {
				if list_fails {
					return (500, r#"{"message":"boom"}"#.to_string());
				}
				let body = format!(
					"[{}]",
					self.held
						.lock()
						.unwrap()
						.iter()
						.map(json)
						.collect::<Vec<_>>()
						.join(",")
				);
				self.swap();
				(200, body)
			}
			"GET" => {
				let key = rest.trim_end_matches("/json");
				let found = self
					.held
					.lock()
					.unwrap()
					.iter()
					.find(|s| s.id == key || s.name == key)
					.cloned();
				match found {
					Some(s) => {
						let body = json(&s);
						self.swap();
						(200, body)
					}
					None => (404, r#"{"message":"no such secret"}"#.to_string()),
				}
			}
			"DELETE" => {
				let mut held = self.held.lock().unwrap();
				match held.iter().position(|s| s.id == rest || s.name == rest) {
					Some(at) => {
						held.remove(at);
						(204, String::new())
					}
					None => (404, r#"{"message":"no such secret"}"#.to_string()),
				}
			}
			// A create under a name that is taken is refused, as libpod does.
			_ => {
				if self.held.lock().unwrap().iter().any(|s| s.name == NAME) {
					(500, r#"{"message":"secret name in use"}"#.to_string())
				} else {
					(201, r#"{"ID":"fresh"}"#.to_string())
				}
			}
		}
	}
}

#[tokio::test]
async fn down_leaves_a_secret_that_took_the_name_after_the_list() {
	let store = Store::new();
	let seen = store.clone();
	let fake = fake_podman::start(move |m, t| seen.respond(m, t, false));
	let e = engine_on(&fake);

	e.remove_internal_secrets(&file_with_content_secrets(1))
		.await
		.expect("teardown should succeed");

	assert_eq!(store.survivor_ids(), ["new"], "the replacement was removed");
}

#[tokio::test]
async fn the_per_secret_fallback_leaves_a_secret_that_took_the_name_after_the_inspect() {
	let store = Store::new();
	let seen = store.clone();
	let fake = fake_podman::start(move |m, t| seen.respond(m, t, true));
	let e = engine_on(&fake);

	// The failed list is reported; what matters is what is left in the store.
	let _ = e
		.remove_internal_secrets(&file_with_content_secrets(1))
		.await;

	assert_eq!(store.survivor_ids(), ["new"], "the replacement was removed");
}

#[tokio::test]
async fn up_does_not_remove_a_secret_that_took_the_name_after_the_inspect() {
	let store = Store::new();
	let seen = store.clone();
	let fake = fake_podman::start(move |m, t| seen.respond(m, t, false));
	let e = engine_on(&fake);

	let _ = e
		.create_project_secrets(&file_with_content_secrets(1))
		.await;

	assert_eq!(store.survivor_ids(), ["new"], "the replacement was removed");
}
