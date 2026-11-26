//! What keeps the units `autostart --mode quadlet` installs from replacing or removing a container
//! of another project.
//!
//! The units Quadlet generates start the container with `podman run --replace` and clean it up with
//! `podman rm --force` by name, so a name that another container holds costs that container its
//! writable layer. An installed unit changes three things:
//!
//! * `--replace=false`, so a name that is taken stops the start instead of removing the holder (the
//!   last `--replace` of the argument line wins);
//! * before the start, the leftover of this project's own container is removed, by name and by the
//!   `podup.project` label together, which leaves a container of anyone else where it is;
//! * a drop-in for the generated service replaces the cleanup by name with a cleanup of the ID the
//!   start wrote to `--cidfile`, in the user's runtime directory, followed by a cleanup by name that
//!   takes only a container carrying this project's label. The second is what stops, with its stop
//!   signal and timeout, a container that has no recorded ID: one an older podup started and the
//!   reinstall left running, or the one of a unit without the lines above that the drop-in outlives.
//!   A drop-in is the only place that works: Quadlet writes its own `ExecStop=` lines after the
//!   ones of the unit's `[Service]` section, and systemd reads the drop-ins of a service after the
//!   unit.
//!
//! The `.pod` unit of a pod project gets the same three things: its creation says
//! `--replace=false` and records the pod ID, the leftover pod of this project is removed before it,
//! and a drop-in replaces the stop and the removal by name. `podman pod rm` takes no filter, so the
//! commands that act on the project's own pod ask `podman pod ps` for it in a shell.
//!
//! `generate quadlet` writes the units as Quadlet reads them and carries none of this: its output is
//! files in a directory of the user's choosing, with no place for a drop-in.

use super::render::Section;
use super::unit::owner_marker;

/// The name of the drop-in, in the `<service>.service.d` directory of each container service.
pub(crate) const DROP_IN_NAME: &str = "50-podup-ownership.conf";

/// Record the container ID in the user's runtime directory, one file per unit.
/// The two specifiers are systemd's, expanded when the service runs.
const CIDFILE: &str = "--cidfile=%t/%N.cid";

/// Record the pod ID in the user's runtime directory, one file per unit.
const POD_ID_FILE: &str = "--pod-id-file=%t/%N.pod-id";

/// The Podman the lines run: the path Quadlet writes into the units it generates, so the cleanup and
/// the start talk to the same one whatever `PATH` the user manager has.
const PODMAN: &str = "/usr/bin/podman";

/// The lines of an installed container unit that stop it from replacing a container it does not own.
pub(super) fn emit(
	project: &str,
	container_name: &str,
	container: &mut Section,
	service: &mut Section,
) {
	container.add("PodmanArgs", "--replace=false".to_string());
	container.add("PodmanArgs", CIDFILE.to_string());
	// A name Podman would refuse has no container to remove, and the start fails on it anyway.
	if let Some(pattern) = name_pattern(container_name) {
		service.add(
			"ExecStartPre",
			format!(
				"-{PODMAN} rm --force --ignore {}",
				own_container(project, &pattern)
			),
		);
	}
}

/// The `--filter name=` pattern that matches `name` and nothing else, as a systemd command line
/// writes it: the filter is a regular expression, so the one metacharacter a container name may
/// hold is escaped (`\\` reaches Podman as `\`), and `$$` reaches it as the `$` that ends it.
/// `None` for a name outside what Podman accepts.
fn name_pattern(name: &str) -> Option<String> {
	let mut chars = name.chars();
	let first_ok = chars.next().is_some_and(|c| c.is_ascii_alphanumeric());
	if !first_ok || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')) {
		return None;
	}
	Some(format!("^{}$$", name.replace('.', "\\\\.")))
}

/// The filters that name the container matching `pattern` only when it is `project`'s.
fn own_container(project: &str, pattern: &str) -> String {
	format!("--filter name={pattern} --filter label=podup.project={project}")
}

/// The drop-in of the container service whose unit holds `unit`: the cleanup Quadlet writes, by
/// name, is emptied and replaced by a cleanup of the container the unit started, then of the
/// container of that name that carries the project's label. Every line carries `-`, so none stops
/// the next, except the last of `ExecStop=`: a removal that really fails there fails the stop
/// instead of passing for one that worked. Neither removal fails when there is nothing to remove.
pub(crate) fn drop_in(project: &str, unit: &str) -> String {
	// The one `ContainerName=` line is the one this module's units are written with; a value of
	// the compose file cannot add a line.
	let pattern = unit
		.lines()
		.find_map(|line| line.strip_prefix("ContainerName="))
		.and_then(name_pattern);
	let by_name =
		pattern.map(|pattern| format!("{PODMAN} rm -v -f -i {}", own_container(project, &pattern)));
	let mut text = owner_marker(project);
	text.push_str("[Service]\n");
	for key in ["ExecStop", "ExecStopPost"] {
		command(&mut text, key, "", false);
		let ignored = key != "ExecStop";
		command(
			&mut text,
			key,
			"/usr/bin/podman rm -v -f -i --cidfile=%t/%N.cid",
			ignored || by_name.is_some(),
		);
		if let Some(by_name) = &by_name {
			command(&mut text, key, by_name, ignored);
		}
	}
	text
}

/// Append one systemd command, optionally ignoring its exit status.
fn command(text: &mut String, key: &str, value: &str, ignored: bool) {
	text.push_str(key);
	text.push('=');
	if ignored {
		text.push('-');
	}
	text.push_str(value);
	text.push('\n');
}

/// The lines of an installed pod unit that stop it from replacing a pod it does not own.
pub(super) fn emit_pod(project: &str, pod_name: &str, pod: &mut Section, service: &mut Section) {
	pod.add("PodmanArgs", "--replace=false".to_string());
	pod.add("PodmanArgs", POD_ID_FILE.to_string());
	if let Some(pattern) = name_pattern(pod_name) {
		service.add("ExecStartPre", on_own_pod(project, &pattern, "rm --force"));
	}
}

/// A shell that runs `podman pod <verb>` on the pod matching `pattern` only when it carries the
/// label of `project`. `$$` reaches the shell as `$`.
fn on_own_pod(project: &str, pattern: &str, verb: &str) -> String {
	let guard = if verb == "rm --force" {
		format!(
			"members=$$({PODMAN} ps -aq --filter pod=$$id \
			 --filter label!=podup.project={project}) || exit $$?; \
			 if [ -n \"$$members\" ]; then echo \"podup: refusing pod cleanup: \
			 containers without podup.project={project}\" >&2; exit 1; fi; "
		)
	} else {
		String::new()
	};
	format!(
		"/bin/sh -c 'ids=$$({PODMAN} pod ps -q --no-trunc --filter name={pattern} \
		 --filter label=podup.project={project}) || exit $$?; \
		 for id in $$ids; do {guard}{PODMAN} pod {verb} $$id || exit $$?; done'"
	)
}

/// The drop-in of the pod service whose unit holds `unit`: the stop and the removal Quadlet writes,
/// by name, are emptied. Removal uses the label-filtered lookup and refuses any unlabelled member,
/// including an infra container. This check is not atomic with removal; see `docs/autostart.md`.
pub(crate) fn pod_drop_in(project: &str, unit: &str) -> String {
	let pattern = unit
		.lines()
		.find_map(|line| line.strip_prefix("PodName="))
		.and_then(name_pattern);
	let mut text = owner_marker(project);
	text.push_str("[Service]\n");
	text.push_str(
		"ExecStop=\nExecStop=-/usr/bin/podman pod stop --ignore --time=10 --pod-id-file=%t/%N.pod-id\n",
	);
	if let Some(pattern) = &pattern {
		command(
			&mut text,
			"ExecStop",
			&on_own_pod(project, pattern, "stop --time=10"),
			false,
		);
	}
	let cleanup = pattern.map_or_else(
		|| "/bin/false".to_string(),
		|pattern| on_own_pod(project, &pattern, "rm --force"),
	);
	command(&mut text, "ExecStopPost", "", false);
	command(&mut text, "ExecStopPost", &cleanup, false);
	text
}
