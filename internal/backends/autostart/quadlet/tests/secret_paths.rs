//! Secret mount paths across upgrades from the target-less units of 6.1.0.

use super::{install_quadlet, with_env, FakeCtl};
use crate::compose::types::ComposeFile;
use crate::{parse_str, quadlet};
use std::path::{Path, PathBuf};

const COMPOSE: &str = "services:\n  web:\n    image: nginx\n    secrets:\n      - source: token\n        uid: '1001'\n        gid: '1002'\n        mode: 0o440\nsecrets:\n  token:\n    file: ./token.txt\n";

fn seed_unit(root: &Path, service: &str, secret: &str) -> PathBuf {
	let dir = root.join("containers/systemd");
	std::fs::create_dir_all(&dir).unwrap();
	let path = dir.join(format!("proj-{service}.container"));
	std::fs::write(
		&path,
		format!(
			"# podup-owner: proj\n[Container]\nImage=nginx\nContainerName=proj-{service}\nLabel=podup.project=proj\nLabel=podup.service={service}\nSecret={secret}\n"
		),
	)
	.unwrap();
	path
}

fn generated(file: &ComposeFile, root: &Path) -> Vec<quadlet::QuadletUnit> {
	let secret = root.join("token.txt");
	std::fs::write(&secret, "secret value").unwrap();
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt;
		std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o644)).unwrap();
	}
	quadlet::generate_for_autostart(file, "proj", root).units
}

fn install(file: &ComposeFile, root: &Path) {
	install_quadlet(&FakeCtl::new(), file, "proj", root, true, false).unwrap();
}

fn assert_secret(root: &Path, service: &str, expected: &str) {
	let contents =
		std::fs::read_to_string(root.join(format!("containers/systemd/proj-{service}.container")))
			.unwrap();
	let lines: Vec<_> = contents
		.lines()
		.filter_map(|line| line.strip_prefix("Secret="))
		.collect();
	assert_eq!(lines, vec![expected], "{contents}");
}

#[test]
fn legacy_path_is_kept_with_new_mode_uid_gid_and_a_notice() {
	with_env(|root| {
		let old = seed_unit(root, "web", "proj_secret_token,uid=9,gid=8,mode=600");
		let file = parse_str(COMPOSE).unwrap();
		let mut units = generated(&file, root);
		let installed = super::super::installed_units("proj").unwrap();
		assert_eq!(installed, vec![old]);

		let notes = super::super::secret_paths::keep_installed_paths(&mut units, &file, &installed);
		assert_eq!(
			notes,
			vec!["keeping secret proj_secret_token at /run/secrets/proj_secret_token, as the installed unit mounts it; set target: token in the compose file to move it"]
		);
		install(&file, root);
		assert_secret(
			root,
			"web",
			"proj_secret_token,target=proj_secret_token,uid=1001,gid=1002,mode=440",
		);
	});
}

#[test]
fn external_secret_keeps_its_physical_name_as_target() {
	with_env(|root| {
		seed_unit(root, "web", "physical");
		let file = parse_str(
			"services:\n  web:\n    image: nginx\n    secrets: [token]\nsecrets:\n  token:\n    external: true\n    name: physical\n",
		)
		.unwrap();
		let mut units = generated(&file, root);
		let installed = super::super::installed_units("proj").unwrap();

		let notes = super::super::secret_paths::keep_installed_paths(&mut units, &file, &installed);
		assert_eq!(
			notes,
			vec!["keeping secret physical at /run/secrets/physical, as the installed unit mounts it; set target: token in the compose file to move it"]
		);
		install(&file, root);
		assert_secret(root, "web", "physical,target=physical");
	});
}

#[test]
fn explicit_compose_target_overrides_the_legacy_path_without_a_notice() {
	with_env(|root| {
		seed_unit(root, "web", "proj_secret_token");
		let file =
			parse_str(&COMPOSE.replace("source: token", "source: token\n        target: chosen"))
				.unwrap();
		let mut units = generated(&file, root);
		let before = units.clone();
		let installed = super::super::installed_units("proj").unwrap();

		let notes = super::super::secret_paths::keep_installed_paths(&mut units, &file, &installed);
		assert!(notes.is_empty(), "{notes:?}");
		assert_eq!(units, before);
		install(&file, root);
		assert_secret(
			root,
			"web",
			"proj_secret_token,target=chosen,uid=1001,gid=1002,mode=440",
		);
	});
}

#[test]
fn clean_install_uses_the_compose_secret_name_without_a_notice() {
	with_env(|root| {
		let file = parse_str(COMPOSE).unwrap();
		let mut units = generated(&file, root);
		let before = units.clone();
		let installed = super::super::installed_units("proj").unwrap();
		assert!(installed.is_empty());

		let notes = super::super::secret_paths::keep_installed_paths(&mut units, &file, &installed);
		assert!(notes.is_empty(), "{notes:?}");
		assert_eq!(units, before);
		install(&file, root);
		assert_secret(
			root,
			"web",
			"proj_secret_token,target=token,uid=1001,gid=1002,mode=440",
		);
	});
}

#[test]
fn installed_compose_target_is_unchanged_without_a_notice() {
	with_env(|root| {
		seed_unit(
			root,
			"web",
			"proj_secret_token,target=token,uid=1001,gid=1002,mode=440",
		);
		let file = parse_str(COMPOSE).unwrap();
		let mut units = generated(&file, root);
		let before = units.clone();
		let installed = super::super::installed_units("proj").unwrap();

		let notes = super::super::secret_paths::keep_installed_paths(&mut units, &file, &installed);
		assert!(notes.is_empty(), "{notes:?}");
		assert_eq!(units, before);
		install(&file, root);
		assert_secret(
			root,
			"web",
			"proj_secret_token,target=token,uid=1001,gid=1002,mode=440",
		);
	});
}

#[test]
fn second_reinstall_keeps_the_previously_preserved_legacy_path() {
	with_env(|root| {
		let path = seed_unit(root, "web", "proj_secret_token");
		let file = parse_str(COMPOSE).unwrap();
		install(&file, root);
		assert_secret(
			root,
			"web",
			"proj_secret_token,target=proj_secret_token,uid=1001,gid=1002,mode=440",
		);
		let first = std::fs::read_to_string(&path).unwrap();
		// The installed input is now the first reinstall's actual output, not
		// another target-less fixture. The compose still has no target.
		let mut units = generated(&file, root);
		let installed = super::super::installed_units("proj").unwrap();
		let notes = super::super::secret_paths::keep_installed_paths(&mut units, &file, &installed);
		assert!(notes.is_empty(), "{notes:?}");
		install(&file, root);
		assert_secret(
			root,
			"web",
			"proj_secret_token,target=proj_secret_token,uid=1001,gid=1002,mode=440",
		);
		assert_eq!(std::fs::read_to_string(path).unwrap(), first);
	});
}

#[test]
fn unmarked_unit_is_excluded_from_secret_path_reads() {
	with_env(|root| {
		let path = seed_unit(root, "web", "proj_secret_token");
		let unmarked =
			std::fs::read_to_string(&path)
				.unwrap()
				.replacen("# podup-owner: proj\n", "", 1);
		std::fs::write(&path, &unmarked).unwrap();
		let file = parse_str(COMPOSE).unwrap();
		let mut units = generated(&file, root);
		let before = units.clone();
		let installed = super::super::installed_units("proj").unwrap();
		assert!(installed.is_empty(), "{installed:?}");

		let notes = super::super::secret_paths::keep_installed_paths(&mut units, &file, &installed);
		assert!(notes.is_empty(), "{notes:?}");
		assert_eq!(units, before);
		assert_eq!(std::fs::read_to_string(&path).unwrap(), unmarked);
		install(&file, root);
		assert_secret(
			root,
			"web",
			"proj_secret_token,target=token,uid=1001,gid=1002,mode=440",
		);
	});
}

#[test]
fn shared_secret_keeps_each_services_distinct_installed_destination() {
	with_env(|root| {
		seed_unit(root, "web", "proj_secret_token");
		seed_unit(root, "api", "proj_secret_token,target=token");
		let file = parse_str(&COMPOSE.replace(
			"secrets:\n  token:",
			"  api:\n    image: nginx\n    secrets: [token]\nsecrets:\n  token:",
		))
		.unwrap();
		let mut units = generated(&file, root);
		let installed = super::super::installed_units("proj").unwrap();
		assert_eq!(installed.len(), 2);

		let notes = super::super::secret_paths::keep_installed_paths(&mut units, &file, &installed);
		assert_eq!(
			notes,
			vec!["keeping secret proj_secret_token at /run/secrets/proj_secret_token, as the installed unit mounts it; set target: token in the compose file to move it"]
		);
		install(&file, root);
		assert_secret(
			root,
			"web",
			"proj_secret_token,target=proj_secret_token,uid=1001,gid=1002,mode=440",
		);
		assert_secret(root, "api", "proj_secret_token,target=token,mode=644");
	});
}
