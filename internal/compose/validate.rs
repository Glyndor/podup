//! Semantic validation of a resolved compose file.
//!
//! Parsing only checks that the YAML deserializes; it never rejects a file that
//! is syntactically valid but semantically contradictory (e.g. a service that
//! declares both `network_mode` and `networks`, or a reference to a network that
//! was never defined). docker-compose errors on these at config time; podup used
//! to accept them and then silently pick one interpretation, with the live
//! engine and the Quadlet exporter diverging on which one. This pass closes that
//! gap by failing fast with a clear message, so `config`, `up`, and `generate`
//! all agree and no surface silently drops or mistranslates the configuration.

use super::types::ComposeFile;
use crate::error::{ComposeError, Result};
use crate::ports::parse_ports;

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
	use crate::compose::types::ComposeFile;
	use crate::parse_str;

	fn validate_str(yaml: &str) -> crate::error::Result<()> {
		let mut file: ComposeFile = parse_str(yaml).unwrap();
		// Mirror the CLI: synthesize the implicit default network before validating
		// so a bare service is not flagged as referencing an undefined network.
		crate::compose::normalize_default_network(&mut file);
		super::validate(&file)
	}

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
