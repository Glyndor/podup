//! Validation of a fully parsed and merged compose file.
//!
//! Two entry points live here. [`validate_config`] backs the `config` subcommand,
//! applying the same cross-reference and well-formedness checks
//! `docker compose config` performs and that the mutating commands would
//! otherwise only surface later (at `resolve_order` time, when Podman rejects a
//! bad port, or when an undeclared volume/network reaches the runtime). Running
//! them up front means `config` reports the divergence at exit non-zero instead
//! of echoing the file verbatim.
//!
//! [`validate`] is the semantic consistency pass run automatically after parsing
//! and merging: it rejects files that deserialize cleanly but are semantically
//! contradictory (e.g. a service that declares both `network_mode` and
//! `networks`, or an `external: true` network that also sets creation-time
//! attributes). docker-compose errors on these at config time; podup used to
//! accept them and then silently pick one interpretation, with the live engine
//! and the Quadlet exporter diverging on which one. Failing fast here keeps
//! `config`, `up`, and `generate` in agreement.

use crate::compose::order::resolve_order;
use crate::compose::types::{ComposeFile, PortMapping, VolumeMount, VolumeType};
use crate::error::{ComposeError, Result};
use crate::ports::parse_ports;

/// Validate a parsed compose file the way `docker compose config` does.
///
/// Checks, in order: at least one service is defined; every service declares an
/// `image:` or `build:`; service names use the compose charset; published/target
/// ports are in range; every referenced named volume and network is declared at
/// the top level; and the `depends_on` graph is acyclic with no dangling
/// required dependency. Returns the first violation found.
pub fn validate_config(file: &ComposeFile) -> Result<()> {
	// An empty file, a missing `services:` key, or `services: {}` is not a valid
	// project — `docker compose config` errors with "no service selected".
	if file.services.is_empty() {
		return Err(ComposeError::Unsupported(
			"no services defined in compose file".to_string(),
		));
	}

	for (name, svc) in &file.services {
		validate_service_name(name)?;
		if svc.image.is_none() && svc.build.is_none() {
			return Err(ComposeError::NoImageOrBuild(name.clone()));
		}
		validate_ports(name, &svc.ports)?;
		validate_network_refs(name, file, svc)?;
		validate_volume_refs(name, file, svc)?;
	}

	// Reject `depends_on` cycles and dangling required dependencies, matching the
	// mutating commands (which run `resolve_order` before they start anything).
	resolve_order(file)?;
	Ok(())
}

/// Reject a service name that is empty or uses characters outside the compose
/// charset (`[a-zA-Z0-9._-]`). Spaces and punctuation like `!` are rejected.
fn validate_service_name(name: &str) -> Result<()> {
	let ok = !name.is_empty()
		&& name
			.chars()
			.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
	if ok {
		Ok(())
	} else {
		Err(ComposeError::Unsupported(format!(
			"service name {name:?} is invalid: use only ASCII letters, digits, '.', '_', '-'"
		)))
	}
}

/// Range-check every port a service publishes. `parse_ports` rejects values that
/// do not fit a `u16` (e.g. `99999`); on top of that a container or host port of
/// `0` is rejected here, since a valid published/target port is `1`–`65535`.
fn validate_ports(service: &str, ports: &[PortMapping]) -> Result<()> {
	for parsed in parse_ports(ports)? {
		if parsed.container_port == 0 || parsed.host_port == Some(0) {
			return Err(ComposeError::InvalidPort(format!(
				"service '{service}' has a port of 0; ports must be in 1-65535"
			)));
		}
	}
	Ok(())
}

/// Every network a service joins must be declared in the top-level `networks:`
/// map (the implicit `default` network is synthesized before this runs, so a
/// bare service still validates). `network_mode:` services declare no networks.
fn validate_network_refs(
	service: &str,
	file: &ComposeFile,
	svc: &crate::compose::types::Service,
) -> Result<()> {
	for net in svc.networks.names() {
		if !file.networks.contains_key(&net) {
			return Err(ComposeError::Unsupported(format!(
				"service '{service}' refers to undefined network '{net}'; declare it under the \
				 top-level 'networks:' key"
			)));
		}
	}
	Ok(())
}

/// Every *named* volume a service mounts must be declared in the top-level
/// `volumes:` map. Bind mounts (host paths) and anonymous volumes carry no
/// top-level declaration and are skipped.
fn validate_volume_refs(
	service: &str,
	file: &ComposeFile,
	svc: &crate::compose::types::Service,
) -> Result<()> {
	for mount in &svc.volumes {
		let named = match mount {
			VolumeMount::Short(s) => short_named_volume(s),
			VolumeMount::Long {
				volume_type: VolumeType::Volume,
				source: Some(src),
				..
			} => Some(src.as_str()),
			VolumeMount::Long { .. } => None,
		};
		if let Some(name) = named {
			if !file.volumes.contains_key(name) {
				return Err(ComposeError::Unsupported(format!(
					"service '{service}' refers to undefined volume '{name}'; declare it under the \
					 top-level 'volumes:' key"
				)));
			}
		}
	}
	Ok(())
}

/// Extract the named-volume reference from a short-form `source:target[:opts]`
/// mount, or `None` when it is a host-path bind or an anonymous volume.
///
/// Mirrors the engine's own classification: a source starting with `/`, `.` or
/// `~`, or a Windows drive prefix (`C:`), is a bind, not a named volume; a
/// single token with no target is an anonymous volume.
fn short_named_volume(spec: &str) -> Option<&str> {
	let (src, _rest) = spec.split_once(':')?;
	if src.is_empty()
		|| src.starts_with('/')
		|| src.starts_with('.')
		|| src.starts_with('~')
		|| is_windows_drive(src)
	{
		return None;
	}
	Some(src)
}

/// Whether `src` is exactly a Windows drive letter (e.g. `C`), meaning the colon
/// after it is part of a host path rather than the `source:target` separator.
fn is_windows_drive(src: &str) -> bool {
	let b = src.as_bytes();
	b.len() == 1 && b[0].is_ascii_alphabetic()
}

/// Validate the semantic consistency of a resolved compose file. Returns the
/// first error found, matching docker-compose's fail-at-config-time behaviour.
///
/// Run only on the interpolated file: with `--no-interpolate` the values may
/// still contain literal `${VAR}` placeholders, which cannot be meaningfully
/// range- or reference-checked.
pub(super) fn validate(file: &ComposeFile) -> Result<()> {
	validate_services(file)?;
	validate_networks(file)?;
	Ok(())
}

/// Per-service checks: the `network_mode`/`networks` mutual exclusion, every
/// network reference resolving to a declared (or external) network, and that the
/// `ports:` entries parse and are in range.
fn validate_services(file: &ComposeFile) -> Result<()> {
	for (name, service) in &file.services {
		let attached = service.networks.names();
		if service.network_mode.is_some() {
			// docker-compose: "network_mode" and "networks" cannot be combined.
			// The live engine silently honours network_mode and drops the declared
			// networks; Quadlet emits both, producing a contradictory unit.
			if !attached.is_empty() {
				return Err(ComposeError::Unsupported(format!(
					"service '{name}' sets both 'network_mode' and 'networks', which are \
					 mutually exclusive; keep one"
				)));
			}
		} else {
			// Every referenced network must be declared at the top level (or be the
			// synthesized `default`). An undefined reference is a config error in
			// docker-compose; podup otherwise prefixes it on the engine path while
			// the Quadlet exporter emits the raw name, a cross-project attach risk.
			for net in &attached {
				// `default` is the implicit project network docker-compose always
				// provides, so an explicit reference to it is valid even without a
				// top-level entry; everything else must be declared.
				if net != "default" && !file.networks.contains_key(net) {
					return Err(ComposeError::Unsupported(format!(
						"service '{name}' refers to undefined network '{net}'; declare it \
						 under the top-level 'networks:' or mark it external"
					)));
				}
			}
		}

		// Surface a malformed/out-of-range port at config time rather than letting
		// it slip through to a podman create error at run time.
		parse_ports(&service.ports)?;
	}
	Ok(())
}

/// Top-level network checks: an `external: true` network must not also carry
/// creation-time attributes (driver, IPAM, internal, …), which podman cannot
/// apply to a pre-existing network and docker-compose rejects.
fn validate_networks(file: &ComposeFile) -> Result<()> {
	for (name, cfg) in &file.networks {
		let Some(cfg) = cfg else { continue };
		if cfg.external != Some(true) {
			continue;
		}
		let mut conflicts = Vec::new();
		if cfg.driver.is_some() {
			conflicts.push("driver");
		}
		if !cfg.driver_opts.is_empty() {
			conflicts.push("driver_opts");
		}
		if cfg.internal.is_some() {
			conflicts.push("internal");
		}
		if cfg.attachable.is_some() {
			conflicts.push("attachable");
		}
		if cfg.enable_ipv6.is_some() {
			conflicts.push("enable_ipv6");
		}
		if cfg.enable_ipv4.is_some() {
			conflicts.push("enable_ipv4");
		}
		if cfg.ipam.is_some() {
			conflicts.push("ipam");
		}
		if !conflicts.is_empty() {
			return Err(ComposeError::Unsupported(format!(
				"network '{name}' is external but also sets {}; an external network is \
				 used as-is and these attributes cannot be applied to it",
				conflicts.join(", ")
			)));
		}
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{parse_str, parse_str_raw};

	fn file(yaml: &str) -> ComposeFile {
		parse_str_raw(yaml).unwrap()
	}

	fn validate_str(yaml: &str) -> crate::error::Result<()> {
		let mut file: ComposeFile = parse_str(yaml).unwrap();
		// Mirror the CLI: synthesize the implicit default network before validating
		// so a bare service is not flagged as referencing an undefined network.
		crate::compose::normalize_default_network(&mut file);
		super::validate(&file)
	}

	// validate_config (the `config` subcommand)

	#[test]
	fn empty_services_is_rejected() {
		let err = validate_config(&file("services: {}\n")).unwrap_err();
		assert!(format!("{err}").contains("no services"));
		// A file with no `services:` key at all is equally rejected.
		assert!(validate_config(&ComposeFile::default()).is_err());
	}

	#[test]
	fn missing_image_and_build_is_rejected() {
		let err = validate_config(&file("services:\n  web:\n    ports: ['80:80']\n")).unwrap_err();
		assert!(matches!(err, ComposeError::NoImageOrBuild(_)));
	}

	#[test]
	fn valid_minimal_file_passes() {
		validate_config(&file("services:\n  web:\n    image: nginx\n")).unwrap();
	}

	#[test]
	fn out_of_range_port_is_rejected() {
		let err = validate_config(&file(
			"services:\n  web:\n    image: nginx\n    ports: ['99999:80']\n",
		))
		.unwrap_err();
		assert!(matches!(err, ComposeError::InvalidPort(_)));
	}

	#[test]
	fn zero_port_is_rejected() {
		let err = validate_config(&file(
			"services:\n  web:\n    image: nginx\n    ports: ['0:80']\n",
		))
		.unwrap_err();
		assert!(matches!(err, ComposeError::InvalidPort(_)));
	}

	#[test]
	fn undefined_named_volume_is_rejected() {
		let err = validate_config(&file(
			"services:\n  web:\n    image: nginx\n    volumes: ['data:/x']\n",
		))
		.unwrap_err();
		assert!(format!("{err}").contains("undefined volume 'data'"));
	}

	#[test]
	fn declared_named_volume_passes() {
		validate_config(&file(
			"services:\n  web:\n    image: nginx\n    volumes: ['data:/x']\nvolumes:\n  data:\n",
		))
		.unwrap();
	}

	#[test]
	fn bind_and_anonymous_volumes_are_not_flagged() {
		// Host-path binds and anonymous volumes carry no top-level declaration.
		validate_config(&file(
			"services:\n  web:\n    image: nginx\n    volumes:\n      - ./host:/x\n      - /abs:/y\n      - /data\n",
		))
		.unwrap();
	}

	#[test]
	fn undefined_network_is_rejected() {
		let err = validate_config(&file(
			"services:\n  web:\n    image: nginx\n    networks: [backend]\n",
		))
		.unwrap_err();
		assert!(format!("{err}").contains("undefined network 'backend'"));
	}

	#[test]
	fn declared_network_passes() {
		validate_config(&file(
			"services:\n  web:\n    image: nginx\n    networks: [backend]\nnetworks:\n  backend:\n",
		))
		.unwrap();
	}

	#[test]
	fn invalid_service_name_is_rejected() {
		let err =
			validate_config(&file("services:\n  'bad name':\n    image: nginx\n")).unwrap_err();
		assert!(format!("{err}").contains("service name"));
	}

	#[test]
	fn dependency_cycle_is_rejected() {
		let err = validate_config(&file(
			"services:\n  a:\n    image: x\n    depends_on: [b]\n  b:\n    image: y\n    depends_on: [a]\n",
		))
		.unwrap_err();
		assert!(matches!(err, ComposeError::CircularDependency(_)));
	}

	#[test]
	fn dangling_required_dependency_is_rejected() {
		let err = validate_config(&file(
			"services:\n  web:\n    image: nginx\n    depends_on: [ghost]\n",
		))
		.unwrap_err();
		assert!(matches!(err, ComposeError::ServiceNotFound(_)));
	}

	// validate (the post-parse semantic pass)

	#[test]
	fn network_mode_with_networks_is_rejected() {
		let yaml = "services:\n  web:\n    image: x\n    network_mode: host\n    networks: [front]\nnetworks:\n  front:\n";
		let err = validate_str(yaml).unwrap_err();
		assert!(err.to_string().contains("mutually exclusive"), "got: {err}");
	}

	#[test]
	fn network_mode_alone_is_accepted() {
		let yaml = "services:\n  web:\n    image: x\n    network_mode: host\n";
		assert!(validate_str(yaml).is_ok());
	}

	#[test]
	fn undefined_network_reference_is_rejected() {
		let yaml = "services:\n  web:\n    image: x\n    networks: [missing]\n";
		let err = validate_str(yaml).unwrap_err();
		assert!(
			err.to_string().contains("undefined network 'missing'"),
			"got: {err}"
		);
	}

	#[test]
	fn declared_network_reference_is_accepted() {
		let yaml = "services:\n  web:\n    image: x\n    networks: [front]\nnetworks:\n  front:\n";
		assert!(validate_str(yaml).is_ok());
	}

	#[test]
	fn bare_service_default_network_is_accepted() {
		// No networks declared at all: the synthesized `default` must satisfy the
		// reference check, not trip it.
		let yaml = "services:\n  web:\n    image: x\n";
		assert!(validate_str(yaml).is_ok());
	}

	#[test]
	fn explicit_default_network_reference_is_accepted() {
		// `default` is the implicit project network; referencing it without a
		// top-level entry must not be flagged as undefined.
		let yaml = "services:\n  web:\n    image: x\n    networks: [default]\n";
		assert!(validate_str(yaml).is_ok());
	}

	#[test]
	fn external_network_with_internal_is_rejected() {
		let yaml = "services:\n  web:\n    image: x\n    networks: [ext]\nnetworks:\n  ext:\n    external: true\n    internal: true\n";
		let err = validate_str(yaml).unwrap_err();
		let msg = err.to_string();
		assert!(msg.contains("external"), "got: {msg}");
		assert!(msg.contains("internal"), "got: {msg}");
	}

	#[test]
	fn external_network_with_ipam_is_rejected() {
		let yaml = "services:\n  web:\n    image: x\n    networks: [ext]\nnetworks:\n  ext:\n    external: true\n    ipam:\n      config:\n        - subnet: 10.0.0.0/24\n";
		let err = validate_str(yaml).unwrap_err();
		assert!(err.to_string().contains("ipam"), "got: {err}");
	}

	#[test]
	fn plain_external_network_is_accepted() {
		let yaml = "services:\n  web:\n    image: x\n    networks: [ext]\nnetworks:\n  ext:\n    external: true\n";
		assert!(validate_str(yaml).is_ok());
	}

	#[test]
	fn external_network_with_name_only_is_accepted() {
		let yaml = "services:\n  web:\n    image: x\n    networks: [ext]\nnetworks:\n  ext:\n    external: true\n    name: shared_net\n";
		assert!(validate_str(yaml).is_ok());
	}

	#[test]
	fn out_of_range_short_port_is_rejected() {
		let yaml = "services:\n  web:\n    image: x\n    ports: ['99999:80']\n";
		assert!(validate_str(yaml).is_err());
	}

	#[test]
	fn invalid_port_protocol_is_rejected() {
		let yaml = "services:\n  web:\n    image: x\n    ports: ['80/banana']\n";
		assert!(validate_str(yaml).is_err());
	}
}
