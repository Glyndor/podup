//! The anonymous volumes of the container a recreate replaces.
//!
//! A volume declared without a source (`volumes: [/data]`) and a path the
//! image declares a `VOLUME` directive for both end up as an anonymous
//! volume the runtime names. The replacement container inherits them by
//! destination, so the data they hold survives the recreate.

use crate::engine::Engine;

/// Drive a `create_and_start` against a fake daemon and return the `volumes`
/// array of the spec it POSTed to `/containers/create`.
///
/// `mounts` is what the container being replaced has, as the runtime reports
/// it; `image_volumes` is the set of paths the image declares a `VOLUME`
/// directive for.
#[cfg(unix)]
async fn created_volumes(
	mounts: &str,
	image_volumes: &str,
	compose: &str,
	renew: bool,
) -> Vec<serde_json::Value> {
	use crate::engine::fake_podman;

	// The container being replaced was created by this project, so it carries
	// the project label; a container without it is refused, not replaced.
	let inspect =
		format!(r#"{{"Mounts":{mounts},"Config":{{"Labels":{{"podup.project":"proj"}}}}}}"#);
	let image = format!(r#"{{"Id":"sha256:abc","Config":{{"Volumes":{image_volumes}}}}}"#);
	let fake = fake_podman::start(move |method, target| {
		if method == "GET" && target.contains("/containers/proj-web-1/json") {
			(200, inspect.clone())
		} else if method == "GET" && target.contains("/images/") && target.ends_with("/json") {
			(200, image.clone())
		} else if method == "GET" && target.contains("/containers/json") {
			(200, "[]".to_string())
		} else if method == "POST" && target.contains("/containers/create") {
			(200, r#"{"Id":"abc","Warnings":[]}"#.to_string())
		} else if (method == "POST" && target.contains("/start"))
			|| (method == "DELETE" && target.contains("/containers/"))
		{
			(200, String::new())
		} else {
			(404, r#"{"message":"unexpected request"}"#.to_string())
		}
	});
	let engine = Engine::with_base_dir(fake.client(), "proj".to_string(), std::env::temp_dir());
	let engine = if renew {
		engine.with_renew_anon_volumes(true)
	} else {
		engine
	};
	let file = crate::parse_str(compose).expect("the compose file must parse");
	let service = file
		.services
		.get("web")
		.expect("the compose file must declare a `web` service")
		.clone();

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
	let body = bodies
		.iter()
		.find(|b| !b.is_empty())
		.expect("the create call must carry a body");
	let body: serde_json::Value = serde_json::from_slice(body).expect("the body must be JSON");
	// The spec omits the whole array when it carries no volume, so an absent
	// `volumes` key means "no volume at all".
	body["volumes"].as_array().cloned().unwrap_or_default()
}

/// The volume a compose `volumes:` entry declares without a source must be
/// the one the previous container had at the same destination.
///
/// Without this the replacement's spec went out with an empty name, the
/// runtime minted a fresh volume, and the data the previous container had
/// written stayed behind in a volume nothing pointed at any more.
#[cfg(unix)]
#[tokio::test]
async fn a_recreate_inherits_the_anonymous_volume_of_the_container_it_replaces() {
	let volumes = created_volumes(
		r#"[{"Type":"volume","Name":"anonvol123","Destination":"/data","RW":true}]"#,
		"{}",
		"services:\n  web:\n    image: img\n    volumes:\n      - /data\n",
		false,
	)
	.await;
	let entry = volumes
		.iter()
		.find(|v| v["Dest"] == "/data")
		.expect("the /data volume must appear under volumes");
	assert_eq!(
		entry["Name"], "anonvol123",
		"the replacement must mount the volume the previous container had: {volumes:?}"
	);
}

/// A path the image declares a `VOLUME` directive for is mounted by the
/// runtime even when the compose file says nothing about it, so the
/// replacement inherits the volume the previous container had there too.
#[cfg(unix)]
#[tokio::test]
async fn a_recreate_inherits_the_volume_the_image_declares() {
	let volumes = created_volumes(
		r#"[{"Type":"volume","Name":"anonimg456","Destination":"/var/lib/data","RW":true}]"#,
		r#"{"/var/lib/data":{}}"#,
		"services:\n  web:\n    image: img\n",
		false,
	)
	.await;
	let entry = volumes
		.iter()
		.find(|v| v["Dest"] == "/var/lib/data")
		.expect("the image volume must appear under volumes");
	assert_eq!(
		entry["Name"], "anonimg456",
		"the replacement must mount the volume the image's VOLUME directive produced: {volumes:?}"
	);
}

/// A mount the compose file dropped is not brought back: the image says
/// nothing about that path, so the previous volume was the compose file's
/// own and the file no longer asks for it.
#[cfg(unix)]
#[tokio::test]
async fn a_dropped_mount_is_not_brought_back() {
	let volumes = created_volumes(
		r#"[{"Type":"volume","Name":"anonvol123","Destination":"/data","RW":true}]"#,
		"{}",
		"services:\n  web:\n    image: img\n",
		false,
	)
	.await;
	assert!(
		!volumes.iter().any(|v| v["Dest"] == "/data"),
		"a mount the compose file dropped must not come back: {volumes:?}"
	);
}

/// `-V/--renew-anon-volumes` asks for fresh volumes, so nothing is
/// inherited: that is the whole point of the flag.
#[cfg(unix)]
#[tokio::test]
async fn renew_anon_volumes_inherits_nothing() {
	let volumes = created_volumes(
		r#"[{"Type":"volume","Name":"anonvol123","Destination":"/data","RW":true}]"#,
		"{}",
		"services:\n  web:\n    image: img\n    volumes:\n      - /data\n",
		true,
	)
	.await;
	let entry = volumes
		.iter()
		.find(|v| v["Dest"] == "/data")
		.expect("the /data volume must appear under volumes");
	assert_eq!(
		entry["Name"], "",
		"`--renew-anon-volumes` must ask for a fresh volume: {volumes:?}"
	);
}

/// A container that does not exist yet has no volume to inherit, and the
/// create must go out with the plain anonymous entry.
#[cfg(unix)]
#[tokio::test]
async fn a_new_container_inherits_nothing() {
	let volumes = created_volumes(
		"[]",
		"{}",
		"services:\n  web:\n    image: img\n    volumes:\n      - /data\n",
		false,
	)
	.await;
	let entry = volumes
		.iter()
		.find(|v| v["Dest"] == "/data")
		.expect("the /data volume must appear under volumes");
	assert_eq!(
		entry["Name"], "",
		"a brand new container must ask for a fresh anonymous volume: {volumes:?}"
	);
}
