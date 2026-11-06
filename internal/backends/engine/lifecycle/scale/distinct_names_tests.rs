use crate::engine::fake_podman;
use crate::engine::Engine;

fn engine() -> (fake_podman::FakePodman, Engine) {
	let fake = fake_podman::start(|method, target| {
		if method == "GET" && target.contains("/containers/json") {
			(200, "[]".to_string())
		} else {
			(500, r#"{"message":"nothing may be created"}"#.to_string())
		}
	});
	let engine = Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir());
	(fake, engine)
}

async fn up(yaml: &str) -> (fake_podman::FakePodman, crate::error::Result<()>) {
	let (fake, engine) = engine();
	let file = crate::parse_str(yaml).unwrap();
	let outcome = engine
		.up_with_options(&file, false, &[], &[], false, false, false, false)
		.await;
	(fake, outcome)
}

fn created(fake: &fake_podman::FakePodman) -> bool {
	fake.requests
		.lock()
		.unwrap()
		.iter()
		.any(|r| r.starts_with("POST") || r.starts_with("DELETE"))
}

#[tokio::test]
async fn a_fixed_name_that_a_replica_of_another_service_would_take_is_refused_first() {
	let (fake, outcome) = up(
		"services:\n  web:\n    image: x\n    scale: 2\n  worker:\n    image: y\n    container_name: proj-web-2\n",
	)
	.await;
	let text = outcome.expect_err("two services, one name").to_string();
	assert!(
		text.contains("'web'") && text.contains("'worker'") && text.contains("proj-web-2"),
		"{text}"
	);
	assert!(!created(&fake), "nothing is created before the refusal");
}

#[tokio::test]
async fn two_services_with_the_same_fixed_name_are_refused() {
	let (_fake, outcome) = up(
		"services:\n  a:\n    image: x\n    container_name: same\n  b:\n    image: x\n    container_name: same\n",
	)
	.await;
	let text = outcome.expect_err("two services, one name").to_string();
	assert!(
		text.contains("both resolve to the container name 'same'"),
		"{text}"
	);
}

#[tokio::test]
async fn distinct_names_pass_the_check() {
	let (_fake, engine) = engine();
	let file = crate::parse_str(
		"services:\n  web:\n    image: x\n    scale: 2\n  worker:\n    image: y\n    container_name: other\n",
	)
	.unwrap();
	let enabled = file.services.keys().cloned().collect();
	engine
		.check_distinct_container_names(&file, &enabled)
		.unwrap();
}

#[tokio::test]
async fn a_service_that_is_not_enabled_does_not_count() {
	let (_fake, engine) = engine();
	let file = crate::parse_str(
		"services:\n  web:\n    image: x\n  worker:\n    image: y\n    container_name: proj-web-1\n",
	)
	.unwrap();
	let enabled = ["web".to_string()].into_iter().collect();
	engine
		.check_distinct_container_names(&file, &enabled)
		.unwrap();
}

#[tokio::test]
async fn a_control_character_in_a_name_does_not_reach_the_message() {
	let (_fake, engine) = engine();
	let file = crate::parse_str(
		"services:\n  a:\n    image: x\n    container_name: \"s\\e[31mame\"\n  b:\n    image: x\n    container_name: \"s\\e[31mame\"\n",
	)
	.unwrap();
	let enabled = file.services.keys().cloned().collect();
	let text = engine
		.check_distinct_container_names(&file, &enabled)
		.unwrap_err()
		.to_string();
	assert!(!text.contains('\u{1b}'), "{text:?}");
}
