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
pub(in crate::parsing::compose) fn merge_service_tagged(
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
/// Every key a file can set is listed, so a tag works on all of them: a key left
/// out kept the earlier file's value under `!reset` and combined under
/// `!override`, the opposite of what the file asked for. A key podup does not
/// model is ignored: a tag cannot change a merge that never happens, and
/// refusing it would reject a file that is valid elsewhere.
fn clear_service_key(svc: &mut Service, key: &str) {
	macro_rules! reset_fields {
		($($field:ident),* $(,)?) => {
			match key {
				"pull_refresh_after" => {
					svc.pull_refresh_after = None;
					svc.pull_refresh_after_reset = true;
				}
				$(stringify!($field) => svc.$field = Default::default(),)*
				_ => {}
			}
		};
	}
	reset_fields!(
		image,
		build,
		command,
		entrypoint,
		ports,
		expose,
		environment,
		env_file,
		volumes,
		tmpfs,
		volumes_from,
		configs,
		secrets,
		networks,
		hostname,
		domainname,
		mac_address,
		links,
		external_links,
		extra_hosts,
		dns,
		dns_search,
		dns_opt,
		network_mode,
		depends_on,
		healthcheck,
		restart,
		stop_signal,
		stop_grace_period,
		profiles,
		post_start,
		pre_start,
		pre_stop,
		labels,
		annotations,
		container_name,
		user,
		umask,
		working_dir,
		group_add,
		platform,
		cap_add,
		cap_drop,
		security_opt,
		read_only,
		privileged,
		init,
		tty,
		stdin_open,
		runtime,
		shm_size,
		userns_mode,
		pid,
		ipc,
		uts,
		cgroup_parent,
		cgroup,
		devices,
		device_cgroup_rules,
		storage_opt,
		scale,
		cpu_shares,
		cpu_quota,
		cpu_period,
		cpuset,
		cpus,
		cpu_count,
		cpu_percent,
		cpu_rt_runtime,
		cpu_rt_period,
		mem_limit,
		memswap_limit,
		mem_reservation,
		mem_swappiness,
		pids_limit,
		oom_kill_disable,
		oom_score_adj,
		blkio_config,
		logging,
		sysctls,
		ulimits,
		label_file,
		attach,
		pull_policy,
		deploy,
		develop,
		gpus,
		credential_spec,
		isolation,
		provider,
		use_api_socket,
	);
}
