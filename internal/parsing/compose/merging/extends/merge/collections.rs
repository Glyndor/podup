use crate::compose::types::{ServiceNetworks, Sysctls};

/// Merge `volumes:` by **container-side target**, with the override winning.
///
/// `merge_vec` dedups by serialized form, so a base mounting `./a:/data` and an
/// override remapping it to `./b:/data` produced both entries, and
/// `podman create` refused the container with `duplicate mount destination`.
/// docker compose replaces the mount at a target it already has, which is what
/// remapping a path in an override is *for*.
///
/// Base order is preserved so an unchanged mount stays where it was; an override
/// introducing a new target appends.
pub(super) fn merge_volumes(
	base: Vec<crate::compose::types::VolumeMount>,
	over: Vec<crate::compose::types::VolumeMount>,
) -> Vec<crate::compose::types::VolumeMount> {
	let mut out = base;
	for item in over {
		match out.iter().position(|m| m.target() == item.target()) {
			Some(i) => out[i] = item,
			None => out.push(item),
		}
	}
	out
}

/// Union two `networks:` declarations, keeping the base's attachments and adding
/// the override's. For a network both declare, the override's per-network config
/// wins; a bare-list entry contributes no config, so it never erases one.
///
/// The short (list) and long (map) forms mix freely across `-f` and `extends`,
/// so both collapse to the map form whenever either side carries config.
pub(super) fn merge_networks(base: ServiceNetworks, over: ServiceNetworks) -> ServiceNetworks {
	if matches!(over, ServiceNetworks::Empty) {
		return base;
	}
	if matches!(base, ServiceNetworks::Empty) {
		return over;
	}
	// Preserve declaration order: the base's networks first, then any the
	// override introduces.
	let mut out: indexmap::IndexMap<String, Option<crate::compose::types::ServiceNetworkConfig>> =
		indexmap::IndexMap::new();
	for name in base.names() {
		let cfg = base.config_for(&name).cloned();
		out.insert(name, cfg);
	}
	for name in over.names() {
		let cfg = over.config_for(&name).cloned();
		match out.entry(name) {
			indexmap::map::Entry::Occupied(mut e) => {
				// A bare name in the override must not wipe config the base set.
				if cfg.is_some() {
					e.insert(cfg);
				}
			}
			indexmap::map::Entry::Vacant(e) => {
				e.insert(cfg);
			}
		}
	}
	// Stay in the short form when nothing carries per-network config. That is the
	// form the user wrote and the one docker keeps, and a map of all-`None`
	// values is pruned as empty by the `config` renderer's null-stripping, which
	// would drop the networks from the rendered file entirely.
	if out.values().all(Option::is_none) {
		return ServiceNetworks::List(out.into_keys().collect());
	}
	ServiceNetworks::Map(out)
}

/// Merge `sysctls:` per key, the way `environment` and `labels` already merge:
/// the override wins for a key both set, and the base keeps the rest.
pub(super) fn merge_sysctls(base: Sysctls, over: Sysctls) -> Sysctls {
	if matches!(over, Sysctls::Empty) {
		return base;
	}
	if matches!(base, Sysctls::Empty) {
		return over;
	}
	let mut out: indexmap::IndexMap<String, serde_yaml::Value> = indexmap::IndexMap::new();
	for (k, v) in base.to_map() {
		out.insert(k, serde_yaml::Value::String(v));
	}
	for (k, v) in over.to_map() {
		out.insert(k, serde_yaml::Value::String(v));
	}
	Sysctls::Map(out)
}

/// Merge two nested blocks (`build`, `healthcheck`, `logging`, `deploy`,
/// `develop`) key by key instead of replacing the base with the override.
///
/// Compose merges mappings recursively: an override that sets only
/// `deploy.resources.limits.memory` keeps the base's `cpus`, and one that sets
/// only a healthcheck `interval` keeps the base's `test`. A value the override
/// does set wins, and a list in the override replaces the base's list (the
/// blocks merged here have no list that compose concatenates). A null in the
/// override is "not set". If the merged value cannot be typed again the
/// override is used as it is, which is what a plain replacement did before.
pub(super) fn merge_block<T>(
	base: Option<T>,
	over: Option<T>,
	short_form_key: Option<&str>,
) -> Option<T>
where
	T: serde::Serialize + serde::de::DeserializeOwned,
{
	use serde_json::Value;

	/// A list of `KEY=VALUE` entries as the map it stands for (a bare `KEY` has
	/// no value), so a list on one side and a map on the other merge key by key.
	fn list_as_map(items: &[Value]) -> Option<serde_json::Map<String, Value>> {
		let mut map = serde_json::Map::new();
		for item in items {
			let entry = item.as_str()?;
			match entry.split_once('=') {
				Some((key, value)) => map.insert(key.to_string(), Value::String(value.to_string())),
				None => map.insert(entry.to_string(), Value::Null),
			};
		}
		Some(map)
	}

	fn deep(base: Value, over: Value) -> Value {
		match (base, over) {
			(Value::Array(list), Value::Object(o)) => match list_as_map(&list) {
				Some(b) => deep(Value::Object(b), Value::Object(o)),
				None => Value::Object(o),
			},
			(Value::Object(b), Value::Array(list)) => match list_as_map(&list) {
				Some(o) => deep(Value::Object(b), Value::Object(o)),
				None => Value::Array(list),
			},
			(Value::Object(mut b), Value::Object(o)) => {
				for (key, value) in o {
					let merged = match b.remove(&key) {
						Some(existing) => deep(existing, value),
						None => value,
					};
					b.insert(key, merged);
				}
				Value::Object(b)
			}
			(base, Value::Null) => base,
			(_, over) => over,
		}
	}

	match (base, over) {
		(None, over) => over,
		(base, None) => base,
		(Some(base), Some(over)) => {
			// The short form (`build: ./app`) stands for `{context: ./app}`; read it
			// that way next to a long form so the context is not lost.
			let long = |value: Value| match (short_form_key, value) {
				(Some(key), Value::String(text)) => Value::Object(serde_json::Map::from_iter([(
					key.to_string(),
					Value::String(text),
				)])),
				(_, value) => value,
			};
			let merged = serde_json::to_value(&base)
				.and_then(|b| serde_json::to_value(&over).map(|o| (b, o)))
				.map(|(b, o)| {
					if b.is_object() || o.is_object() {
						deep(long(b), long(o))
					} else {
						deep(b, o)
					}
				})
				.and_then(serde_json::from_value::<T>);
			Some(merged.unwrap_or(over))
		}
	}
}
