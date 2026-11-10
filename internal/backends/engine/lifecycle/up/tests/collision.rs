//! A container that holds the name of a replica but is not one is not that
//! replica: `scale` and `up` must not report it as present, start it as one, or
//! replace it.

use crate::engine::fake_podman;
use crate::engine::lifecycle::Engine;
use crate::error::ComposeError;

fn engine_with(client: crate::libpod::Client) -> Engine {
	Engine::with_base_dir(client, "proj".into(), std::env::temp_dir())
}

const WEB_AND_WORKER: &str = r#"[
	{"Names":["/proj-web-1"],"Labels":{"podup.service":"web"}},
	{"Names":["/proj-web-2"],"Labels":{"podup.service":"worker"}}]"#;

fn fake(containers: &'static str) -> fake_podman::FakePodman {
	fake_podman::start(move |method, target| {
		if method == "GET" && target.contains("/containers/json") {
			(200, containers.to_string())
		} else if method == "POST" && target.contains("/images/pull") {
			(200, String::new())
		} else if method == "POST" && (target.contains("/create") || target.contains("/start")) {
			(200, "{}".to_string())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	})
}

fn touched(fake: &fake_podman::FakePodman, name: &str) -> bool {
	fake.requests
		.lock()
		.unwrap()
		.iter()
		.any(|r| r.contains(name) && !r.starts_with("GET"))
}

#[tokio::test]
async fn a_name_held_by_another_service_is_not_the_missing_replica() {
	let fake = fake(WEB_AND_WORKER);
	let e = engine_with(fake.client());
	let file = crate::parse_str(
		"services:\n  web:\n    image: img\n    scale: 2\n  worker:\n    image: other\n",
	)
	.unwrap();
	// `scale` goes through `no_recreate`.
	let err = e
		.up_with_options(
			&file,
			false,
			&[],
			&["web".to_string()],
			true,
			false,
			false,
			false,
		)
		.await
		.expect_err("proj-web-2 belongs to worker");
	assert!(matches!(err, ComposeError::Unsupported(_)), "{err:?}");
	let text = err.to_string();
	assert!(
		text.contains("proj-web-2") && text.contains("'worker'"),
		"{text}"
	);
	assert!(
		!touched(&fake, "proj-web-2"),
		"worker's container was touched"
	);
}

#[tokio::test]
async fn nor_is_it_replaced_by_a_plain_up_of_the_service() {
	let fake = fake(WEB_AND_WORKER);
	let e = engine_with(fake.client());
	let file = crate::parse_str(
		"services:\n  web:\n    image: img\n    scale: 2\n  worker:\n    image: other\n",
	)
	.unwrap();
	let err = e
		.up_with_options(
			&file,
			false,
			&[],
			&["web".to_string()],
			false,
			false,
			false,
			false,
		)
		.await
		.expect_err("proj-web-2 belongs to worker");
	assert!(err.to_string().contains("'worker'"), "{err}");
	assert!(
		!touched(&fake, "proj-web-2"),
		"worker's container was touched"
	);
}

#[tokio::test]
async fn a_one_off_run_container_is_not_a_replica_either() {
	let fake = fake(
		r#"[{"Names":["/proj-web-1"],"Labels":{"podup.service":"web"}},
		{"Names":["/proj-web-2"],"Labels":{"podup.service":"web","podup.oneoff":"true"}}]"#,
	);
	let e = engine_with(fake.client());
	let file = crate::parse_str("services:\n  web:\n    image: img\n    scale: 2\n").unwrap();
	let err = e
		.up_with_options(
			&file,
			false,
			&[],
			&["web".to_string()],
			true,
			false,
			false,
			false,
		)
		.await
		.expect_err("proj-web-2 is a run container");
	assert!(err.to_string().contains("one-off"), "{err}");
}

#[tokio::test]
async fn a_replica_of_the_same_service_is_still_reused() {
	let fake = fake(
		r#"[{"Names":["/proj-web-1"],"Labels":{"podup.service":"web"}},
		{"Names":["/proj-web-2"],"Labels":{"podup.service":"web"}}]"#,
	);
	let e = engine_with(fake.client());
	let file = crate::parse_str("services:\n  web:\n    image: img\n    scale: 2\n").unwrap();
	e.up_with_options(
		&file,
		false,
		&[],
		&["web".to_string()],
		true,
		false,
		false,
		false,
	)
	.await
	.expect("both replicas are web's");
}

#[tokio::test]
async fn a_container_of_a_service_the_file_no_longer_defines_is_replaced_as_before() {
	// `web` was renamed `frontend` and kept its container_name: the old container
	// is an orphan holding the name, and there is no other service to rename.
	let fake = fake(r#"[{"Names":["/app"],"Labels":{"podup.service":"web"}}]"#);
	let e = engine_with(fake.client());
	let file =
		crate::parse_str("services:\n  frontend:\n    image: img\n    container_name: app\n")
			.unwrap();
	e.up_with_options(
		&file,
		false,
		&[],
		&["frontend".to_string()],
		false,
		false,
		false,
		false,
	)
	.await
	.expect("an orphan does not block the name");
	assert!(touched(&fake, "app"), "the orphan is replaced");
}

#[tokio::test]
async fn the_advice_fits_who_holds_the_name() {
	let one_off = fake(
		r#"[{"Names":["/proj-web-1"],"Labels":{"podup.service":"web"}},
		{"Names":["/proj-web-2"],"Labels":{"podup.service":"web","podup.oneoff":"true"}}]"#,
	);
	let file = crate::parse_str("services:\n  web:\n    image: img\n    scale: 2\n").unwrap();
	let text = engine_with(one_off.client())
		.up_with_options(
			&file,
			false,
			&[],
			&["web".to_string()],
			true,
			false,
			false,
			false,
		)
		.await
		.unwrap_err()
		.to_string();
	assert!(
		text.contains("podman rm") && !text.contains("rename"),
		"{text}"
	);

	let other = fake(WEB_AND_WORKER);
	let file = crate::parse_str(
		"services:\n  web:\n    image: img\n    scale: 2\n  worker:\n    image: other\n",
	)
	.unwrap();
	let text = engine_with(other.client())
		.up_with_options(
			&file,
			false,
			&[],
			&["web".to_string()],
			true,
			false,
			false,
			false,
		)
		.await
		.unwrap_err()
		.to_string();
	assert!(
		text.contains("service 'worker'") && text.contains("container_name"),
		"{text}"
	);
}
