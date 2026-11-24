//! What the export refuses about the secrets and configs a service mounts.
//!
//! `Secret=` is a comma-separated list of `key=value` options, so a separator in
//! a name, a target or an owner would move the split, and the unit writer used to
//! drop such characters, which silently mounted the thing somewhere `up` does not.
//! A config is held to the strict set (the separators, blanks, quotes and control
//! characters); a secret keeps the set it has always had.

use crate::compose::types::{ComposeFile, ServiceConfigRef};

use super::Def;
use crate::error::{sanitize_name, ComposeError, Result};

/// A character a `Secret=` option cannot carry as it is.
fn is_separator(c: char) -> bool {
	c == ',' || c == '=' || c.is_control()
}

/// The strict set, for configs: also blanks and quotes.
fn is_strict_separator(c: char) -> bool {
	is_separator(c) || c.is_whitespace() || c == '"' || c == '\''
}

fn refuse(service: &str, what: &str, field: &str) -> ComposeError {
	ComposeError::Unsupported(format!(
		"service '{}': a {what} {field} with a comma, an equals sign, a blank, a quote or a control \
		 character cannot be written in a Quadlet unit; rename it",
		sanitize_name(service)
	))
}

/// Check every config reference of every service, the explicit modes of secrets
/// and configs, and the `name:` of an external secret or config.
pub(crate) fn validate(file: &ComposeFile) -> Result<()> {
	for (service, svc) in &file.services {
		for config in &svc.configs {
			let (source, target, uid, gid, mode) = match config {
				ServiceConfigRef::Short(name) => (name.as_str(), None, None, None, None),
				ServiceConfigRef::Long {
					source,
					target,
					uid,
					gid,
					mode,
				} => (
					source.as_str(),
					target.as_deref(),
					uid.as_deref(),
					gid.as_deref(),
					*mode,
				),
			};
			for (field, value) in [
				("source", Some(source)),
				("target", target),
				("uid", uid),
				("gid", gid),
			] {
				if value.is_some_and(|v| v.chars().any(is_strict_separator)) {
					return Err(refuse(service, "config", field));
				}
			}
			if let Some(mode) = mode {
				crate::engine::reject_dangerous_secret_mode(
					mode,
					&format!(
						"config {:?} of service {:?}",
						sanitize_name(source),
						sanitize_name(service)
					),
				)?;
			}
		}
		for secret in &svc.secrets {
			if let crate::compose::types::ServiceSecretRef::Long {
				source,
				mode: Some(mode),
				..
			} = secret
			{
				crate::engine::reject_dangerous_secret_mode(
					*mode,
					&format!(
						"secret {:?} of service {:?}",
						sanitize_name(source),
						sanitize_name(service)
					),
				)?;
			}
		}
	}
	// A definition with more than one source is not one Compose allows, and `up`
	// would take a different one than the export: refuse it, as the ones a service
	// mounts are the ones that are read.
	for (service, svc) in &file.services {
		let secrets = svc.secrets.iter().map(|r| {
			(
				"secret",
				r.source(),
				file.secrets.get(r.source()).map(Def::from),
			)
		});
		let configs = svc.configs.iter().map(|r| {
			(
				"config",
				r.source(),
				file.configs.get(r.source()).map(Def::from),
			)
		});
		for (word, name, def) in secrets.chain(configs) {
			if def.is_some_and(|d| !d.external && d.sources() > 1) {
				return Err(ComposeError::Unsupported(format!(
					"service '{}': {}",
					sanitize_name(service),
					super::more_than_one_source(word, &sanitize_name(name))
				)));
			}
		}
	}
	for (key, def) in &file.configs {
		if def.external == Some(true)
			&& def
				.name
				.as_deref()
				.is_some_and(|n| n.chars().any(is_strict_separator))
		{
			return Err(refuse(key, "external config", "name"));
		}
	}
	for (key, def) in &file.secrets {
		if def.external == Some(true)
			&& def
				.name
				.as_deref()
				.is_some_and(|n| n.chars().any(is_separator))
		{
			return Err(ComposeError::Unsupported(format!(
				"secret '{}': an external name with a comma, an equals sign or a control character \
				 cannot be written in a Quadlet unit; rename it",
				sanitize_name(key)
			)));
		}
	}
	Ok(())
}
