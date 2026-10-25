use super::pre_start_test_support::{created, engine, file, peer};
use super::{ExistingContainer, ResolvedImage};
use std::collections::HashMap;

#[tokio::test]
async fn pre_start_up_failure_preserves_the_previous_service() {
	let peer = peer(7, true);
	let engine = engine(&peer);
	let file = file();
	let existing = HashMap::from([(
		"proj-web".into(),
		ExistingContainer {
			config_hash: None,
			image_id: String::new(),
			service: Some("web".into()),
		},
	)]);
	let error = engine
		.up_one_replica(
			"proj-web".into(),
			"web",
			&file.services["web"],
			&file,
			&existing,
			"new",
			&ResolvedImage::new(),
			false,
			true,
			true,
		)
		.await
		.unwrap_err();
	assert_eq!(
		error.to_string(),
		"service 'web': pre_start step 1: exited with code 7"
	);
	assert_eq!(created(&peer).len(), 1);
	let seen = peer.requests.lock().unwrap();
	assert!(!seen.iter().any(|(path, _)| path.contains("/proj-web/stop")
		|| path.contains("DELETE") && path.contains("/proj-web?")));
	assert!(!seen
		.iter()
		.any(|(path, _)| path.contains("/proj-web/start")));
	assert!(seen.iter().any(|(path, _)| path.contains("DELETE")
		&& path.contains("pre-start")
		&& path.contains("v=true")));
}

#[tokio::test]
async fn pre_start_up_initializes_stopped_reused_containers_only() {
	for no_recreate in [false, true] {
		for running in [false, true] {
			let peer = peer(0, running);
			let engine = engine(&peer);
			let file = file();
			let existing = HashMap::from([(
				"proj-web".into(),
				ExistingContainer {
					config_hash: Some("same".into()),
					image_id: "same-image".into(),
					service: Some("web".into()),
				},
			)]);
			engine
				.up_one_replica(
					"proj-web".into(),
					"web",
					&file.services["web"],
					&file,
					&existing,
					"same",
					&ResolvedImage::new(),
					no_recreate,
					false,
					true,
				)
				.await
				.unwrap();
			assert_eq!(created(&peer).len(), usize::from(!running));
			let seen = peer.requests.lock().unwrap();
			let service_start = seen
				.iter()
				.position(|(path, _)| path.contains("/proj-web/start"))
				.unwrap();
			if !running {
				let wait = seen
					.iter()
					.position(|(path, _)| path.contains("/wait?"))
					.unwrap();
				assert!(wait < service_start);
			}
		}
	}
}
