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
