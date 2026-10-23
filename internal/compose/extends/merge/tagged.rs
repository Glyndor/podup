use crate::compose::types::Service;

use super::merge_service;

/// [`merge_service`] with the overriding file's `!override`/`!reset` tags.
///
/// A key tagged `!override` takes the overriding value whole, skipping whatever
/// combine rule would normally apply; `!reset` drops the key entirely, leaving
/// the type's default. Both were accepted and silently ignored before, so a file
/// asking for replacement got an append instead, the opposite of what it said.
///
/// Implemented by pre-shaping the two sides and then running the ordinary merge,
/// rather than by branching inside it: `!reset` empties both sides so the result
/// is the default, and `!override` empties only the base so the combine rule has
/// nothing to combine with and the overriding value survives whole. One rule
/// each, and the field-by-field merge below stays the single place that knows
/// how any given key combines.
pub(in crate::compose) fn merge_service_tagged(
	base: Service,
	override_svc: Service,
	tagged: Option<&std::collections::HashMap<String, crate::compose::tags::MergeTag>>,
) -> Service {
	let Some(tagged) = tagged.filter(|t| !t.is_empty()) else {
		return merge_service(base, override_svc);
	};
	let mut base = base;
	let mut over = override_svc;
	for (key, tag) in tagged {
		clear_service_key(&mut base, key);
		if matches!(tag, crate::compose::tags::MergeTag::Reset) {
			clear_service_key(&mut over, key);
		}
	}
	merge_service(base, over)
}

/// Reset one service key to its default, by name: the primitive both tags are
/// built from.
///
/// A key podup does not model is ignored: a tag cannot change a merge that never
/// happens, and refusing it would reject a file that is valid elsewhere.
fn clear_service_key(svc: &mut Service, key: &str) {
	match key {
		"ports" => svc.ports = Vec::new(),
		"expose" => svc.expose = Vec::new(),
		"volumes" => svc.volumes = Vec::new(),
		"volumes_from" => svc.volumes_from = Vec::new(),
		"networks" => svc.networks = Default::default(),
		"environment" => svc.environment = Default::default(),
		"env_file" => svc.env_file = Default::default(),
		"labels" => svc.labels = Default::default(),
		"label_file" => svc.label_file = Default::default(),
		"dns" => svc.dns = Default::default(),
		"dns_search" => svc.dns_search = Default::default(),
		"dns_opt" => svc.dns_opt = Default::default(),
		"tmpfs" => svc.tmpfs = Default::default(),
		"sysctls" => svc.sysctls = Default::default(),
		"cap_add" => svc.cap_add = Vec::new(),
		"cap_drop" => svc.cap_drop = Vec::new(),
		"devices" => svc.devices = Vec::new(),
		"extra_hosts" => svc.extra_hosts = Vec::new(),
		"ulimits" => svc.ulimits = Default::default(),
		"depends_on" => svc.depends_on = Default::default(),
		"secrets" => svc.secrets = Vec::new(),
		"configs" => svc.configs = Vec::new(),
		"links" => svc.links = Vec::new(),
		"external_links" => svc.external_links = Vec::new(),
		"group_add" => svc.group_add = Vec::new(),
		"security_opt" => svc.security_opt = Vec::new(),
		"profiles" => svc.profiles = Vec::new(),
		_ => {}
	}
}
