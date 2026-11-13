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
	// The first position of each target, so a long list is not rescanned for
	// every entry of the other.
	let mut at: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
	for (i, m) in out.iter().enumerate() {
		at.entry(target_key(m.target())).or_insert(i);
	}
	for item in over {
		match at.get(&target_key(item.target())) {
			Some(&i) => out[i] = item,
			None => {
				at.insert(target_key(item.target()), out.len());
				out.push(item);
			}
		}
	}
	out
}

/// A mount target as docker compares it: `/data/` and `/data` are one path.
fn target_key(target: &str) -> String {
	match target.trim_end_matches('/') {
		"" if target.starts_with('/') => "/".to_string(),
		trimmed => trimmed.to_string(),
	}
}

/// Merge `devices:` by **container path**, with the override winning.
///
/// docker compose keeps one entry per path a device appears at, so remapping a
/// device in an override replaces the base entry instead of adding a second one
/// at the same path. Base order is kept; a new path appends.
pub(super) fn merge_devices(base: Vec<String>, over: Vec<String>) -> Vec<String> {
	use crate::compose::types::service::devices::container_path;
	let mut out = base;
	for item in over {
		match out
			.iter()
			.position(|d| container_path(d) == container_path(&item))
		{
			Some(i) => out[i] = item,
			None => out.push(item),
		}
	}
	out
}

/// Merge the per-network config of one attachment declared in two files.
///
/// Docker appends the lists (`aliases`, `link_local_ips`) without repeats, lets
/// the override win for each scalar it sets, and keeps the base's for the rest.
fn merge_network_config(
	base: crate::compose::types::ServiceNetworkConfig,
	over: crate::compose::types::ServiceNetworkConfig,
) -> crate::compose::types::ServiceNetworkConfig {
	use crate::compose::types::ServiceNetworkConfig as Config;

	fn append(mut base: Vec<String>, over: Vec<String>) -> Vec<String> {
		for item in over {
			if !base.contains(&item) {
				base.push(item);
			}
		}
		base
	}

	let aliases = match (base.aliases, over.aliases) {
		(Some(b), Some(o)) => Some(append(b, o)),
		(b, o) => o.or(b),
	};
	let mut driver_opts = base.driver_opts;
	driver_opts.extend(over.driver_opts);
	Config {
		aliases,
		ipv4_address: over.ipv4_address.or(base.ipv4_address),
		ipv6_address: over.ipv6_address.or(base.ipv6_address),
		link_local_ips: append(base.link_local_ips, over.link_local_ips),
		priority: over.priority.or(base.priority),
		mac_address: over.mac_address.or(base.mac_address),
		driver_opts,
		gw_priority: over.gw_priority.or(base.gw_priority),
		interface_name: over.interface_name.or(base.interface_name),
	}
}

/// Union two `networks:` declarations, keeping the base's attachments and adding
/// the override's. For a network both declare, the per-network config merges
/// field by field (see [`merge_network_config`]); a bare-list entry contributes
/// no config, so it never erases one.
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
				// A bare name in the override must not wipe config the base set,
				// and a config in the override adds to the base's field by field.
				if let Some(over_cfg) = cfg {
					let merged = match e.get().clone() {
						Some(base_cfg) => merge_network_config(base_cfg, over_cfg),
						None => over_cfg,
					};
					e.insert(Some(merged));
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
/// does set wins, and a list in the override replaces the base's list, except
/// the lists compose concatenates without repeats (see [`appends`]). A new
/// `logging.driver` drops the base's `options`, which belong to the old driver.
/// A null in the override is "not set". If the merged value cannot be typed
/// again the override is used as it is, which is what a plain replacement did
/// before.
pub(in crate::parsing::compose) fn merge_block<T>(
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

	fn deep(base: Value, over: Value, path: &mut Vec<String>) -> Value {
		match (base, over) {
			(Value::Array(list), Value::Object(o)) => match list_as_map(&list) {
				Some(b) => deep(Value::Object(b), Value::Object(o), path),
				None => Value::Object(o),
			},
			(Value::Object(b), Value::Array(list)) => match list_as_map(&list) {
				Some(o) => deep(Value::Object(b), Value::Object(o), path),
				None => Value::Array(list),
			},
			(Value::Object(mut b), Value::Object(o)) => {
				// A different driver makes the base's options meaningless for it.
				let new_driver = match (b.get("driver"), o.get("driver")) {
					(Some(old), Some(new)) => old != new,
					_ => false,
				};
				if new_driver && !o.contains_key("options") {
					b.remove("options");
				}
				for (key, value) in o {
					path.push(key.clone());
					let merged = match b.remove(&key) {
						Some(existing) => deep(existing, value, path),
						None => value,
					};
					path.pop();
					b.insert(key, merged);
				}
				Value::Object(b)
			}
			(Value::Array(mut b), Value::Array(o)) if appends(path) => {
				for item in o {
					if !b.contains(&item) {
						b.push(item);
					}
				}
				Value::Array(b)
			}
			(base, Value::Null) => base,
			(_, over) => over,
		}
	}

	/// The lists, by path from the block's root, that compose appends to instead
	/// of replacing: measured against `docker compose config` 5.7.0. The blocks
	/// are `build`, `deploy`, `develop` and `blkio_config`, and no path is shared
	/// between two of them.
	const APPENDS: &[&[&str]] = &[
		&["cache_from"],
		&["cache_to"],
		&["tags"],
		&["extra_hosts"],
		&["platforms"],
		&["resources", "reservations", "devices"],
		&["placement", "constraints"],
		&["placement", "preferences"],
		&["watch"],
		&["weight_device"],
		&["device_read_bps"],
		&["device_write_bps"],
		&["device_read_iops"],
		&["device_write_iops"],
	];

	fn appends(path: &[String]) -> bool {
		APPENDS
			.iter()
			.any(|known| known.iter().copied().eq(path.iter().map(String::as_str)))
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
						deep(long(b), long(o), &mut Vec::new())
					} else {
						deep(b, o, &mut Vec::new())
					}
				})
				.and_then(serde_json::from_value::<T>);
			Some(merged.unwrap_or(over))
		}
	}
}
