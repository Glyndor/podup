//! The drop-in that goes with each installed container service: written with the units, refused when
//! it is someone else's, and removed with its unit.

use super::{install_quadlet, uninstall_quadlet, FakeCtl, ScriptedCtl, IMG};
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

#[test]
fn the_drop_in_is_in_place_before_systemd_reloads() {
	super::with_env(|root| {
		let path = drop_in(root, "proj-web.service");
		let unit = root.join("containers/systemd/proj-web.container");
		let sc = ScriptedCtl::new(move |args| {
			i32::from(args == ["daemon-reload"] && !(path.is_file() && unit.is_file()))
		});
		install_quadlet(
			&sc,
			&parse_str(IMG).unwrap(),
			"proj",
			Path::new(BASE),
			true,
			false,
		)
		.unwrap();
		assert_eq!(sc.log(), vec![vec!["daemon-reload".to_string()]]);
	});
}

#[test]
fn an_unmarked_file_at_the_drop_in_is_not_overwritten_and_nothing_is_written() {
	super::with_env(|root| {
		let path = drop_in(root, "proj-web.service");
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		std::fs::write(&path, "[Service]\n").unwrap();
		let err = install(IMG, "proj", false).err().unwrap();
		assert!(err.to_string().contains("ownership marker"), "{err}");
		assert_eq!(std::fs::read_to_string(&path).unwrap(), "[Service]\n");
		assert!(!root.join("containers/systemd/proj-web.container").exists());
	});
}

#[test]
fn a_directory_others_can_write_is_refused_before_anything_is_written() {
	use std::os::unix::fs::PermissionsExt;
	for which in ["systemd/user", "containers/systemd"] {
		super::with_env(|root| {
			let dir = root.join(which);
			std::fs::create_dir_all(&dir).unwrap();
			std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
			let err = install(IMG, "proj", false).err().unwrap();
			assert!(
				err.to_string().contains("writable by others"),
				"{which}: {err}"
			);
			assert!(
				!drop_in(root, "proj-web.service").exists(),
				"{which}: a drop-in was written"
			);
			assert!(
				!root.join("containers/systemd/proj-web.container").exists(),
				"{which}: a unit was written"
			);
		});
	}
}

#[test]
fn uninstall_goes_on_past_a_drop_in_it_cannot_prove_is_its_own() {
	for case in ["link", "unmarked"] {
		super::with_env(|root| {
			install(
				"services:\n  web:\n    image: nginx\n  db:\n    image: postgres\n",
				"proj",
				false,
			)
			.unwrap();
			let path = drop_in(root, "proj-db.service");
			std::fs::remove_file(&path).unwrap();
			if case == "link" {
				std::os::unix::fs::symlink(root.join("elsewhere"), &path).unwrap();
			} else {
				std::fs::write(&path, "[Service]\n").unwrap();
			}
			let sc = FakeCtl::new();
			let log = warnings_of(|| uninstall_quadlet(&sc, "proj").unwrap());
			let why = if case == "link" {
				"symbolic link"
			} else {
				"no podup ownership marker"
			};
			assert!(
				log.contains("proj-db.service.d") && log.contains(why),
				"{case}: {log}"
			);
			assert!(path.symlink_metadata().is_ok(), "{case}: it was removed");
			for unit in ["proj-web.container", "proj-db.container"] {
				assert!(
					!root.join("containers/systemd").join(unit).exists(),
					"{case}: {unit} stayed"
				);
			}
			assert!(!drop_in(root, "proj-web.service").exists(), "{case}");
			assert!(
				sc.log().contains(&vec!["daemon-reload".to_string()]),
				"{case}: {:?}",
				sc.log()
			);
		});
	}
}

/// What `run` logs.
fn warnings_of(run: impl FnOnce()) -> String {
	let bytes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
	let writer = bytes.clone();
	let subscriber = tracing_subscriber::fmt()
		.without_time()
		.with_ansi(false)
		.with_writer(move || Sink(writer.clone()))
		.finish();
	tracing::subscriber::with_default(subscriber, run);
	let text = bytes.lock().unwrap().clone();
	String::from_utf8(text).unwrap()
}

struct Sink(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Sink {
	fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
		self.0.lock().unwrap().extend_from_slice(bytes);
		Ok(bytes.len())
	}
	fn flush(&mut self) -> std::io::Result<()> {
		Ok(())
	}
}

#[test]
fn a_drop_in_directory_others_can_write_is_refused_before_anything_is_written() {
	use std::os::unix::fs::PermissionsExt;
	super::with_env(|root| {
		let path = drop_in(root, "proj-web.service");
		let dir = path.parent().unwrap();
		std::fs::create_dir_all(dir).unwrap();
		std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o777)).unwrap();
		let err = install(IMG, "proj", false).err().unwrap();
		assert!(err.to_string().contains("writable by others"), "{err}");
		assert!(!path.exists(), "a drop-in was written");
		assert!(!root.join("containers/systemd/proj-web.container").exists());
	});
}

const POD: &str = "x-podman-pod: true\nservices:\n  web:\n    image: nginx\n";

#[test]
fn a_pod_project_gets_a_drop_in_for_its_pod_and_uninstall_removes_both() {
	super::with_env(|root| {
		install(POD, "proj", false).unwrap();
		let pod = drop_in(root, "proj-pod.service");
		let web = drop_in(root, "proj-web.service");
		let text = std::fs::read_to_string(&pod).unwrap();
		assert!(text.starts_with("# podup-owner: proj\n"), "{text}");
		assert!(text.contains("pod rm"), "{text}");
		assert!(web.is_file());
		let sc = FakeCtl::new();
		uninstall_quadlet(&sc, "proj").unwrap();
		assert!(!pod.exists() && !web.exists());
		assert!(!root.join("containers/systemd/proj.pod").exists());
		let stops: Vec<_> = sc.log().into_iter().filter(|c| c[0] == "stop").collect();
		assert_eq!(
			stops,
			[["stop", "proj-web.service"], ["stop", "proj-pod.service"]],
		);
	});
}

#[test]
fn a_project_that_leaves_pod_mode_loses_the_pod_drop_in() {
	super::with_env(|root| {
		install(POD, "proj", false).unwrap();
		let sc = install(IMG, "proj", false).unwrap();
		assert!(sc
			.log()
			.contains(&vec!["stop".to_string(), "proj-pod.service".to_string()]));
		assert!(!root.join("containers/systemd/proj.pod").exists());
		assert!(!drop_in(root, "proj-pod.service").exists());
		assert!(drop_in(root, "proj-web.service").is_file());
	});
}

/// Another project's pod unit is left alone, whatever its name shares.
#[test]
fn uninstall_leaves_the_pod_of_another_project() {
	super::with_env(|root| {
		install(POD, "app-x", false).unwrap();
		uninstall_quadlet(&FakeCtl::new(), "app").unwrap();
		assert!(root.join("containers/systemd/app-x.pod").is_file());
		assert!(drop_in(root, "app-x-pod.service").is_file());
	});
}
