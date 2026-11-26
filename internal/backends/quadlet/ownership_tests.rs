//! The installed units must not replace or remove a container they do not own.

use std::path::Path;

use super::{drop_in, generate_at, generate_for_autostart, pod_drop_in, QuadletOutput};
use crate::parse_str;

const BASE: &str = "/srv/app";

fn installed(yaml: &str, project: &str) -> QuadletOutput {
	generate_for_autostart(&parse_str(yaml).unwrap(), project, Path::new(BASE))
}

fn lines<'a>(out: &'a QuadletOutput, filename: &str, prefix: &str) -> Vec<&'a str> {
	let unit = out
		.units
		.iter()
		.find(|u| u.filename == filename)
		.unwrap_or_else(|| panic!("no unit named {filename}"));
	unit.contents
		.lines()
		.filter(|l| l.starts_with(prefix))
		.collect()
}

#[test]
fn an_installed_container_never_replaces_by_name_and_records_its_id() {
	let out = installed("services:\n  web:\n    image: nginx\n", "proj");
	assert_eq!(
		lines(&out, "proj-web.container", "PodmanArgs="),
		vec![
			"PodmanArgs=--replace=false",
			"PodmanArgs=--cidfile=%t/%N.cid",
		]
	);
	assert_eq!(
		lines(&out, "proj-web.container", "ExecStartPre="),
		vec![
			"ExecStartPre=-/usr/bin/podman rm --force --ignore --filter name=^proj-web$$ \
			 --filter label=podup.project=proj"
		]
	);
}

#[test]
fn replace_false_comes_after_every_other_podman_argument() {
	let out = installed(
		"services:\n  web:\n    image: nginx\n    cpu_shares: 512\n",
		"proj",
	);
	let args = lines(&out, "proj-web.container", "PodmanArgs=");
	assert_eq!(args.len(), 3, "{args:?}");
	assert_eq!(args[1], "PodmanArgs=--replace=false", "{args:?}");
}

#[test]
fn the_leftover_filter_is_exact_for_a_name_with_dots() {
	let out = installed(
		"services:\n  web:\n    image: nginx\n    container_name: a.b-c_d\n",
		"proj",
	);
	assert_eq!(
		lines(&out, "proj-web.container", "ExecStartPre="),
		vec![
			"ExecStartPre=-/usr/bin/podman rm --force --ignore --filter name=^a\\\\.b-c_d$$ \
			 --filter label=podup.project=proj"
		]
	);
}

#[test]
fn two_projects_that_name_the_same_container_filter_by_their_own_label() {
	let first = installed("services:\n  x-web:\n    image: nginx\n", "app");
	let second = installed("services:\n  web:\n    image: nginx\n", "app-x");
	for (out, project) in [(&first, "app"), (&second, "app-x")] {
		assert_eq!(
			lines(out, "app-x-web.container", "ExecStartPre="),
			vec![format!(
				"ExecStartPre=-/usr/bin/podman rm --force --ignore --filter name=^app-x-web$$ \
				 --filter label=podup.project={project}"
			)]
		);
	}
}

#[test]
fn a_name_podman_refuses_gets_no_removal_line() {
	let out = installed(
		"services:\n  web:\n    image: nginx\n    container_name: \"a b|c\"\n",
		"proj",
	);
	assert!(
		lines(&out, "proj-web.container", "ExecStartPre=").is_empty(),
		"a name that is not a container name must not reach a command line"
	);
}

#[test]
fn the_exported_units_carry_none_of_it() {
	let file = parse_str("services:\n  web:\n    image: nginx\n").unwrap();
	let out = generate_at(&file, "proj", Path::new(BASE));
	let unit = out
		.units
		.iter()
		.find(|u| u.filename == "proj-web.container")
		.unwrap();
	for needle in ["--replace=false", "--cidfile", "ExecStartPre="] {
		assert!(
			!unit.contents.contains(needle),
			"{needle}: {}",
			unit.contents
		);
	}
}

#[test]
fn the_drop_in_empties_both_cleanup_lists_then_removes_by_id_and_by_own_name() {
	assert_eq!(
		drop_in("proj", "[Container]\nContainerName=a.b\n"),
		"# podup-owner: proj\n\
		 [Service]\n\
		 ExecStop=\n\
		 ExecStop=-/usr/bin/podman rm -v -f -i --cidfile=%t/%N.cid\n\
		 ExecStop=/usr/bin/podman rm -v -f -i --filter name=^a\\\\.b$$ --filter label=podup.project=proj\n\
		 ExecStopPost=\n\
		 ExecStopPost=-/usr/bin/podman rm -v -f -i --cidfile=%t/%N.cid\n\
		 ExecStopPost=-/usr/bin/podman rm -v -f -i --filter name=^a\\\\.b$$ --filter label=podup.project=proj\n"
	);
}

#[test]
fn the_drop_in_of_a_unit_without_a_usable_name_removes_by_id_alone() {
	for unit in ["[Container]\nImage=x\n", "[Container]\nContainerName=a b\n"] {
		let text = drop_in("proj", unit);
		// The one command is the last one, so its failure is reported.
		assert!(
			text.contains("ExecStop=\nExecStop=/usr/bin/podman rm -v -f -i --cidfile="),
			"{text}"
		);
		assert!(
			text.contains("ExecStopPost=\nExecStopPost=-/usr/bin/podman rm -v -f -i --cidfile="),
			"{text}"
		);
		assert!(!text.contains("--filter"), "{text}");
	}
}

/// What the real Quadlet generator makes of an installed unit: the start ends in the
/// flags that stop it replacing by name and record the ID, the removal of this project's
/// leftover comes first, and the cleanup the drop-in replaces is the one by name, run by
/// the Podman the drop-in names.
#[cfg(unix)]
#[test]
fn the_generator_writes_the_start_the_drop_in_is_written_against() {
	use crate::backends::quadlet::tests::generator::generator;
	let Some(generator) = generator() else {
		return;
	};
	let out = installed("services:\n  web:\n    image: nginx\n", "proj");
	let dir = tempfile::tempdir().unwrap();
	crate::quadlet::write_units(dir.path(), &out.units).unwrap();
	let run = std::process::Command::new(&generator)
		.args(["-dryrun", "-user"])
		.env("QUADLET_UNIT_DIRS", dir.path())
		.env_remove("PODMAN")
		.output()
		.unwrap();
	let text = String::from_utf8_lossy(&run.stdout).into_owned();
	let service = text
		.split("---proj-web.service---")
		.nth(1)
		.unwrap_or_else(|| panic!("no proj-web.service in {text}"))
		.split("\n---")
		.next()
		.unwrap();
	let line = |key: &str| {
		service
			.lines()
			.filter(|l| l.starts_with(key))
			.collect::<Vec<_>>()
	};
	let start = line("ExecStart=");
	assert_eq!(start.len(), 1, "{service}");
	let replaces: Vec<&str> = start[0]
		.split(' ')
		.filter(|w| w.starts_with("--replace"))
		.collect();
	assert_eq!(replaces, ["--replace", "--replace=false"], "{}", start[0]);
	assert!(start[0].contains(" --cidfile=%t/%N.cid "), "{}", start[0]);
	assert_eq!(
		line("ExecStartPre="),
		[
			"ExecStartPre=-/usr/bin/podman rm --force --ignore --filter name=^proj-web$$ \
			 --filter label=podup.project=proj"
		]
	);
	for (key, quadlet) in [
		("ExecStop=", "ExecStop=/usr/bin/podman rm -v -f -i proj-web"),
		(
			"ExecStopPost=",
			"ExecStopPost=-/usr/bin/podman rm -v -f -i proj-web",
		),
	] {
		assert_eq!(line(key), [quadlet], "{service}");
		let own = drop_in("proj", &out.units.last().unwrap().contents);
		assert!(
			own.contains(&format!("{key}\n{key}-/usr/bin/podman rm ")),
			"{own}"
		);
	}
}

const POD: &str = "x-podman-pod: true\nservices:\n  web:\n    image: nginx\n";

/// The shell command the pod lines run, as the unit writes it.
fn on_own_pod(verb: &str) -> String {
	let guard = if verb == "rm --force" {
		"members=$$(/usr/bin/podman ps -aq --filter pod=$$id \
		 --filter label!=podup.project=proj) || exit $$?; \
		 if [ -n \"$$members\" ]; then echo \"podup: refusing pod cleanup for $$id: \
		 containers without podup.project=proj: $$members\" >&2; exit 1; fi; "
	} else {
		""
	};
	format!(
		"/bin/sh -c 'ids=$$(/usr/bin/podman pod ps -q --no-trunc --filter name=^proj$$ \
		 --filter label=podup.project=proj) || exit $$?; \
		 for id in $$ids; do {guard}/usr/bin/podman pod {verb} $$id || exit $$?; done'"
	)
}

#[test]
fn an_installed_pod_never_replaces_by_name_and_records_its_id() {
	let out = installed(POD, "proj");
	let args = lines(&out, "proj.pod", "PodmanArgs=");
	assert_eq!(
		&args[args.len() - 2..],
		[
			"PodmanArgs=--replace=false",
			"PodmanArgs=--pod-id-file=%t/%N.pod-id",
		]
	);
	assert_eq!(
		lines(&out, "proj.pod", "ExecStartPre="),
		[format!("ExecStartPre={}", on_own_pod("rm --force"))]
	);
}

#[test]
fn the_exported_pod_carries_none_of_it() {
	let file = parse_str(POD).unwrap();
	let out = generate_at(&file, "proj", Path::new(BASE));
	let unit = out.units.iter().find(|u| u.filename == "proj.pod").unwrap();
	for needle in ["--replace=false", "--pod-id-file", "ExecStartPre="] {
		assert!(
			!unit.contents.contains(needle),
			"{needle}: {}",
			unit.contents
		);
	}
}

#[test]
fn the_pod_drop_in_stops_by_id_and_checks_members_before_removal() {
	assert_eq!(
		pod_drop_in("proj", "[Pod]\nPodName=proj\n"),
		format!(
			"# podup-owner: proj\n\
			 [Service]\n\
			 ExecStop=\n\
			 ExecStop=-/usr/bin/podman pod stop --ignore --time=10 --pod-id-file=%t/%N.pod-id\n\
			 ExecStop={}\n\
			 ExecStopPost=\n\
			 ExecStopPost={}\n",
			on_own_pod("stop --time=10"),
			on_own_pod("rm --force")
		)
	);
}

/// What the real generator makes of an installed pod: this project's leftover is removed
/// before the creation, which ends in the flags that stop it replacing by name, and the
/// stop and removal the drop-in replaces are the ones by name.
#[cfg(unix)]
#[test]
fn the_generator_writes_the_pod_start_the_drop_in_is_written_against() {
	use crate::backends::quadlet::tests::generator::generator;
	let Some(generator) = generator() else {
		return;
	};
	let out = installed(POD, "proj");
	let dir = tempfile::tempdir().unwrap();
	crate::quadlet::write_units(dir.path(), &out.units).unwrap();
	let run = std::process::Command::new(&generator)
		.args(["-dryrun", "-user"])
		.env("QUADLET_UNIT_DIRS", dir.path())
		.env_remove("PODMAN")
		.output()
		.unwrap();
	let text = String::from_utf8_lossy(&run.stdout).into_owned();
	let service = text
		.split("---proj-pod.service---")
		.nth(1)
		.unwrap_or_else(|| panic!("no proj-pod.service in {text}"))
		.split("\n---")
		.next()
		.unwrap();
	let line = |key: &str| {
		service
			.lines()
			.filter(|l| l.starts_with(key))
			.collect::<Vec<_>>()
	};
	let before = line("ExecStartPre=");
	assert_eq!(before.len(), 2, "{service}");
	assert!(before[0].contains("pod ps -q"), "{service}");
	let create = before[1];
	assert!(create.contains(" pod create "), "{create}");
	let replaces: Vec<&str> = create
		.split(' ')
		.filter(|w| w.starts_with("--replace"))
		.collect();
	assert_eq!(replaces, ["--replace", "--replace=false"], "{create}");
	assert!(create.contains("--pod-id-file=%t/%N.pod-id"), "{create}");
	assert!(
		line("ExecStop=")[0].ends_with(" pod stop --ignore --time=10 proj"),
		"{service}"
	);
	assert!(
		line("ExecStopPost=")[0].ends_with(" pod rm --ignore --force proj"),
		"{service}"
	);
}

#[cfg(unix)]
#[path = "ownership_shell_tests.rs"]
mod shell;
