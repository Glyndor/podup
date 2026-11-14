use podup::compose::types::{ComposeFile, Service};

use super::{finding, Finding};

/// `security_opt` entry that switches off a confinement layer the runtime
/// applies by default: `seccomp=unconfined` drops the syscall filter,
/// `apparmor=unconfined` the AppArmor profile, and `label=disable` SELinux
/// separation. One finding per entry.
///
/// The entry is split the way the engine splits it, on the first `:` or `=`,
/// so both spellings match. A value that names a profile
/// (`seccomp=/etc/podman/strict.json`, `apparmor=my-profile`) or another
/// label option (`label=type:spc_t`) is a choice of confinement, not its
/// absence, and stays silent. An option the engine does not pass on, such as
/// `systempaths=unconfined`, is no layer this container loses, so it stays
/// silent too.
pub fn check_unconfined_security_opt(
	name: &str,
	service: &Service,
	_file: &ComposeFile,
) -> Vec<Finding> {
	let mut out = Vec::new();
	for opt in &service.security_opt {
		let Some(i) = opt.find([':', '=']) else {
			continue;
		};
		let (key, value) = (opt[..i].trim(), opt[i + 1..].trim());
		let layer = match (key, value) {
			("seccomp", "unconfined") => "the seccomp syscall filter",
			("apparmor", "unconfined") => "the AppArmor profile",
			("label", "disable") => "SELinux label separation",
			_ => continue,
		};
		out.push(finding(
			name,
			"unconfined_security_opt",
			&format!("security_opt: {opt} turns off {layer}"),
		));
	}
	out
}
