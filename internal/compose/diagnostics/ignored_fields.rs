//! Warnings for service/network/secret fields that podup parses but cannot
//! translate. Split out of the diagnostics root so each collector stays small.

use crate::compose::types::{BuildConfig, ComposeFile, EnvFileEntry, PortMapping, VolumeMount};

/// Service fields that podup models but cannot honor on rootless Podman.
pub(super) fn ignored_service_fields(file: &ComposeFile, out: &mut Vec<String>) {
	for (service, def) in &file.services {
		if def.cpu_count.is_some() {
			out.push(format!(
				"service '{service}': cpu_count is a Windows/Hyper-V control with no \
				 rootless Podman equivalent and is ignored"
			));
		}
		if def.cpu_percent.is_some() {
			out.push(format!(
				"service '{service}': cpu_percent is a Windows/Hyper-V control with no \
				 rootless Podman equivalent and is ignored"
			));
		}
		if def.credential_spec.is_some() {
			out.push(format!(
				"service '{service}': credential_spec is a Windows managed-service-account \
				 control with no rootless Podman equivalent and is not honored"
			));
		}
		if def.isolation.is_some() {
			out.push(format!(
				"service '{service}': isolation has no rootless Podman equivalent and is \
				 not honored"
			));
		}
		if def.provider.is_some() {
			out.push(format!(
				"service '{service}': provider delegates the service lifecycle to an \
				 external plugin that podup does not invoke; the service is not honored"
			));
		}
		if def.use_api_socket.is_some() {
			out.push(format!(
				"service '{service}': use_api_socket has no podup equivalent and is not \
				 honored"
			));
		}
		for entry in def.env_file.to_entries() {
			if let EnvFileEntry::Config {
				format: Some(fmt), ..
			} = entry
			{
				out.push(format!(
					"service '{service}': env_file format '{fmt}' is not honored; podup \
					 always parses env files as dotenv"
				));
			}
		}
	}
}

/// Top-level `models:` (Compose v2.38): podup runs no model runner, so any
/// declared model is parsed for fidelity but not honored.
pub(super) fn ignored_models(file: &ComposeFile, out: &mut Vec<String>) {
	for name in file.models.keys() {
		out.push(format!(
			"model '{name}': podup runs no model runner, so the models element is not \
			 honored"
		));
	}
}

/// Long-form port fields podup parses but does not forward to Podman.
pub(super) fn ignored_port_fields(file: &ComposeFile, out: &mut Vec<String>) {
	for (service, def) in &file.services {
		for port in &def.ports {
			if let PortMapping::Long { mode: Some(m), .. } = port {
				out.push(format!(
					"service '{service}': port mode '{m}' is a Swarm/ingress control \
					 with no single-host Podman equivalent and is ignored"
				));
			}
		}
	}
}

/// One port the parse-time port-exposure warning flags. Returned by
/// [`ports_published_on_all_interfaces`] so the audit module can build
/// its findings from the same notion of "published on every interface"
/// instead of inventing a second predicate (#1835). The fields are
/// exactly what each downstream formatter needs: `service` for both,
/// `host` for the diagnostic message and the audit reason, and
/// `cont` (set only for the short form) so the diagnostic can include
/// the container port in the `127.0.0.1:host:cont` fix-it suggestion.
pub(crate) struct PortExposure {
	/// Compose service name.
	pub service: String,
	/// Host port label, e.g. `"5432"` or `"8080-8090"`. Used in both
	/// the diagnostic warning and the audit finding reason.
	pub host: String,
	/// Container port, set for the short form so the diagnostic
	/// suggestion can read `127.0.0.1:{host}:{cont}`. `None` for the
	/// long form where the fix is `host_ip: "127.0.0.1"`.
	pub cont: Option<String>,
}

/// Enumerate every port the diagnostic warning would flag: a
/// short-form `host:container` with no IP, or a long-form mapping with
/// `published` but no `host_ip`. The predicate is the single source of
/// truth for "published on every interface" in this crate; both the
/// parse-time warning in [`port_published_on_all_interfaces`] and the
/// audit module's `port_published_on_all_interfaces` check read from
/// this list, so the two cannot drift on a future compose-shape
/// addition (#1835).
///
/// Threshold:
/// - Short form with 1 colon (`"5432:5432"`): flagged, no IP, the bind
///   falls on every interface.
/// - Short form with 2+ colons (`"0.0.0.0:5432:5432"`): not flagged.
///   An explicit `host_ip`, including `0.0.0.0`, is a decision taken;
///   flagging it would only train the reader to ignore the warning,
///   the same argument the diagnostic comment above
///   [`port_published_on_all_interfaces`] makes.
/// - Short form with 0 colons (`"5432"`): not flagged. Container-only
///   is the short-form mirror of `expose:`, not a publish.
/// - Short form `[::1]:5432:5432`: not flagged. IPv6 carries its own
///   host-IP detection; the `[` is the marker.
/// - Long form with `published` but no `host_ip`: flagged.
/// - Long form with `host_ip` set (any non-empty value, including
///   `0.0.0.0` or a private LAN address like `192.168.1.10`): not
///   flagged. Same argument: an explicit bind is a decision.
/// - Long form with no `published`: not flagged. The port is exposed,
///   not published on the host.
pub(crate) fn ports_published_on_all_interfaces(file: &ComposeFile) -> Vec<PortExposure> {
	let mut out = Vec::new();
	for (service, def) in &file.services {
		for port in &def.ports {
			match port {
				PortMapping::Short(s) => {
					let no_proto = s.split('/').next().unwrap_or(s);
					// IPv6 form (`[::1]:host:container`) always carries an IP.
					if no_proto.starts_with('[') {
						continue;
					}
					let colon_count = no_proto.chars().filter(|&c| c == ':').count();
					// 0 colons = container-only (expose, not publish).
					// 2+ colons = ip:host:container (has IP).
					// 1 colon = host:container without IP.
					if colon_count == 1 {
						let mut parts = no_proto.split(':');
						let host = parts.next().unwrap_or("").to_string();
						let cont = parts.next().unwrap_or("").to_string();
						if host.is_empty() {
							// Malformed port string; skip rather than emit a
							// finding with an empty label.
							continue;
						}
						out.push(PortExposure {
							service: service.clone(),
							host,
							cont: Some(cont),
						});
					}
				}
				PortMapping::Long {
					published: Some(p),
					host_ip,
					..
				} => {
					let explicit = host_ip.as_deref().is_some_and(|s| !s.trim().is_empty());
					if !explicit {
						out.push(PortExposure {
							service: service.clone(),
							host: p.as_str_val(),
							cont: None,
						});
					}
				}
				PortMapping::Long {
					published: None, ..
				} => {
					// No `published` = port is exposed, not published on host.
				}
			}
		}
	}
	out
}

/// Enumerate every port published with an explicit wildcard host IP
/// (`0.0.0.0` or `::`). Companion to
/// [`ports_published_on_all_interfaces`]: the all-interfaces predicate
/// fires on "no IP given" (the operator let the runtime pick), and this
/// one fires on "the operator wrote a wildcard on purpose". The two are
/// disjoint on the same mapping: an IP-less form has no IP to compare,
/// and a wildcard form has an explicit IP that is not "all interfaces"
/// in the audit's no-IP sense, so a compose file can have both findings
/// raised for different mappings but never for the same one (#1881).
///
/// Threshold:
/// - Short form with 2+ colons, first segment `0.0.0.0`: flagged.
/// - Short form `[::]:host:container`: flagged. The `[::]` is the
///   IPv6 wildcard; the `[` is the IP marker, the body is `::`.
/// - Short form with 2+ colons and any other IP (`127.0.0.1`,
///   `192.168.1.10`, `[::1]`, `fd00::1`): not flagged. A specific IP
///   is a deliberate decision and is exactly the case the operator
///   opted into when they typed one.
/// - Short form with 1 colon (`"5432:5432"`): not flagged here. No IP,
///   so no wildcard to match; the all-interfaces check covers it.
/// - Short form with 0 colons (`"5432"`): not flagged. Container-only.
/// - Long form with `host_ip` exactly `0.0.0.0` or `::`: flagged.
/// - Long form with `host_ip` set to any other value: not flagged.
/// - Long form with no `host_ip` or empty `host_ip`: not flagged here.
///   The all-interfaces check covers "no IP".
/// - Long form with no `published`: not flagged. The port is exposed,
///   not published on the host.
pub(crate) fn ports_published_on_wildcard(file: &ComposeFile) -> Vec<PortExposure> {
	let mut out = Vec::new();
	for (service, def) in &file.services {
		for port in &def.ports {
			match port {
				PortMapping::Short(s) => {
					let no_proto = s.split('/').next().unwrap_or(s);
					let Some((host, cont)) = wildcard_short_port(no_proto) else {
						continue;
					};
					if host.is_empty() {
						// Malformed port string; skip rather than emit a
						// finding with an empty label.
						continue;
					}
					out.push(PortExposure {
						service: service.clone(),
						host,
						cont: Some(cont),
					});
				}
				PortMapping::Long {
					published: Some(p),
					host_ip: Some(ip),
					..
				} => {
					if is_wildcard_long(ip) {
						out.push(PortExposure {
							service: service.clone(),
							host: p.as_str_val(),
							cont: None,
						});
					}
				}
				PortMapping::Long { .. } => {
					// No `host_ip` set, or no `published`: not a wildcard.
					// `host_ip: ""` is empty and the long-form check treats
					// it as "no IP", which is the all-interfaces predicate's
					// case.
				}
			}
		}
	}
	out
}

/// `Some((host_port, container_port))` when `no_proto` is a short-form
/// `ip:host:container` whose IP is a wildcard. `None` otherwise (no IP,
/// specific IP, container-only, malformed). The pair matches the
/// `PortExposure.host`/`PortExposure.cont` shape the all-interfaces
/// predicate populates so the audit message can name the port the
/// operator sees in the compose file (`#1881`).
fn wildcard_short_port(no_proto: &str) -> Option<(String, String)> {
	if no_proto.starts_with('[') {
		// `[::]:host:container` is the wildcard; `[::1]:host:container`
		// is a specific IPv6 loopback. The body between `[` and `]`
		// decides which one.
		if !no_proto.starts_with("[::]:") {
			return None;
		}
		let rest = &no_proto["[::]:".len()..];
		// `host:container` has 1 colon, exactly like the all-interfaces
		// predicate expects; anything else (no colon, two colons) is
		// not a publish of the `host:container` shape.
		if rest.chars().filter(|&c| c == ':').count() != 1 {
			return None;
		}
		let mut parts = rest.split(':');
		let host = parts.next().unwrap_or("").to_string();
		let cont = parts.next().unwrap_or("").to_string();
		return Some((host, cont));
	}
	// IPv4 form: `ip:host:container` has 2 colons, same as the
	// all-interfaces predicate's "has IP" branch uses to decide.
	let colon_count = no_proto.chars().filter(|&c| c == ':').count();
	if colon_count != 2 {
		return None;
	}
	let mut parts = no_proto.split(':');
	let ip = parts.next().unwrap_or("");
	let host = parts.next().unwrap_or("").to_string();
	let cont = parts.next().unwrap_or("").to_string();
	if ip != "0.0.0.0" {
		return None;
	}
	Some((host, cont))
}

/// `true` when the long-form `host_ip` value, trimmed, is exactly
/// `0.0.0.0` or `::`. Empty / whitespace-only values are not wildcards
/// here: the all-interfaces predicate owns the "no IP given" case so a
/// compose file with `host_ip: ""` does not double-fire (#1881).
fn is_wildcard_long(ip: &str) -> bool {
	let trimmed = ip.trim();
	trimmed == "0.0.0.0" || trimmed == "::"
}

/// Warn when a service publishes a port on every host interface. The
/// compose-spec short form (`"5432:5432"`) and the long form with no
/// `host_ip` both bind on all interfaces, which exposes services the
/// operator thought were local-only (databases, admin UIs) to anything
/// reachable on the host's network. An explicit `host_ip`, including
/// `0.0.0.0`, is a decision taken and is not flagged, so flagging it
/// would only train the reader to ignore the warning.
///
/// The warning text and the audit module's `port_published_on_all_interfaces`
/// check are both built from [`ports_published_on_all_interfaces`]; the
/// two surfaces share the same predicate so they cannot drift (#1835).
pub(super) fn port_published_on_all_interfaces(file: &ComposeFile, out: &mut Vec<String>) {
	for exposure in ports_published_on_all_interfaces(file) {
		let fix = match &exposure.cont {
			Some(cont) => format!("\"127.0.0.1:{host}:{cont}\"", host = exposure.host),
			None => "host_ip: \"127.0.0.1\"".to_string(),
		};
		out.push(format!(
			"service '{service}': port {host} is published on every \
			 interface; use {fix} to keep it on the host",
			service = exposure.service,
			host = exposure.host,
		));
	}
}

/// Per-mount long-form volume options podup parses but does not forward.
pub(super) fn ignored_volume_mount_fields(file: &ComposeFile, out: &mut Vec<String>) {
	for (service, def) in &file.services {
		for mount in &def.volumes {
			if let VolumeMount::Long {
				volume: Some(opts), ..
			} = mount
			{
				if opts.driver_config.is_some() {
					out.push(format!(
						"service '{service}' volume '{}': per-mount driver_config is not \
						 forwarded to Podman and is ignored",
						mount.target()
					));
				}
			}
		}
	}
}

/// Build options that exist only in BuildKit/buildx and have no libpod
/// build-API mapping. Honored fields are left untouched.
pub(super) fn ignored_build_fields(file: &ComposeFile, out: &mut Vec<String>) {
	for (service, def) in &file.services {
		let Some(BuildConfig::Config {
			privileged,
			isolation,
			entitlements,
			provenance,
			sbom,
			ssh,
			..
		}) = &def.build
		else {
			continue;
		};
		let mut unmapped: Vec<&str> = Vec::new();
		if privileged.is_some() {
			unmapped.push("privileged");
		}
		if !ssh.is_empty() {
			unmapped.push("ssh");
		}
		if isolation.is_some() {
			unmapped.push("isolation");
		}
		if !entitlements.is_empty() {
			unmapped.push("entitlements");
		}
		if provenance.is_some() {
			unmapped.push("provenance");
		}
		if sbom.is_some() {
			unmapped.push("sbom");
		}
		for field in unmapped {
			out.push(format!(
				"service '{service}': build.{field} has no libpod build-API equivalent \
				 and is ignored"
			));
		}
	}
}

/// Network fields podup parses but does not forward to Podman.
pub(super) fn ignored_network_fields(file: &ComposeFile, out: &mut Vec<String>) {
	for (name, cfg) in &file.networks {
		if let Some(c) = cfg {
			if c.enable_ipv4.is_some() {
				out.push(format!(
					"network '{name}': enable_ipv4 is not forwarded; Podman networks \
					 enable IPv4 by default and expose no toggle"
				));
			}
			if let Some(ipam) = &c.ipam {
				if ipam
					.config
					.iter()
					.any(|pool| !pool.aux_addresses.is_empty())
				{
					out.push(format!(
						"network '{name}': ipam aux_addresses are not supported by Podman \
						 and are ignored"
					));
				}
			}
		}
	}
}

/// `deploy.restart_policy` sub-fields with no Podman restart-policy equivalent.
/// Only `condition` and `max_attempts` are honored (see container_config); Podman
/// has no first-class restart delay or attempt-counting window.
pub(super) fn ignored_restart_policy_fields(file: &ComposeFile, out: &mut Vec<String>) {
	for (service, def) in &file.services {
		let Some(drp) = def.deploy.as_ref().and_then(|d| d.restart_policy.as_ref()) else {
			continue;
		};
		if drp.delay.is_some() {
			out.push(format!(
				"service '{service}': deploy.restart_policy.delay has no Podman equivalent \
				 and is ignored"
			));
		}
		if drp.window.is_some() {
			out.push(format!(
				"service '{service}': deploy.restart_policy.window has no Podman equivalent \
				 and is ignored"
			));
		}
	}
}

/// Per-service network attachment fields podup parses but cannot forward.
/// `gw_priority` has no Podman equivalent, so the engine drops it silently.
pub(super) fn ignored_service_network_fields(file: &ComposeFile, out: &mut Vec<String>) {
	for (service, def) in &file.services {
		for name in def.networks.names() {
			if let Some(c) = def.networks.config_for(&name) {
				if c.gw_priority.is_some() {
					out.push(format!(
						"service '{service}' network '{name}': gw_priority is not supported \
						 by Podman and is ignored"
					));
				}
			}
		}
	}
}

/// Top-level secret/config driver fields. An external secret-store driver
/// (Vault, AWS SM, etc.) on a non-`external` definition is not honored: podup
/// only stages `file`/`content`/`environment` sources and routes `external:
/// true` to Podman-native secrets, so warn rather than mount nothing silently.
pub(super) fn ignored_secret_config_drivers(file: &ComposeFile, out: &mut Vec<String>) {
	for (name, cfg) in &file.secrets {
		if cfg.external != Some(true) {
			if cfg.driver.is_some() {
				out.push(format!(
					"secret '{name}': driver is an external secret-store plugin that podup \
					 does not invoke; the secret will not be staged"
				));
			}
			if cfg.template_driver.is_some() {
				out.push(format!(
					"secret '{name}': template_driver is not supported and is ignored"
				));
			}
		}
	}
	for (name, cfg) in &file.configs {
		if cfg.external != Some(true) {
			if cfg.driver.is_some() {
				out.push(format!(
					"config '{name}': driver is an external plugin that podup does not \
					 invoke; the config will not be staged"
				));
			}
			if cfg.template_driver.is_some() {
				out.push(format!(
					"config '{name}': template_driver is not supported and is ignored"
				));
			}
		}
	}
}
