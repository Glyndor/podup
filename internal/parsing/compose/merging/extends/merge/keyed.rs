//! Lists whose entries have an identity: a later file's entry with the same
//! identity replaces the earlier one in place, as `docker compose config` 5.7.0
//! does, instead of being added next to it.

use crate::compose::types::{PortMapping, ServiceConfigRef, ServiceSecretRef};

/// Merge two lists by `key`. An entry whose key is already there replaces it at
/// the earlier position; a new key appends. An entry with no key (it could not
/// be read) is kept unless the very same entry, compared serialized, is there.
pub(super) fn merge_keyed<T: serde::Serialize>(
	base: Vec<T>,
	over: Vec<T>,
	key: impl Fn(&T) -> Option<String>,
) -> Vec<T> {
	let raw = |item: &T| serde_yaml::to_string(item).ok().map(|s| format!("raw:{s}"));
	let ident = |item: &T| key(item).or_else(|| raw(item));
	let mut at: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
	let mut out = base;
	for (i, item) in out.iter().enumerate() {
		if let Some(k) = ident(item) {
			at.entry(k).or_insert(i);
		}
	}
	for item in over {
		match ident(&item) {
			Some(k) => match at.get(&k) {
				Some(&i) => out[i] = item,
				None => {
					at.insert(k, out.len());
					out.push(item);
				}
			},
			None => out.push(item),
		}
	}
	out
}

/// A secret is identified by where it is mounted: `/run/secrets/<source>` when
/// no target is given, the target as written otherwise.
pub(super) fn secret_key(secret: &ServiceSecretRef) -> Option<String> {
	Some(match secret.target() {
		Some(target) => target.to_string(),
		None => format!("/run/secrets/{}", secret.source()),
	})
}

/// A config is identified by where it is mounted: `/<source>` by default.
pub(super) fn config_key(config: &ServiceConfigRef) -> Option<String> {
	Some(match config.target() {
		Some(target) => target.to_string(),
		None => format!("/{}", config.source()),
	})
}

/// A port is identified by the bindings it makes (host IP, host port, container
/// port, protocol), so `8080:80` and its long form are one entry. `mode`,
/// `name` and `app_protocol` do not count. `None` when it cannot be parsed.
pub(super) fn port_key(port: &PortMapping) -> Option<String> {
	let parsed = crate::ports::parse_ports(std::slice::from_ref(port)).ok()?;
	serde_json::to_string(&parsed).ok()
}
