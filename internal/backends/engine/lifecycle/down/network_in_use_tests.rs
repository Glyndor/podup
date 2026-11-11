use crate::compose::types::ComposeFile;
use crate::engine::fake_podman;
use crate::engine::Engine;

const IN_USE: &str = r#"{"message":"\"proj_default\" has associated containers with it. Use -f to forcibly delete containers and pods: network is being used"}"#;

fn host(message: &'static str) -> fake_podman::FakePodman {
	fake_podman::start(move |method, target| {
		if method == "GET" && target.contains("/networks/json") {
			(200, r#"[{"name":"proj_default"}]"#.to_string())
		} else if method == "GET" && target.contains("/networks/proj_default/json") {
			(
				200,
				r#"{"name":"proj_default","labels":{"podup.project":"proj"}}"#.to_string(),
			)
		} else if method == "DELETE" && target.contains("/networks/proj_default") {
			(500, message.to_string())
		} else if method == "GET" && target.contains("/containers/json") {
			(200, "[]".to_string())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	})
}

fn file() -> ComposeFile {
	crate::parse_str("services:\n  a:\n    image: alpine\n").unwrap()
}

/// A container outside the project holding the network is not a failed `down`:
/// the project's own objects are gone, so the command warns and exits 0.
#[tokio::test]
#[cfg(unix)]
async fn down_succeeds_when_a_foreign_container_still_uses_the_network() {
	let fake = host(IN_USE);
	let e = Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir());
	e.down(&file())
		.await
		.expect("a network held by a foreign container is left in place");
	e.down_by_label(false)
		.await
		.expect("the label-only teardown leaves it in place too");
}

/// Control: any other removal failure still fails `down`.
#[tokio::test]
#[cfg(unix)]
async fn down_still_fails_on_another_network_removal_error() {
	let fake = host(r#"{"message":"permission denied"}"#);
	let e = Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir());
	e.down(&file())
		.await
		.expect_err("an unrelated removal failure is still an error");
}
