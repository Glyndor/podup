use super::*;

/// The create call of `web` when its compose says `extra` under it and the file
/// also defines `other` (and `named`, with a `container_name`).
async fn create_body(extra: &str) -> serde_json::Value {
	let fake = fake_routing();
	let engine = engine_for(&fake);
	let file = crate::compose::parse_str(&format!(
		"services:\n  web:\n    image: example/web:1\n{extra}  other:\n    image: example/o:1\n  named:\n    image: example/n:1\n    container_name: fixed\n"
	))
	.expect("compose must parse");
	let service = file.services.get("web").unwrap().clone();
	engine
		.create_and_start(
			"proj-web-1",
			"web",
			&service,
			&file,
			true,
			&std::collections::BTreeMap::new(),
		)
		.await
		.expect("create_and_start must succeed against the fake socket");
	let bodies = fake.bodies.lock().unwrap().clone();
	decode_create_body(&bodies)
}

#[tokio::test]
#[cfg(unix)]
async fn ipc_service_joins_the_first_container_of_that_service() {
	let body = create_body("    ipc: \"service:other\"\n").await;
	assert_eq!(body["ipcns"]["nsmode"], "container", "{body}");
	assert_eq!(body["ipcns"]["value"], "proj-other-1", "{body}");
}

#[tokio::test]
#[cfg(unix)]
async fn pid_service_joins_the_first_container_of_that_service() {
	let body = create_body("    pid: \"service:other\"\n").await;
	assert_eq!(body["pidns"]["nsmode"], "container", "{body}");
	assert_eq!(body["pidns"]["value"], "proj-other-1", "{body}");
}

#[tokio::test]
#[cfg(unix)]
async fn a_container_name_of_the_target_is_honoured() {
	let body = create_body("    pid: \"service:named\"\n").await;
	assert_eq!(body["pidns"]["value"], "fixed", "{body}");
}

#[tokio::test]
#[cfg(unix)]
async fn the_other_modes_are_sent_as_written() {
	let body = create_body("    pid: host\n    ipc: shareable\n").await;
	assert_eq!(body["pidns"]["nsmode"], "host", "{body}");
	assert_eq!(body["ipcns"]["nsmode"], "shareable", "{body}");
}
