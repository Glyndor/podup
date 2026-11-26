//! The drop-in that goes with each installed container service: written with the units, refused when
//! it is someone else's, and removed with its unit.

use super::{install_quadlet, uninstall_quadlet, FakeCtl, IMG};
use crate::parse_str;
use std::path::{Path, PathBuf};

const BASE: &str = "/srv/app";
const NAME: &str = "50-podup-ownership.conf";

fn drop_in(root: &Path, service: &str) -> PathBuf {
	root.join("systemd/user")
		.join(format!("{service}.d"))
		.join(NAME)
}

fn install(yaml: &str, project: &str, dry_run: bool) -> crate::Result<FakeCtl> {
	let sc = FakeCtl::new();
	install_quadlet(
		&sc,
		&parse_str(yaml).unwrap(),
		project,
		Path::new(BASE),
		true,
		dry_run,
	)?;
	Ok(sc)
}

#[test]
fn install_writes_a_private_drop_in_for_each_container_service_only() {
	use std::os::unix::fs::PermissionsExt;
	super::with_env(|root| {
		install(
			"services:\n  web:\n    image: nginx\n  db:\n    image: postgres\n",
			"proj",
			false,
		)
		.unwrap();
		for service in ["proj-web.service", "proj-db.service"] {
			let path = drop_in(root, service);
			let contents = std::fs::read_to_string(&path).unwrap();
			assert!(contents.starts_with("# podup-owner: proj\n"), "{contents}");
			assert!(contents.contains("ExecStopPost=\n"), "{contents}");
			let mode = std::fs::metadata(&path).unwrap().permissions().mode();
			assert_eq!(mode & 0o777, 0o600, "{mode:o}");
		}
		let entries: Vec<_> = std::fs::read_dir(root.join("systemd/user"))
			.unwrap()
			.map(|e| e.unwrap().file_name().into_string().unwrap())
			.collect();
		assert_eq!(entries.len(), 2, "the network unit gets none: {entries:?}");
	});
}

#[test]
fn install_refuses_another_projects_drop_in_and_writes_nothing() {
	super::with_env(|root| {
		let path = drop_in(root, "proj-web.service");
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		std::fs::write(&path, "# podup-owner: other\n").unwrap();
		let sc = FakeCtl::new();
		let err = install_quadlet(
			&sc,
			&parse_str(IMG).unwrap(),
			"proj",
			Path::new(BASE),
			false,
			false,
		)
		.unwrap_err();
		assert!(
			err.to_string().contains("belongs to project 'other'"),
			"{err}"
		);
		assert!(!root.join("containers/systemd/proj-web.container").exists());
		assert_eq!(
			std::fs::read_to_string(&path).unwrap(),
			"# podup-owner: other\n"
		);
		assert!(sc.log().is_empty(), "{:?}", sc.log());
	});
}

#[test]
fn install_refuses_a_drop_in_that_is_a_link_and_writes_nothing() {
	super::with_env(|root| {
		let path = drop_in(root, "proj-web.service");
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		std::os::unix::fs::symlink(root.join("elsewhere"), &path).unwrap();
		assert!(install(IMG, "proj", false).is_err());
		assert!(!root.join("containers/systemd/proj-web.container").exists());
		assert!(!root.join("elsewhere").exists());
	});
}

#[test]
fn dry_run_shows_the_drop_in_and_writes_none() {
	super::with_env(|root| {
		let file = parse_str(IMG).unwrap();
		let units = crate::quadlet::generate_for_autostart(&file, "proj", Path::new(BASE)).units;
		let plan = super::super::dry_run_plan(
			&units,
			&super::super::drop_ins::plan("proj", &units),
			&root.join("containers/systemd"),
			&["proj-web.service".to_string()],
			&[],
			false,
		);
		let text = plan.join("\n");
		assert!(
			text.contains(&format!("# proj-web.service.d/{NAME}\n# podup-owner: proj")),
			"{text}"
		);
		assert!(text.contains("# would write 1 drop-in(s) under "), "{text}");
		install(IMG, "proj", true).unwrap();
		assert!(!drop_in(root, "proj-web.service").exists());
	});
}

#[test]
fn uninstall_removes_this_projects_drop_ins_and_leaves_the_rest() {
	super::with_env(|root| {
		install(IMG, "app", false).unwrap();
		install(IMG, "app-x", false).unwrap();
		let own = drop_in(root, "app-web.service");
		let users = own.with_file_name("10-mine.conf");
		std::fs::write(&users, "[Service]\n").unwrap();
		uninstall_quadlet(&FakeCtl::new(), "app").unwrap();
		assert!(!own.exists());
		assert!(users.is_file(), "a drop-in of the user's stays");
		assert!(drop_in(root, "app-x-web.service").is_file());
	});
}

#[test]
fn uninstall_removes_the_empty_drop_in_directory() {
	super::with_env(|root| {
		install(IMG, "proj", false).unwrap();
		uninstall_quadlet(&FakeCtl::new(), "proj").unwrap();
		assert!(!root.join("systemd/user/proj-web.service.d").exists());
	});
}

#[test]
fn a_service_removed_from_the_file_loses_its_drop_in() {
	super::with_env(|root| {
		install(
			"services:\n  web:\n    image: nginx\n  db:\n    image: postgres\n",
			"proj",
			false,
		)
		.unwrap();
		install(IMG, "proj", false).unwrap();
		assert!(drop_in(root, "proj-web.service").is_file());
		assert!(!drop_in(root, "proj-db.service").exists());
	});
}
