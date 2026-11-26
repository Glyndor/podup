//! The installed units must not replace or remove a container they do not own.

use std::path::Path;

use super::{drop_in, generate_at, generate_for_autostart, QuadletOutput};
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
			"ExecStartPre=-podman rm --force --ignore --filter name=^proj-web$$ \
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
			"ExecStartPre=-podman rm --force --ignore --filter name=^a\\\\.b-c_d$$ \
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
				"ExecStartPre=-podman rm --force --ignore --filter name=^app-x-web$$ \
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
fn the_drop_in_empties_both_cleanup_lists_and_removes_by_the_recorded_id() {
	assert_eq!(
		drop_in("proj"),
		"# podup-owner: proj\n\
		 [Service]\n\
		 ExecStop=\n\
		 ExecStop=podman rm -v -f -i --cidfile=%t/%N.cid\n\
		 ExecStopPost=\n\
		 ExecStopPost=-podman rm -v -f -i --cidfile=%t/%N.cid\n"
	);
}
