//! Engine integration tests for `build`: the Dockerfile shapes a compose file can
//! declare, and the build options that override the args it declares.
use std::fs;

use super::*;
// Build
// ---------------------------------------------------------------------------

#[tokio::test]
async fn build_with_target_stage() {
	let _build_lock = BUILD_IMAGES.lock().await;
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	// Multi-stage Dockerfile: build with target: base covers build.rs L77.
	// Each stage leaves its name on disk, so which one was built is readable from
	// inside the container. With `RUN echo` alone both stages produce an image
	// that looks the same from outside, and a build that ignored `target:`
	// entirely would have passed.
	fs::write(
		dir.path().join("Dockerfile"),
		b"FROM alpine:latest AS base\nRUN echo base-stage > /stage\nFROM base AS final\nRUN echo final-stage > /stage\n",
	)
	.unwrap();

	let proj = proj("bst");
	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	let image_tag = format!("podup-test-bst-{}:latest", std::process::id());
	let yaml = format!(
		"services:\n  app:\n    build:\n      context: .\n      target: base\n    image: {image_tag}\n    command: [\"sleep\", \"infinity\"]\n"
	);
	let file = parse_str(&yaml).unwrap();

	engine.up(&file).await.unwrap();
	let stage = engine
		.test_exec_capture(
			&format!("{proj}-app-1"),
			vec!["cat".into(), "/stage".into()],
		)
		.await
		.unwrap_or_default();
	engine.down(&file).await.unwrap();
	let _ = std::process::Command::new("podman")
		.args(["rmi", "-f", &image_tag])
		.status();

	assert_eq!(
		stage.trim(),
		"base-stage",
		"build stopped at the wrong stage: target: base must not run the final stage"
	);
}

#[tokio::test]
async fn build_with_args_and_extra_tags() {
	let _build_lock = BUILD_IMAGES.lock().await;
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let proj = proj("bat");
	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	let pid = std::process::id();
	let main_tag = format!("podup-test-bat-{}:latest", pid);
	let extra_tag = format!("podup-test-bat-extra-{}:v1", pid);
	let yaml = format!(
		"services:\n  app:\n    build:\n      context: .\n      dockerfile_inline: |\n        FROM alpine:latest\n        ARG VERSION=0\n        RUN echo $$VERSION > /version\n      args:\n        VERSION: \"1.0\"\n      tags:\n        - {extra_tag}\n    image: {main_tag}\n    command: [\"sleep\", \"infinity\"]\n"
	);
	let file = parse_str(&yaml).unwrap();

	engine.up(&file).await.unwrap();
	// Two separate problems, both of which made the old version unfalsifiable.
	// `RUN echo` leaves nothing in the image, so a build that dropped the arg
	// produced a byte-identical result, hence writing the value to a file. And
	// the `$$` is load-bearing: compose substitutes `$VERSION` in the YAML before
	// the Dockerfile is ever assembled, so the single-dollar form the old test
	// used reached the build as an empty string and the ARG was never exercised
	// at all.
	let version = engine
		.test_exec_capture(
			&format!("{proj}-app-1"),
			vec!["cat".into(), "/version".into()],
		)
		.await
		.unwrap_or_default();
	let extra = image_id(&extra_tag);
	let main = image_id(&main_tag);
	engine.down(&file).await.unwrap();
	for t in [&main_tag, &extra_tag] {
		let _ = std::process::Command::new("podman")
			.args(["rmi", "-f", t])
			.status();
	}

	assert_eq!(
		version.trim(),
		"1.0",
		"the build arg did not reach the image"
	);
	assert!(!extra.is_empty(), "the extra tag was never applied");
	assert_eq!(
		extra, main,
		"the extra tag points at a different image than the main one"
	);
}

#[tokio::test]
async fn build_with_cli_no_cache_and_build_arg() {
	let _build_lock = BUILD_IMAGES.lock().await;
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let proj = proj("bco");
	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	let tag = format!("podup-test-bco-{}:latest", std::process::id());
	let yaml = format!(
		"services:\n  app:\n    build:\n      context: .\n      dockerfile_inline: |\n        FROM alpine:latest\n        ARG VERSION=0\n        RUN echo $$VERSION > /version\n      args:\n        VERSION: \"1.0\"\n    image: {tag}\n    command: [\"sleep\", \"infinity\"]\n"
	);
	let file = parse_str(&yaml).unwrap();

	// CLI overrides: force no-cache and override the compose VERSION build arg.
	engine
		.build_all_with_options(
			&file,
			&[],
			&podup::BuildOptions::new(true, false, vec!["VERSION=2.0".to_string()], false),
		)
		.await
		.unwrap();
	// The point of the override is that 2.0 wins over the 1.0 in the compose file.
	// This test never starts a container, so read the baked value straight out of
	// the image with a throwaway run.
	let baked = String::from_utf8_lossy(
		&std::process::Command::new("podman")
			.args(["run", "--rm", &tag, "cat", "/version"])
			.output()
			.expect("podman run")
			.stdout,
	)
	.trim()
	.to_string();
	let _ = std::process::Command::new("podman")
		.args(["rmi", "-f", &tag])
		.status();

	assert_eq!(
		baked, "2.0",
		"the CLI build arg did not override the value in the compose file"
	);
}

#[tokio::test]
async fn build_inline_dockerfile() {
	let _build_lock = BUILD_IMAGES.lock().await;
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let proj = proj("bld");
	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	let image_tag = format!("podup-test-build-{}:latest", std::process::id());
	let yaml = format!(
		"services:\n  app:\n    build:\n      context: .\n      dockerfile_inline: |\n        FROM alpine:latest\n        RUN echo inline-built > /built\n    image: {image_tag}\n    command: [\"sleep\", \"infinity\"]\n"
	);
	let file = parse_str(&yaml).unwrap();

	engine.up(&file).await.unwrap();
	// `RUN echo built` left nothing to look at, so an inline Dockerfile that was
	// ignored in favour of a plain `FROM alpine` pull produced the same container.
	let built = engine
		.test_exec_capture(
			&format!("{proj}-app-1"),
			vec!["cat".into(), "/built".into()],
		)
		.await
		.unwrap_or_default();
	engine.down(&file).await.unwrap();
	let _ = std::process::Command::new("podman")
		.args(["rmi", "-f", &image_tag])
		.status();

	assert_eq!(
		built.trim(),
		"inline-built",
		"dockerfile_inline was not the image that got built"
	);
}

#[tokio::test]
async fn build_from_dockerfile_in_context() {
	let _build_lock = BUILD_IMAGES.lock().await;
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	fs::write(
		dir.path().join("Dockerfile"),
		b"FROM alpine:latest\nRUN echo context-build > /built\n",
	)
	.unwrap();

	let proj = proj("bdc");
	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	let image_tag = format!("podup-test-build-ctx-{}:latest", std::process::id());
	let yaml = format!(
		"services:\n  app:\n    build:\n      context: .\n    image: {image_tag}\n    command: [\"sleep\", \"infinity\"]\n"
	);
	let file = parse_str(&yaml).unwrap();

	engine.up(&file).await.unwrap();
	// The context's own Dockerfile has to be the one that built the image. With
	// `RUN echo` alone, a build that fell back to pulling alpine straight looked
	// identical from outside.
	let built = engine
		.test_exec_capture(
			&format!("{proj}-app-1"),
			vec!["cat".into(), "/built".into()],
		)
		.await
		.unwrap_or_default();
	engine.down(&file).await.unwrap();
	let _ = std::process::Command::new("podman")
		.args(["rmi", "-f", &image_tag])
		.status();

	assert_eq!(
		built.trim(),
		"context-build",
		"the Dockerfile in the build context was not used"
	);
}
