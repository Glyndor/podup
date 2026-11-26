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
//!   start wrote to `--cidfile`, in the user's runtime directory. A drop-in is the only place that
//!   works: Quadlet writes its own `ExecStop=` lines after the ones of the unit's `[Service]`
//!   section, and systemd reads the drop-ins of a service after the unit.
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
				"-podman rm --force --ignore --filter name={pattern} \
				 --filter label=podup.project={project}"
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

/// The drop-in of one container service of `project`: the cleanup Quadlet writes, by name, is
/// emptied and replaced by a cleanup of the container this unit started.
pub(crate) fn drop_in(project: &str) -> String {
	let mut text = owner_marker(project);
	text.push_str(&format!(
		"[Service]\n\
		 ExecStop=\n\
		 ExecStop=podman rm -v -f -i --cidfile={CIDFILE}\n\
		 ExecStopPost=\n\
		 ExecStopPost=-podman rm -v -f -i --cidfile={CIDFILE}\n"
	));
	text
}
