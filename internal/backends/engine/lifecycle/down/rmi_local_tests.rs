//! `down --rmi local` removes only the images the project names itself.
//!
//! Docker Compose documents `local` as "only images that don't have a custom
//! tag set by the image field". A service that builds and also sets `image:`
//! has chosen a tag of its own, so `local` leaves it; only a service that builds
//! and sets no `image:` has the generated name `local` is about.

use crate::engine::fake_podman;
use crate::engine::Engine;

const FILE: &str = "\
services:
  generated:
    build: .
  tagged:
    build: .
    image: app:1.4
  pulled:
    image: nginx:1
";

fn engine() -> (fake_podman::FakePodman, Engine) {
	let fake = fake_podman::start(|method, _target| {
		if method == "DELETE" {
			(200, String::new())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let engine = Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir());
	(fake, engine)
}

fn image_deletes(fake: &fake_podman::FakePodman) -> Vec<String> {
	fake.requests
		.lock()
		.unwrap()
		.iter()
		.filter(|r| r.starts_with("DELETE ") && r.contains("/images/"))
		.cloned()
		.collect()
}

#[tokio::test]
async fn local_removes_the_generated_image_of_a_service_that_builds() {
	let (fake, engine) = engine();
	let file = crate::parse_str(FILE).unwrap();
	engine.remove_service_images(&file, true).await.unwrap();
	let removed = image_deletes(&fake);
	assert!(
		removed.iter().any(|r| r.contains("proj-generated")),
		"{removed:?}"
	);
}

#[tokio::test]
async fn local_keeps_the_image_a_service_that_builds_tagged_itself() {
	let (fake, engine) = engine();
	let file = crate::parse_str(FILE).unwrap();
	engine.remove_service_images(&file, true).await.unwrap();
	let removed = image_deletes(&fake);
	assert!(
		!removed
			.iter()
			.any(|r| r.contains("app:1.4") || r.contains("app%3A1.4")),
		"a custom tag is not `local`: {removed:?}"
	);
}

#[tokio::test]
async fn local_keeps_an_image_the_project_only_pulls() {
	let (fake, engine) = engine();
	let file = crate::parse_str(FILE).unwrap();
	engine.remove_service_images(&file, true).await.unwrap();
	let removed = image_deletes(&fake);
	assert!(!removed.iter().any(|r| r.contains("nginx")), "{removed:?}");
}

#[tokio::test]
async fn all_still_removes_every_image() {
	let (fake, engine) = engine();
	let file = crate::parse_str(FILE).unwrap();
	engine.remove_service_images(&file, false).await.unwrap();
	let removed = image_deletes(&fake);
	assert!(
		removed.iter().any(|r| r.contains("proj-generated")),
		"{removed:?}"
	);
	assert!(
		removed
			.iter()
			.any(|r| r.contains("app:1.4") || r.contains("app%3A1.4")),
		"{removed:?}"
	);
	assert!(removed.iter().any(|r| r.contains("nginx")), "{removed:?}");
}
