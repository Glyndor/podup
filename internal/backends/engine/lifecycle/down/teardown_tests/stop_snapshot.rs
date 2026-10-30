use super::engine_with;
use crate::compose::types::{ComposeFile, Service};
use crate::engine::fake_podman;

/// The stop at the end of a foreground `up` is for the containers it was
/// attached to. A container that took the name since, put there by another
/// `up`, is a different container and is left running.
#[tokio::test]
#[cfg(unix)]
async fn stop_snapshot_leaves_a_container_that_replaced_one_of_the_snapshot() {
	use std::sync::atomic::{AtomicUsize, Ordering};
	use std::sync::Arc;
	let listings = Arc::new(AtomicUsize::new(0));
	let seen = listings.clone();
	let fake = fake_podman::start(move |method, target| {
		if method == "GET" && target.contains("/containers/json") {
			// The first listing is the snapshot; later ones show web-1 replaced.
			let id = if seen.fetch_add(1, Ordering::SeqCst) == 0 {
				"aaaa"
			} else {
				"bbbb"
			};
			(
				200,
				format!(
					r#"[
						{{"Id":"{id}","Names":["/proj-web-1"],"Labels":{{"podup.service":"web"}}}},
						{{"Id":"cccc","Names":["/proj-db-1"],"Labels":{{"podup.service":"db"}}}}
					]"#
				),
			)
		} else if method == "POST" && target.contains("/stop") {
			(204, String::new())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "proj");
	let mut file = ComposeFile::default();
	file.services.insert("web".into(), Service::default());
	file.services.insert("db".into(), Service::default());

	let snapshot = e.snapshot_containers().await.unwrap();
	e.stop_snapshot(&file, &snapshot).await.unwrap();

	let seen = fake.requests.lock().unwrap();
	let posts: Vec<_> = seen.iter().filter(|r| r.starts_with("POST ")).collect();
	assert_eq!(posts.len(), 1, "{seen:?}");
	let expected = format!("POST {}/containers/cccc/stop", crate::libpod::API_PREFIX);
	assert_eq!(posts[0].split('?').next(), Some(expected.as_str()));
}

#[tokio::test]
#[cfg(unix)]
async fn stop_snapshot_posts_to_the_id_even_when_the_name_changes_owner_after_the_listing() {
	use std::sync::atomic::{AtomicUsize, Ordering};
	use std::sync::Arc;

	for replaced_at_post in [false, true] {
		let listings = Arc::new(AtomicUsize::new(0));
		let listed = listings.clone();
		let fake = fake_podman::start(move |method, target| {
			if method == "GET" && target.contains("/containers/json") {
				listed.fetch_add(1, Ordering::SeqCst);
				return (
					200,
					r#"[{"Id":"aaaa","Names":["/proj-web-1"],"Labels":{"podup.service":"web"}}]"#
						.into(),
				);
			}
			if method == "POST" && target.contains("/stop") {
				// At POST time the old ID may be absent while the name has a new owner.
				if replaced_at_post && target.contains("/containers/aaaa/stop?") {
					return (404, r#"{"message":"not found"}"#.into());
				}
				return (204, String::new());
			}
			(404, r#"{"message":"not found"}"#.into())
		});
		let e = engine_with(fake.client(), "proj");
		let mut file = ComposeFile::default();
		file.services.insert("web".into(), Service::default());

		let snapshot = e.snapshot_containers().await.unwrap();
		e.stop_snapshot(&file, &snapshot).await.unwrap();

		assert_eq!(listings.load(Ordering::SeqCst), 2);
		let seen = fake.requests.lock().unwrap();
		let posts: Vec<_> = seen.iter().filter(|r| r.starts_with("POST ")).collect();
		assert_eq!(posts.len(), 1, "{seen:?}");
		let expected = format!("POST {}/containers/aaaa/stop", crate::libpod::API_PREFIX);
		assert_eq!(posts[0].split('?').next(), Some(expected.as_str()));
	}
}

#[tokio::test]
#[cfg(unix)]
async fn stop_snapshot_excludes_one_off_containers_of_the_same_service() {
	let fake = fake_podman::start(|method, target| {
		if method == "GET" && target.contains("/containers/json") {
			(200, r#"[
				{"Id":"aaaa","Names":["/proj-web-1"],"Labels":{"podup.service":"web"}},
				{"Id":"bbbb","Names":["/proj-web-run-123"],"Labels":{"podup.service":"web","podup.oneoff":"true"}}
			]"#.into())
		} else if method == "POST" && target.contains("/stop") {
			(204, String::new())
		} else {
			(404, r#"{"message":"not found"}"#.into())
		}
	});
	let e = engine_with(fake.client(), "proj");
	let mut file = ComposeFile::default();
	file.services.insert("web".into(), Service::default());

	let snapshot = e.snapshot_containers().await.unwrap();
	e.stop_snapshot(&file, &snapshot).await.unwrap();

	let seen = fake.requests.lock().unwrap();
	let posts: Vec<_> = seen.iter().filter(|r| r.starts_with("POST ")).collect();
	assert_eq!(posts.len(), 1, "{seen:?}");
	let expected = format!("POST {}/containers/aaaa/stop", crate::libpod::API_PREFIX);
	assert_eq!(posts[0].split('?').next(), Some(expected.as_str()));
}
