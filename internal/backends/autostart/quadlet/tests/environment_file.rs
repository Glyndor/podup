//! The env file of a container unit is installed, replaced and removed with it.
//! The values stand for passwords, so no failure message holds one.

use std::os::unix::fs::PermissionsExt;

use super::*;

const WITH: &str =
	"services:\n  web:\n    image: nginx\n    environment:\n      DB_PASSWORD: marker-value-4711\n";

fn install(yaml: &str) {
	install_quadlet(
		&FakeCtl::new(),
		&parse_str(yaml).unwrap(),
		"proj",
		Path::new(BASE),
		true,
		false,
	)
	.unwrap();
}

#[test]
fn install_writes_the_file_private_and_uninstall_removes_it() {
	with_env(|root| {
		install(WITH);
		let units = root.join("containers/systemd");
		let env = units.join("proj-web.podup.env");
		let mode = std::fs::metadata(&env).unwrap().permissions().mode();
		assert_eq!(mode & 0o777, 0o600);
		let unit = std::fs::read_to_string(units.join("proj-web.container")).unwrap();
		assert!(unit.contains("EnvironmentFile=proj-web.podup.env\n"));
		assert!(!unit.contains("marker-value"), "the unit holds the value");
		uninstall_quadlet(&FakeCtl::new(), "proj").unwrap();
		assert!(!env.exists(), "the values stay behind");
	});
}

#[test]
fn a_reinstall_without_values_removes_the_file() {
	with_env(|root| {
		install(WITH);
		install(IMG);
		assert!(!root.join("containers/systemd/proj-web.podup.env").exists());
		assert!(root.join("containers/systemd/proj-web.container").exists());
	});
}

/// An earlier version wrote the values into the container unit. Installing over
/// it takes them out of the unit.
#[test]
fn a_reinstall_over_a_unit_that_holds_the_values_moves_them_to_the_file() {
	with_env(|root| {
		let units = root.join("containers/systemd");
		std::fs::create_dir_all(&units).unwrap();
		std::fs::set_permissions(&units, std::fs::Permissions::from_mode(0o700)).unwrap();
		let unit = units.join("proj-web.container");
		std::fs::write(
			&unit,
			"# podup-owner: proj\n[Container]\nImage=nginx\nEnvironment=DB_PASSWORD=marker-value-4711\n",
		)
		.unwrap();
		install(WITH);
		let text = std::fs::read_to_string(&unit).unwrap();
		assert!(!text.contains("marker-value"), "the unit holds the value");
		assert!(units.join("proj-web.podup.env").exists());
	});
}

#[test]
fn another_projects_env_file_is_left_alone() {
	with_env(|root| {
		let units = root.join("containers/systemd");
		std::fs::create_dir_all(&units).unwrap();
		std::fs::set_permissions(&units, std::fs::Permissions::from_mode(0o700)).unwrap();
		let sibling = units.join("proj-web.podup.env");
		std::fs::write(&sibling, "# podup-owner: proj-web\nA=b\n").unwrap();
		let refused = install_quadlet(
			&FakeCtl::new(),
			&parse_str(WITH).unwrap(),
			"proj",
			Path::new(BASE),
			true,
			false,
		);
		assert!(refused.is_err());
		assert!(!units.join("proj-web.container").exists());
		assert!(
			std::fs::read_to_string(&sibling).unwrap() == "# podup-owner: proj-web\nA=b\n",
			"the sibling's file changed"
		);
	});
}
