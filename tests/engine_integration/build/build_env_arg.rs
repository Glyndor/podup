//! Engine integration tests for a build arg declared with no value, which takes it
//! from the environment.

use super::*;
// ---------------------------------------------------------------------------
// Build: arg with null value (from environment)
// ---------------------------------------------------------------------------

#[test]
fn build_with_env_arg() {
	let rt = tokio::runtime::Runtime::new().unwrap();
	// FROM_ENV has no explicit value → read from environment (build.rs L89 None branch)
	temp_env::with_var("FROM_ENV", Some("test-value"), || {
		rt.block_on(async {
			let client = match podman().await {
				Some(d) => d,
				None => return,
			};
			let dir = tempfile::tempdir().unwrap();
			let proj = proj("bea");
			let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
			let image_tag = format!("podup-test-bea-{}:latest", std::process::id());
			let yaml = format!(
				"services:\n  app:\n    build:\n      context: .\n      dockerfile_inline: |\n        FROM alpine:latest\n        ARG FROM_ENV\n        RUN echo $$FROM_ENV > /from-env\n      args:\n        FROM_ENV:\n    image: {image_tag}\n    command: [\"sleep\", \"infinity\"]\n"
			);
			let file = parse_str(&yaml).unwrap();

			engine.up(&file).await.unwrap();
			// Same two traps as the build-arg tests in build_images.rs, and this one
			// is the sharper case. A valueless `args: FROM_ENV:` entry is supposed to
			// take its value from the environment, and the single-dollar form the old
			// version used made compose substitute FROM_ENV in the YAML instead, so
			// the value arrived by a completely different route than the one under
			// test, and the ARG never mattered. `RUN echo` then left nothing to
			// contradict it.
			let baked = engine
				.test_exec_capture(
					&format!("{proj}-app-1"),
					vec!["cat".into(), "/from-env".into()],
				)
				.await
				.unwrap_or_default();
			engine.down(&file).await.unwrap();
			let _ = std::process::Command::new("podman")
				.args(["rmi", "-f", &image_tag])
				.status();

			assert_eq!(
				baked.trim(),
				"test-value",
				"a valueless build arg did not pick up the environment variable"
			);
		});
	});
}

// label_file: load tests moved to label_file_safety.rs (#1361).
