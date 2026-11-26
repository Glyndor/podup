//! The secrets and configs a container mounts.

use indexmap::IndexMap;

use crate::compose::types::{
	ConfigConfig, SecretConfig, Service, ServiceConfigRef, ServiceSecretRef,
};

use crate::backends::quadlet::secrets_unit::{self, Def, Kind, Source};

use super::super::{is_inline_secret, render_secret, secret_field, Section};

/// Emit one `Secret=` line per `secrets:` entry and one per `configs:` entry.
///
/// A secret or config whose source is a file is created by the project's secrets
/// unit; the container then has `Requires=` and `After=` on it, so a file that
/// cannot be read keeps the container from starting. The others are the
/// operator's to create (see [`secrets_unit`]); the container still names them,
/// so it does not start without them. One that `up` would refuse is in the unit
/// too, as a command that fails, so the container does not start at all.
#[allow(clippy::too_many_arguments)]
#[inline(never)]
pub(in super::super) fn emit_secrets(
	name: &str,
	project: &str,
	service: &Service,
	secrets: &IndexMap<String, SecretConfig>,
	configs: &IndexMap<String, ConfigConfig>,
	secrets_unit_written: bool,
	base_dir: &std::path::Path,
	unit: &mut Section,
	container: &mut Section,
	warnings: &mut Vec<String>,
) {
	let mut needs_unit = false;
	// What a mounted name is to the export, and the mode its mount gets from it.
	let mut mode_of = |kind: Kind, source: &str, def: Option<Def<'_>>, explicit: Option<u32>| {
		let source = secrets_unit::classify(kind, project, source, def, base_dir);
		needs_unit |= matches!(source, Source::File { .. } | Source::Refused { .. });
		derived_mode(&source, secrets_unit_written, explicit)
	};
	for secret in &service.secrets {
		let derived = mode_of(
			Kind::Secret,
			secret.source(),
			secrets.get(secret.source()).map(Def::from),
			explicit_secret_mode(secret),
		);
		// Without the secrets unit (the units `autostart --mode quadlet`
		// installs) nothing creates the secret: say so, as the export always has.
		if !secrets_unit_written && is_inline_secret(secrets.get(secret.source())) {
			warnings.push(format!(
				"{name}: secret {:?} is referenced but Quadlet does not \
				 create it; provision the project-scoped secret \"{project}_secret_{}\" \
				 first (e.g. via `podup up`)",
				secret.source(),
				secret.source()
			));
		}
		let mut line = render_secret(secret, project, secrets);
		if let Some(mode) = derived {
			line.push_str(&format!(",mode={mode:o}"));
		}
		container.add("Secret", line);
	}
	// A config is a secret `up` made; in the units without the secrets unit it
	// stays unmounted, as it has been, because a stack that starts without it
	// must keep starting.
	if !secrets_unit_written {
		return;
	}
	for config in &service.configs {
		let derived = mode_of(
			Kind::Config,
			config.source(),
			configs.get(config.source()).map(Def::from),
			explicit_config_mode(config),
		);
		if let Some(line) = render_config(config, project, configs, derived) {
			container.add("Secret", line);
		}
	}
	if needs_unit && secrets_unit_written {
		let service = secrets_unit::service_name(project);
		unit.add("Requires", service.clone());
		unit.add("After", service);
	}
}

/// The `Secret=` value for a mounted config: the secret `up` makes of it, as a
/// file at `target` (`/<name>` unless the file says another), with the
/// ownership and mode the service gave it.
fn render_config(
	config: &ServiceConfigRef,
	project: &str,
	configs: &IndexMap<String, ConfigConfig>,
	derived_mode: Option<u32>,
) -> Option<String> {
	let name = config.source();
	let def = configs.get(name);
	let podman_name = match def {
		Some(d) if d.external == Some(true) => d.name.clone().unwrap_or_else(|| name.to_string()),
		Some(_) => crate::engine::scoped_name(project, "config", name),
		None => return None,
	};
	let (target, uid, gid, mode) = match config {
		ServiceConfigRef::Short(_) => (None, None, None, None),
		ServiceConfigRef::Long {
			target,
			uid,
			gid,
			mode,
			..
		} => (target.as_deref(), uid.as_deref(), gid.as_deref(), *mode),
	};
	let field = secret_field;
	let mut value = format!(
		"{},type=mount,target={}",
		field(&podman_name),
		field(&target.map_or_else(|| format!("/{name}"), str::to_string))
	);
	if let Some(uid) = uid {
		value.push_str(&format!(",uid={}", field(uid)));
	}
	if let Some(gid) = gid {
		value.push_str(&format!(",gid={}", field(gid)));
	}
	if let Some(mode) = mode.or(derived_mode) {
		value.push_str(&format!(",mode={mode:o}"));
	}
	Some(value)
}

/// The mode a mounted file-sourced secret or config gets when the service gives
/// none: what `up` gives it, the permission bits of the host file (measured on
/// Podman 6.2.0: a 0600 file is 0600 in the container under `up`, and 0444 when
/// `Secret=` says nothing). Never more open than that, whoever creates the secret:
/// the units `autostart --mode quadlet` installs mount one that `up` made.
fn derived_mode(source: &Source, written: bool, explicit: Option<u32>) -> Option<u32> {
	match source {
		Source::File { mode, .. } if explicit.is_none() => Some(*mode),
		// A name that was refused is mounted as tight as can be, and the unit that
		// the container requires fails for it, so it does not start.
		Source::Refused { .. } if written && explicit.is_none() => Some(0o400),
		_ => None,
	}
}

fn explicit_secret_mode(secret: &ServiceSecretRef) -> Option<u32> {
	match secret {
		ServiceSecretRef::Short(_) => None,
		ServiceSecretRef::Long { mode, .. } => *mode,
	}
}

fn explicit_config_mode(config: &ServiceConfigRef) -> Option<u32> {
	match config {
		ServiceConfigRef::Short(_) => None,
		ServiceConfigRef::Long { mode, .. } => *mode,
	}
}
