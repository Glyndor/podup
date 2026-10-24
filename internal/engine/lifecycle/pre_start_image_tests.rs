use super::pre_start_test_support::{created, engine, peer};

#[tokio::test]
async fn pre_start_reuses_the_service_build_image() {
	for build in ["'.'", "{context: '.', tags: [custom:latest]}"] {
		let peer = peer(0, false);
		let mut engine = engine(&peer);
		engine.no_build = true;
		let file = crate::parse_str(&format!(
			"services:\n  web:\n    build: {build}\n    pre_start:\n      - command: init\n      - image: separate:latest\n      - build: {{context: other, tags: [init:latest]}}\n"
		))
		.unwrap();
		let steps = engine.pre_start_steps("web", &file.services["web"]);
		assert!(steps[0].build.is_none());
		assert_eq!(steps[1].image.as_deref(), Some("separate:latest"));
		assert!(steps[2].build.is_some());
		engine
			.run_pre_start("proj-web", "web", &file.services["web"], &file)
			.await
			.unwrap();
		assert!(!peer.requests.lock().unwrap().iter().any(|(path, _)| {
			path.contains("/images/") && (path.contains("proj-web") || path.contains("custom"))
		}));
		let specs = created(&peer);
		assert_eq!(specs.len(), 3);
		assert_eq!(
			specs[0]["image"],
			if build.contains("tags") {
				"custom:latest"
			} else {
				"proj-web:latest"
			}
		);
	}
}

/// A step that brings its own build runs the image that build produces, not
/// the service's image. The service's image being on the host says nothing
/// about the step's build, so reading it as "already there" left the step
/// with an image nobody built.
#[tokio::test]
async fn pre_start_step_with_its_own_build_does_not_reuse_the_service_image() {
	let peer = peer(0, false);
	let engine = engine(&peer);
	let file = crate::parse_str(
		"services:\n  web:\n    image: base\n    pre_start:\n      - build: {context: init, tags: [init:latest]}\n",
	)
	.unwrap();
	let steps = engine.pre_start_steps("web", &file.services["web"]);
	assert!(
		steps[0].image.is_none(),
		"a step that declares its own build must not carry the service's image: {:?}",
		steps[0].image
	);
	assert!(
		steps[0].build.is_some(),
		"a step that declares its own build keeps it"
	);
	engine
		.run_pre_start("proj-web", "web", &file.services["web"], &file)
		.await
		.unwrap();
	let (looked_for_step_tag, looked_for_service_image) = {
		let seen = peer.requests.lock().unwrap();
		(
			seen.iter().any(|(path, _)| path.contains("init%3Alatest")),
			seen.iter().any(|(path, _)| path.contains("/images/base/")),
		)
	};
	assert!(
		looked_for_step_tag,
		"the engine must look for the image the step's own build produces"
	);
	assert!(
		!looked_for_service_image,
		"the engine must not look for the service's image on the step's behalf"
	);
	let specs = created(&peer);
	assert_eq!(specs.len(), 1);
	assert_eq!(
		specs[0]["image"], "init:latest",
		"the init container runs the image its own build produces"
	);
}

/// A step that brings its own image is pulled, even when the service it
/// belongs to is built from source. Inheriting the service's build made the
/// engine read the step's image as locally built, so the pull was skipped
/// and the step ran whatever the service's build had left behind.
#[tokio::test]
async fn pre_start_step_with_its_own_image_is_pulled_and_not_built() {
	let peer = peer(0, false);
	let engine = engine(&peer);
	let file = crate::parse_str(
		"services:\n  web:\n    build: {context: ., tags: [web:latest]}\n    pre_start:\n      - image: busybox\n",
	)
	.unwrap();
	let steps = engine.pre_start_steps("web", &file.services["web"]);
	assert_eq!(steps[0].image.as_deref(), Some("busybox"));
	assert!(
		steps[0].build.is_none(),
		"a step that declares its own image must not inherit the service's build"
	);
	engine
		.run_pre_start("proj-web", "web", &file.services["web"], &file)
		.await
		.unwrap();
	let (pulled, built) = {
		let seen = peer.requests.lock().unwrap();
		(
			seen.iter()
				.any(|(path, _)| path.contains("/images/pull?reference=busybox")),
			seen.iter().any(|(path, _)| path.contains("/build?")),
		)
	};
	assert!(pulled, "the step's own image must be pulled");
	assert!(
		!built,
		"the step must not be built from the service's context"
	);
	let specs = created(&peer);
	assert_eq!(specs.len(), 1);
	assert_eq!(specs[0]["image"], "busybox");
}
