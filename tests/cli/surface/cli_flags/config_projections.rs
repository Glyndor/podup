//! The list projections `config` offers render the lines the operator reads:
//! volumes, images, profiles and the service hash.
//!
//! Split out from `tests/cli/surface/cli_flags.rs`; the helper `bin` is inherited via
//! `use super::*`.

use super::*;

#[test]
fn config_projections_render_lists() {
	let dir = std::env::temp_dir().join(format!("podup-cflags-{}", std::process::id()));
	fs::create_dir_all(&dir).unwrap();
	let compose = dir.join("compose.yaml");
	// `web` is always active; `admin` lives behind the `frontend` profile so the
	// profile-aware projections are exercised when that profile is activated.
	fs::write(
		&compose,
		"services:\n  web:\n    image: nginx:1.27\n  admin:\n    image: busybox:1.36\n    profiles: [frontend]\nvolumes:\n  data: {}\n",
	)
	.unwrap();
	let c = compose.to_str().unwrap();

	let volumes = Command::new(bin())
		.args(["-f", c, "config", "--volumes"])
		.output()
		.unwrap();
	assert!(volumes.status.success());
	assert!(String::from_utf8_lossy(&volumes.stdout)
		.lines()
		.any(|l| l == "data"));

	let images = Command::new(bin())
		.args(["-f", c, "config", "--images"])
		.output()
		.unwrap();
	assert!(String::from_utf8_lossy(&images.stdout).contains("nginx:1.27"));

	// `--profiles` lists the declared profiles of the active services; activate
	// `frontend` so `admin` (and its profile) survive profile resolution.
	let profiles = Command::new(bin())
		.args(["-f", c, "--profile", "frontend", "config", "--profiles"])
		.output()
		.unwrap();
	assert!(String::from_utf8_lossy(&profiles.stdout)
		.lines()
		.any(|l| l == "frontend"));

	let hash = Command::new(bin())
		.args(["-f", c, "config", "--hash", "*"])
		.output()
		.unwrap();
	assert!(hash.status.success());
	assert!(String::from_utf8_lossy(&hash.stdout).contains("web "));

	let _ = fs::remove_dir_all(&dir);
}

/// A service with only `build:` runs the image the build step tags, which carries
/// the project name; `config --images` used to print `web:latest` for it.
#[test]
fn config_images_names_the_tag_a_build_only_service_gets() {
	let dir = tempfile::tempdir().unwrap();
	let file = dir.path().join("compose.yaml");
	std::fs::write(
		&file,
		"services:\n  web:\n    build: .\n  api:\n    build:\n      context: .\n      tags: [registry.example/api:7]\n  db:\n    image: postgres:17\n",
	)
	.unwrap();
	let run = |project: &str| {
		let out = Command::new(bin())
			.args([
				"-f",
				file.to_str().unwrap(),
				"-p",
				project,
				"config",
				"--images",
			])
			.output()
			.unwrap();
		assert!(
			out.status.success(),
			"{}",
			String::from_utf8_lossy(&out.stderr)
		);
		String::from_utf8_lossy(&out.stdout)
			.lines()
			.map(str::to_string)
			.collect::<Vec<_>>()
	};
	let lines = run("demo");
	assert!(lines.contains(&"demo-web:latest".to_string()), "{lines:?}");
	assert!(
		lines.contains(&"registry.example/api:7".to_string()),
		"{lines:?}"
	);
	assert!(lines.contains(&"postgres:17".to_string()), "{lines:?}");
	assert!(run("other").contains(&"other-web:latest".to_string()));
}
