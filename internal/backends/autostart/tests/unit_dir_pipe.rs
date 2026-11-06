//! A pipe in the Quadlet unit directory must not hold the commands that scan it.
//!
//! The directory is read to find which units belong to a project. Opening a
//! FIFO for reading with no writer waits in the kernel until somebody opens the
//! other end, so a FIFO planted under a project's name used to hang the scan,
//! and with it `uninstall`, `status` and `rebuild`, with no output. Each case
//! runs the scan on a thread and fails when it has not returned in 10 seconds.

use super::*;
use std::sync::mpsc;
use std::time::Duration;

use crate::autostart::quadlet::installed_units;
use crate::autostart::uninstall_quadlet;

fn within_budget<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
	let (tx, rx) = mpsc::channel();
	std::thread::spawn(move || {
		let _ = tx.send(f());
	});
	rx.recv_timeout(Duration::from_secs(10))
		.expect("the scan of the unit directory did not return within 10 seconds")
}

fn mkfifo(path: &Path) {
	assert!(std::process::Command::new("mkfifo")
		.arg(path)
		.status()
		.expect("mkfifo")
		.success());
}

fn unit_dir(root: &Path) -> PathBuf {
	let dir = root.join("containers/systemd");
	std::fs::create_dir_all(&dir).unwrap();
	dir
}

#[test]
fn a_pipe_under_the_project_name_is_an_error_naming_it() {
	with_env(|root| {
		let pipe = unit_dir(root).join("proj-front.container");
		mkfifo(&pipe);
		let err = within_budget(|| installed_units("proj")).expect_err("a pipe is not a unit");
		let text = err.to_string();
		assert!(text.contains("proj-front.container"), "{text}");
		assert!(text.contains("not a regular file"), "{text}");
	});
}

#[test]
fn a_link_to_a_pipe_is_left_alone_without_being_followed() {
	with_env(|root| {
		let dir = unit_dir(root);
		let pipe = root.join("elsewhere");
		mkfifo(&pipe);
		std::os::unix::fs::symlink(&pipe, dir.join("proj-link.container")).unwrap();
		let found = within_budget(|| installed_units("proj")).expect("a link is not a failure");
		assert!(found.is_empty(), "{found:?}");
	});
}

const OWN: &str = "# podup-owner: proj\n[Container]\nImage=nginx\n";

/// Entries a user legitimately keeps next to the units: a drop-in directory of
/// this project, one of a sibling project that shares the prefix, a sibling's
/// unit that is not text, and a link to a unit (a dotfiles manager). None of
/// them may stop the scan, and the project's own unit is still found.
fn plant_neighbours(dir: &Path, root: &Path) {
	std::fs::write(dir.join("proj-web.container"), OWN).unwrap();
	std::fs::create_dir(dir.join("proj-web.container.d")).unwrap();
	std::fs::create_dir(dir.join("proj-extra-web.container.d")).unwrap();
	std::fs::write(dir.join("proj-extra-x.container"), [0xff, 0xfe, 0x00, 0xc3]).unwrap();
	std::fs::write(root.join("dotfile.container"), OWN).unwrap();
	std::os::unix::fs::symlink(
		root.join("dotfile.container"),
		dir.join("proj-stowed.container"),
	)
	.unwrap();
	std::fs::write(dir.join("proj-notes.txt"), "not a unit").unwrap();
	mkfifo(&dir.join("proj-socket.fifo"));
}

#[test]
fn neighbours_that_are_not_units_do_not_stop_the_scan() {
	with_env(|root| {
		let dir = unit_dir(root);
		plant_neighbours(&dir, root);
		let found = within_budget(|| installed_units("proj")).expect("neighbours are no failure");
		assert_eq!(found, vec![dir.join("proj-web.container")]);
	});
}

#[test]
fn status_and_uninstall_go_through_the_neighbours() {
	with_env(|root| {
		let dir = unit_dir(root);
		plant_neighbours(&dir, root);
		within_budget(|| collect_status(&FakeCtl::new(), "proj").map(|_| ()))
			.expect("status must report");
		within_budget(|| uninstall_quadlet(&FakeCtl::new(), "proj")).expect("uninstall must run");
		assert!(
			!dir.join("proj-web.container").exists(),
			"own unit not removed"
		);
		assert!(
			dir.join("proj-web.container.d").is_dir(),
			"a drop-in was removed"
		);
		assert!(
			dir.join("proj-stowed.container").exists(),
			"a link was removed"
		);
	});
}

#[test]
fn a_pipe_with_a_drop_in_directory_beside_it_still_names_the_pipe() {
	with_env(|root| {
		let dir = unit_dir(root);
		plant_neighbours(&dir, root);
		mkfifo(&dir.join("proj-front.container"));
		let err = within_budget(|| installed_units("proj")).expect_err("a pipe is not a unit");
		let text = err.to_string();
		assert!(text.contains("proj-front.container"), "{text}");
		assert!(!text.contains(".container.d"), "{text}");
	});
}

#[test]
fn a_pipe_that_is_not_the_projects_is_left_alone() {
	with_env(|root| {
		let dir = unit_dir(root);
		mkfifo(&dir.join("other-front.container"));
		std::fs::write(
			dir.join("proj-web.container"),
			"# podup-owner: proj\n[Container]\nImage=nginx\n",
		)
		.unwrap();
		let found =
			within_budget(|| installed_units("proj")).expect("a sibling's pipe is no concern");
		assert_eq!(found.len(), 1);
	});
}

#[test]
fn uninstall_stops_at_a_pipe_and_removes_nothing() {
	with_env(|root| {
		let dir = unit_dir(root);
		let unit = dir.join("proj-web.container");
		std::fs::write(&unit, "# podup-owner: proj\n[Container]\nImage=nginx\n").unwrap();
		mkfifo(&dir.join("proj-front.container"));
		let err = within_budget(|| uninstall_quadlet(&FakeCtl::new(), "proj"))
			.expect_err("uninstall must stop");
		assert!(err.to_string().contains("proj-front.container"), "{err}");
		assert!(unit.exists(), "the project's own unit was removed anyway");
	});
}

#[test]
fn status_stops_at_a_pipe() {
	with_env(|root| {
		let dir = unit_dir(root);
		std::fs::write(
			dir.join("proj-web.container"),
			"# podup-owner: proj\n[Container]\nImage=nginx\n",
		)
		.unwrap();
		mkfifo(&dir.join("proj-front.container"));
		let err = within_budget(|| collect_status(&FakeCtl::new(), "proj"))
			.map(|_| ())
			.expect_err("status must not report a list it could not read");
		assert!(err.to_string().contains("proj-front.container"), "{err}");
	});
}

/// The service-mode unit sits in another directory with the same exposure. A
/// pipe at its path is read as a unit that cannot be understood, which is
/// refused, and as no owner at all, not waited on.
#[test]
fn a_pipe_at_the_service_unit_path_is_not_waited_on() {
	with_env(|_root| {
		let path = super::super::unit_files::paths::unit_path("proj");
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		mkfifo(&path);
		let mode =
			within_budget(|| super::super::install::conflict::installed_single_unit_mode("proj"));
		assert!(
			matches!(
				mode,
				Some(super::super::install::conflict::DetectedSingleUnitMode::Unknown)
			),
			"a pipe must read as a unit that cannot be understood: {mode:?}"
		);
		let owner = within_budget(move || super::super::unit_files::owner::owner_of_file(&path));
		assert!(owner.is_none(), "a pipe has no owner");
	});
}
