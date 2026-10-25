//! Engine integration tests for the image a `build --no-cache` produces surviving
//! the `up` that follows it.

use super::*;
/// #1094: `up` must not rebuild a service whose image is already present.
///
/// It used to rebuild unconditionally, *with* the cache, so the rebuild could
/// resolve to an older layer chain and retag the image backwards, silently
/// discarding a `build --no-cache` that had just run. That breaks the ordinary
/// deploy shape: build explicitly, then start.
///
/// The sequence matters. A plain `build` first seeds the cache with one chain;
/// `build --no-cache` then produces a different one and tags it. Without both
/// steps there is no older chain for `up` to revert to and the bug does not
/// appear at all.
#[tokio::test]
async fn up_keeps_the_image_a_no_cache_build_produced() {
	let _build_lock = BUILD_IMAGES.lock().await;
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let proj = proj("upnb");
	let tag = format!("podup-test-upnb-{}:latest", std::process::id());
	let _image = TestImage::new(&tag);
	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	let yaml = format!(
		"services:\n  app:\n    build:\n      context: .\n      dockerfile_inline: |\n        FROM alpine:latest\n        RUN echo marker > /marker\n    image: {tag}\n    command: [\"sleep\", \"infinity\"]\n"
	);
	let file = parse_str(&yaml).unwrap();

	// Seed the cache with one chain, then force a different one.
	engine.build_all(&file, &[]).await.unwrap();
	engine
		.build_all_with_options(
			&file,
			&[],
			&podup::BuildOptions::new(true, false, Vec::new(), false),
		)
		.await
		.unwrap();
	let after_build = image_id(&tag);

	engine
		.up_with_options(&file, true, &[], &[], false, false, false, false)
		.await
		.unwrap();
	let after_up = image_id(&tag);

	assert_eq!(
		after_build, after_up,
		"`up` must not retag the image built by `build --no-cache`"
	);

	engine.down_with_options(&file, true).await.ok();
}
