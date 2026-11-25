//! The variant of a service's `platform:` picks the instance of a manifest
//! list when the container is created, not only when the image is pulled.
//!
//! The list is built locally from two different images, one filed as
//! `linux/arm/v6` and one as `linux/arm/v7`. Creating a container never runs
//! it, so no emulator is needed: what is read is which image the container was
//! made from.

use std::fs;
use std::process::Command;

use super::*;

fn podman_cli(args: &[&str]) -> std::process::Output {
	Command::new("podman")
		.args(args)
		.output()
		.expect("run the podman CLI")
}

fn image_id(reference: &str) -> String {
	let out = podman_cli(&["image", "inspect", "--format", "{{.Id}}", reference]);
	assert!(out.status.success(), "{reference}: {out:?}");
	String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Removes the manifest list, and the containers made from it, by exact name.
struct Cleanup {
	list: String,
	project: String,
	compose: String,
}

impl Drop for Cleanup {
	fn drop(&mut self) {
		let _ = run(&["-f", &self.compose, "-p", &self.project, "down"]);
		let _ = podman_cli(&["manifest", "rm", &self.list]);
		let _ = podman_cli(&["rmi", "--ignore", &self.list]);
	}
}

#[tokio::test]
async fn the_platform_variant_selects_the_manifest_list_instance_at_create() {
	if super::podman().await.is_none() {
		return;
	}
	for image in [
		"docker.io/library/busybox:latest",
		"docker.io/library/alpine:latest",
	] {
		assert!(
			podman_cli(&["pull", "--quiet", image]).status.success(),
			"could not pull {image}"
		);
	}
	let pid = std::process::id();
	let list = format!("localhost/podup-variant-{pid}");
	let project = format!("t{pid}-variant");
	let dir = tempfile::tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let compose_path = compose.to_str().unwrap().to_string();
	let _cleanup = Cleanup {
		list: list.clone(),
		project: project.clone(),
		compose: compose_path.clone(),
	};
	assert!(podman_cli(&["manifest", "create", &list]).status.success());
	for (variant, source) in [
		("v6", "docker.io/library/busybox:latest"),
		("v7", "docker.io/library/alpine:latest"),
	] {
		let add = podman_cli(&[
			"manifest",
			"add",
			"--os",
			"linux",
			"--arch",
			"arm",
			"--variant",
			variant,
			&list,
			&format!("containers-storage:{source}"),
		]);
		assert!(add.status.success(), "manifest add {variant}: {add:?}");
	}

	for (variant, source) in [
		("v6", "docker.io/library/busybox:latest"),
		("v7", "docker.io/library/alpine:latest"),
	] {
		fs::write(
			&compose,
			format!(
				"services:\n  web:\n    image: {list}\n    platform: linux/arm/{variant}\n    pull_policy: never\n    command: [\"true\"]\n"
			),
		)
		.unwrap();
		let create = run(&["-f", &compose_path, "-p", &project, "create"]);
		assert!(
			create.status.success(),
			"create ({variant}): {}",
			String::from_utf8_lossy(&create.stderr)
		);
		let made_from = podman_cli(&[
			"container",
			"inspect",
			"--format",
			"{{.Image}}",
			&format!("{project}-web-1"),
		]);
		assert_eq!(
			String::from_utf8_lossy(&made_from.stdout).trim(),
			image_id(source),
			"the {variant} instance was not the one the container was created from"
		);
		run(&["-f", &compose_path, "-p", &project, "down"]);
	}
}
