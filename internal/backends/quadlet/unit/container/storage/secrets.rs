//! The secrets a container mounts.

use indexmap::IndexMap;

use crate::compose::types::{SecretConfig, Service};

use super::super::{is_inline_secret, render_secret, Section};

/// Emit one `Secret=` line per `secrets:` entry, warning about an inline secret
/// Quadlet does not create.
pub(in super::super) fn emit_secrets(
	name: &str,
	project: &str,
	service: &Service,
	secrets: &IndexMap<String, SecretConfig>,
	container: &mut Section,
	warnings: &mut Vec<String>,
) {
	for secret in &service.secrets {
		// An inline (`content:`/`environment:`) secret is created by `up` under the
		// project-scoped name `{project}_secret_{name}`; reference that here so the
		// generated unit points at the secret `up` would create, not an unscoped
		// (possibly colliding or non-existent) host secret. Quadlet does not create
		// the secret itself, so warn the operator to provision it first.
		let source = secret.source();
		if is_inline_secret(secrets.get(source)) {
			warnings.push(format!(
				"{name}: inline secret {source:?} is referenced but Quadlet does not \
				 create it; provision the project-scoped secret \"{project}_secret_{source}\" \
				 first (e.g. via `podup up`)"
			));
		}
		container.add("Secret", render_secret(secret, project, secrets));
	}
}
