//! Name, path, link, and config-hash resolution for container creation.

use std::collections::HashMap;
use std::path::Path;

use crate::compose::types::{ComposeFile, Service};
use crate::env_file;
use crate::error::{ComposeError, Result};

/// Resolve a named-volume reference to the volume name `create_volumes`
/// produced: a custom `name:`, the raw name for an external volume, or the
/// `{project}_{name}` form. References not declared in the top-level `volumes:`
/// map (anonymous/implicit) are returned unchanged.
pub(super) fn resolve_volume_name(reference: &str, project: &str, file: &ComposeFile) -> String {
	match file.volumes.get(reference) {
		Some(cfg) => {
			if let Some(name) = cfg.as_ref().and_then(|c| c.name.as_deref()) {
				name.to_string()
			} else if cfg.as_ref().and_then(|c| c.external).unwrap_or(false) {
				reference.to_string()
			} else {
				format!("{project}_{reference}")
			}
		}
		None => reference.to_string(),
	}
}

/// Resolve a bind-mount source path: expand a leading `~`, then make a relative
/// path absolute against the project base directory. Absolute paths (including
/// staged secret/config files) are returned unchanged.
pub(crate) fn resolve_bind_source(src: &str, base_dir: &Path) -> String {
	if src.is_empty() {
		return src.to_string();
	}
	let expanded = if let Some(rest) = src.strip_prefix("~/") {
		// Join with the platform separator rather than hardcoding `/`, and look
		// up the home directory in a platform-correct way (USERPROFILE on
		// native Windows, where HOME is usually unset).
		match home_dir() {
			Some(home) => home.join(rest).to_string_lossy().into_owned(),
			None => src.to_string(),
		}
	} else if src == "~" {
		home_dir()
			.map(|h| h.to_string_lossy().into_owned())
			.unwrap_or_else(|| src.to_string())
	} else {
		src.to_string()
	};
	if Path::new(&expanded).is_absolute() {
		expanded
	} else {
		base_dir.join(&expanded).to_string_lossy().into_owned()
	}
}

/// The current user's home directory. Prefers `HOME` (set on Unix and most
/// shells), falling back to `USERPROFILE` for native Windows where `HOME` is
/// usually absent. Empty values are treated as unset.
fn home_dir() -> Option<std::path::PathBuf> {
	std::env::var_os("HOME")
		.or_else(|| std::env::var_os("USERPROFILE"))
		.filter(|v| !v.is_empty())
		.map(std::path::PathBuf::from)
}

/// Resolve a service's `links` to concrete container references.
///
/// A compose `links:` entry names a sibling service; it is rewritten to that
/// service's container name with the service name kept as the network alias
/// (`{container}:{alias}`), so the linked container is reachable by the compose
/// service name. `external_links` reference containers outside the project and
/// are passed through verbatim.
pub(super) fn resolve_links(service: &Service, file: &ComposeFile, project: &str) -> Vec<String> {
	let mut links: Vec<String> = service
		.links
		.iter()
		.map(|link| {
			let (target, alias) = link.split_once(':').unwrap_or((link, link));
			let container = file
				.services
				.get(target)
				.map(|svc| {
					svc.container_name
						.clone()
						.unwrap_or_else(|| format!("{project}-{target}"))
				})
				.unwrap_or_else(|| target.to_string());
			format!("{container}:{alias}")
		})
		.collect();
	links.extend(service.external_links.iter().cloned());
	links
}

/// Stable content hash of a service definition, stored as the
/// `podup.config-hash` label. On `up`, comparing this against the label on an
/// existing container tells podup whether the service configuration changed
/// and the container must be recreated, or is unchanged and can be left as is.
///
/// The resolved bytes of any inline `content:`/`environment:` secret or config
/// the service references are folded in, so rotating an inline value recreates
/// the container to pick it up. Previously these were live host bind-mounts, so
/// a re-`up` reflected the change without recreation; now they are point-in-time
/// Podman-native secrets, so the recreate must be driven by the hash. `file:`
/// sources stay live bind-mounts and `external:` sources are by-reference, so
/// neither needs to influence the hash.
pub(crate) fn config_hash(service: &Service, file: &ComposeFile) -> Result<String> {
	use sha2::{Digest, Sha256};
	let mut hasher = Sha256::new();
	// Canonicalise through `serde_json::Value` first: object keys are emitted in
	// sorted order, so map-typed fields (e.g. `storage_opt`) cannot reorder
	// between runs and flap the hash into a spurious recreate. Fail closed if
	// serialization fails (e.g. a non-scalar mapping key in an `x-` extension):
	// returning an empty/default hash would make distinct services hash
	// identically and silently suppress recreation and inline-secret rotation.
	let serialized = serde_json::to_value(service)
		.and_then(|v| serde_json::to_vec(&v))
		.map_err(|e| ComposeError::Unsupported(format!("cannot hash service config: {e}")))?;
	hasher.update(&serialized);
	for secret_ref in &service.secrets {
		if let Some(def) = file.secrets.get(secret_ref.source()) {
			hash_inline_payload(
				&mut hasher,
				b"secret",
				secret_ref.source(),
				def.content.as_deref(),
				def.environment.as_deref(),
			);
		}
	}
	for config_ref in &service.configs {
		if let Some(def) = file.configs.get(config_ref.source()) {
			hash_inline_payload(
				&mut hasher,
				b"config",
				config_ref.source(),
				def.content.as_deref(),
				def.environment.as_deref(),
			);
		}
	}
	Ok(hasher
		.finalize()
		.iter()
		.map(|b| format!("{b:02x}"))
		.collect())
}

/// Fold an inline secret/config's resolved bytes into the config hasher. Inline
/// `content:` contributes its literal bytes; `environment:` contributes the
/// current value of the named variable (empty if unset — `up` errors on a
/// genuinely missing var later). `file:`/`external:` sources contribute nothing.
fn hash_inline_payload(
	hasher: &mut sha2::Sha256,
	kind: &[u8],
	name: &str,
	content: Option<&str>,
	environment: Option<&str>,
) {
	use sha2::Digest;
	let payload = match (content, environment) {
		(Some(c), _) => Some(c.as_bytes().to_vec()),
		(None, Some(var)) => Some(std::env::var(var).unwrap_or_default().into_bytes()),
		(None, None) => None,
	};
	if let Some(payload) = payload {
		hasher.update(kind);
		hasher.update(name.as_bytes());
		// Length-prefix so (name, payload) pairs cannot be confused across refs.
		hasher.update((payload.len() as u64).to_le_bytes());
		hasher.update(&payload);
	}
}

pub(super) fn build_env(service: &Service, base_dir: &Path) -> Result<Vec<String>> {
	let entries = service.env_file.to_entries();
	let env_file_vars = if !entries.is_empty() {
		env_file::load_env_file_entries(&entries, base_dir)?
	} else {
		HashMap::new()
	};
	Ok(env_file::merge_env(
		service.environment.to_map(),
		env_file_vars,
	))
}

#[cfg(test)]
mod tests {
	use super::{config_hash, resolve_links, resolve_volume_name};
	use crate::parse_str;

	#[test]
	fn links_resolve_to_container_names_external_links_verbatim() {
		let file = parse_str(
			"services:\n  db:\n    image: x\n  web:\n    image: x\n    links:\n      - db\n      - db:primary\n    external_links:\n      - legacy_db:db\n",
		)
		.unwrap();
		let links = resolve_links(&file.services["web"], &file, "proj");
		assert!(links.contains(&"proj-db:db".to_string()));
		assert!(links.contains(&"proj-db:primary".to_string()));
		assert!(links.contains(&"legacy_db:db".to_string()));
	}

	#[test]
	fn links_honour_custom_container_name() {
		let file = parse_str(
			"services:\n  db:\n    image: x\n    container_name: my-db\n  web:\n    image: x\n    links:\n      - db\n",
		)
		.unwrap();
		let links = resolve_links(&file.services["web"], &file, "proj");
		assert_eq!(links, vec!["my-db:db".to_string()]);
	}

	#[test]
	#[cfg(unix)]
	fn bind_source_resolution() {
		use super::resolve_bind_source;
		use std::path::Path;
		let base = Path::new("/srv/app");
		assert_eq!(resolve_bind_source("/abs/path", base), "/abs/path");
		assert_eq!(resolve_bind_source("./data", base), "/srv/app/./data");
		assert_eq!(resolve_bind_source("data", base), "/srv/app/data");
		std::env::set_var("HOME", "/home/u");
		assert_eq!(resolve_bind_source("~/x", base), "/home/u/x");
		assert_eq!(resolve_bind_source("~", base), "/home/u");
	}

	#[test]
	fn volume_name_resolution() {
		let f = parse_str(
			"services:\n  s:\n    image: x\nvolumes:\n  data:\n  ext:\n    external: true\n  custom:\n    name: my-vol\n",
		)
		.unwrap();
		assert_eq!(resolve_volume_name("data", "proj", &f), "proj_data");
		assert_eq!(resolve_volume_name("ext", "proj", &f), "ext");
		assert_eq!(resolve_volume_name("custom", "proj", &f), "my-vol");
		// Not declared in top-level volumes -> left as-is.
		assert_eq!(resolve_volume_name("anon", "proj", &f), "anon");
	}

	#[test]
	fn config_hash_is_stable_and_sensitive() {
		let a = parse_str("services:\n  web:\n    image: nginx:1.27\n").unwrap();
		let b = parse_str("services:\n  web:\n    image: nginx:1.27\n").unwrap();
		let c = parse_str("services:\n  web:\n    image: nginx:1.28\n").unwrap();
		let ha = config_hash(&a.services["web"], &a).unwrap();
		let hb = config_hash(&b.services["web"], &b).unwrap();
		let hc = config_hash(&c.services["web"], &c).unwrap();
		assert_eq!(ha, hb, "same config produces the same hash");
		assert_ne!(ha, hc, "a changed image produces a different hash");
		assert_eq!(ha.len(), 64, "sha-256 hex is 64 chars");
	}

	#[test]
	fn config_hash_tracks_inline_secret_content() {
		// Rotating an inline `content:` secret must change the hash so the
		// container is recreated to pick up the new (point-in-time) native secret.
		let a = parse_str(
			"services:\n  web:\n    image: x\n    secrets: [tok]\nsecrets:\n  tok:\n    content: v1\n",
		)
		.unwrap();
		let b = parse_str(
			"services:\n  web:\n    image: x\n    secrets: [tok]\nsecrets:\n  tok:\n    content: v2\n",
		)
		.unwrap();
		assert_ne!(
			config_hash(&a.services["web"], &a).unwrap(),
			config_hash(&b.services["web"], &b).unwrap(),
			"changed inline secret content must change the hash",
		);
	}

	#[test]
	fn config_hash_ignores_external_secret_identity() {
		// An `external:` secret is by-reference (no payload), so it does not add
		// to the hash beyond the service's own secret list.
		let a = parse_str(
			"services:\n  web:\n    image: x\n    secrets: [tok]\nsecrets:\n  tok:\n    external: true\n",
		)
		.unwrap();
		let b = parse_str(
			"services:\n  web:\n    image: x\n    secrets: [tok]\nsecrets:\n  tok:\n    external: true\n    name: other\n",
		)
		.unwrap();
		// The service definition is identical; only the top-level external name
		// differs, which is resolved at attach time, not baked into the hash.
		assert_eq!(
			config_hash(&a.services["web"], &a).unwrap(),
			config_hash(&b.services["web"], &b).unwrap(),
		);
	}

	#[test]
	fn config_hash_stable_despite_map_field_order() {
		// `storage_opt` is a HashMap; canonical serialisation must sort its keys
		// so the hash does not flap and trigger a spurious recreate on `up`.
		let a = parse_str(
			"services:\n  web:\n    image: x\n    storage_opt:\n      size: \"10G\"\n      foo: bar\n      baz: qux\n",
		)
		.unwrap();
		let b = parse_str(
			"services:\n  web:\n    image: x\n    storage_opt:\n      baz: qux\n      size: \"10G\"\n      foo: bar\n",
		)
		.unwrap();
		assert_eq!(
			config_hash(&a.services["web"], &a).unwrap(),
			config_hash(&b.services["web"], &b).unwrap(),
			"hash must be independent of storage_opt key order",
		);
	}
}
