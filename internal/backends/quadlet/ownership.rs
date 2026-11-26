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
//! `generate quadlet` writes the units as Quadlet reads them and carries none of this: its output is
//! files in a directory of the user's choosing, with no place for a drop-in.

use super::render::Section;
use super::unit::owner_marker;

/// The name of the drop-in, in the `<service>.service.d` directory of each container service.
pub(crate) const DROP_IN_NAME: &str = "50-podup-ownership.conf";

/// The file the start writes the container ID to: the user's runtime directory, one per unit. The
/// two specifiers are systemd's, expanded when the service runs.
const CIDFILE: &str = "%t/%N.cid";

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
	container.add("PodmanArgs", format!("--cidfile={CIDFILE}"));
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
	let mut commands = vec![format!("{PODMAN} rm -v -f -i --cidfile={CIDFILE}")];
	if let Some(pattern) = pattern {
		commands.push(format!(
			"{PODMAN} rm -v -f -i {}",
			own_container(project, &pattern)
		));
	}
	let mut text = owner_marker(project);
	text.push_str("[Service]\n");
	for key in ["ExecStop", "ExecStopPost"] {
		text.push_str(&format!("{key}=\n"));
		for (position, command) in commands.iter().enumerate() {
			let reported = key == "ExecStop" && position + 1 == commands.len();
			let prefix = if reported { "" } else { "-" };
			text.push_str(&format!("{key}={prefix}{command}\n"));
		}
	}
	text
}
