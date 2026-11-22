//! The Quadlet keys podup can emit, per unit type, as the floor's Quadlet reads
//! them.
//!
//! podup requires Podman 6.2.0 or newer (the floor). Quadlet drops a whole unit
//! at daemon-reload when it meets a key it does not know ("unsupported key"),
//! so a key newer than the floor must never reach a rendered unit. The
//! allowlists below hold every `[Container]`, `[Pod]`, `[Network]`, `[Volume]`
//! and `[Build]` key the Quadlet of Podman 6.2.0 accepts, and the floor tests
//! check that every `Key=` line the renderer writes is in them. The sources are
//! the 6.2.0 man pages (`podman-systemd.unit` and the per-unit pages) and, for
//! the keys those pages leave out, the generator itself: a key was added to the
//! lists only after `quadlet -dryrun` accepted it (2026-11-22). Those keys are
//! `[Container]` `AppArmor` and `Umask`, `[Pod]` `StopTimeout`, and `[Build]`
//! `BuildArg` and `IgnoreFile`.
//!
//! The whole module is gated to tests: production code never reads the lists.
//! A key newer than the floor needs a new floor, so the guard names the key
//! instead of letting the unit through.

/// A Quadlet unit type, narrowed to the ones we actually emit. The same Rust
/// type covers `[Container]`, `[Pod]`, `[Network]`, `[Volume]` and `[Build]`
/// keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum UnitType {
	/// `.container` units (the `[Container]` section).
	Container,
	/// `.pod` units (the `[Pod]` section).
	Pod,
	/// `.network` units (the `[Network]` section).
	Network,
	/// `.volume` units (the `[Volume]` section).
	Volume,
	/// `.build` units (the `[Build]` section).
	Build,
}

impl UnitType {
	/// The file-name suffix the unit type corresponds to, with the leading
	/// dot.
	pub(crate) const fn suffix(self) -> &'static str {
		match self {
			UnitType::Container => ".container",
			UnitType::Pod => ".pod",
			UnitType::Network => ".network",
			UnitType::Volume => ".volume",
			UnitType::Build => ".build",
		}
	}
}

/// Whether Podman 6.2.0's Quadlet reads `key` in a unit of this type.
/// `PodmanArgs=` and `GlobalArgs=` exist on every unit type.
pub(crate) fn is_at_floor(unit_type: UnitType, key: &str) -> bool {
	if matches!(key, "PodmanArgs" | "GlobalArgs" | "ContainersConfModule") {
		return true;
	}
	let keys: &[&str] = match unit_type {
		UnitType::Container => &[
			"AddCapability",
			"AddDevice",
			"AddHost",
			"Annotation",
			"AppArmor",
			"AutoUpdate",
			"CgroupsMode",
			"ContainerName",
			"DNS",
			"DNSOption",
			"DNSSearch",
			"DropCapability",
			"Entrypoint",
			"Environment",
			"EnvironmentFile",
			"EnvironmentHost",
			"Exec",
			"ExposeHostPort",
			"GIDMap",
			"Group",
			"GroupAdd",
			"HealthCmd",
			"HealthInterval",
			"HealthLogDestination",
			"HealthMaxLogCount",
			"HealthMaxLogSize",
			"HealthOnFailure",
			"HealthRetries",
			"HealthStartPeriod",
			"HealthStartupCmd",
			"HealthStartupInterval",
			"HealthStartupRetries",
			"HealthStartupSuccess",
			"HealthStartupTimeout",
			"HealthTimeout",
			"HostName",
			"HttpProxy",
			"Image",
			"ImageVolume",
			"IP",
			"IP6",
			"Label",
			"LogDriver",
			"LogOpt",
			"Mask",
			"Memory",
			"Mount",
			"Network",
			"NetworkAlias",
			"NoNewPrivileges",
			"Notify",
			"PidsLimit",
			"Pod",
			"PublishPort",
			"Pull",
			"ReadOnly",
			"ReadOnlyTmpfs",
			"ReloadCmd",
			"ReloadSignal",
			"Retry",
			"RetryDelay",
			"Rootfs",
			"RunInit",
			"SeccompProfile",
			"Secret",
			"SecurityLabelDisable",
			"SecurityLabelFileType",
			"SecurityLabelLevel",
			"SecurityLabelNested",
			"SecurityLabelType",
			"ShmSize",
			"StartWithPod",
			"StopSignal",
			"StopTimeout",
			"SubGIDMap",
			"SubUIDMap",
			"Sysctl",
			"Timezone",
			"Tmpfs",
			"UIDMap",
			"Ulimit",
			"Umask",
			"Unmask",
			"User",
			"UserNS",
			"Volume",
			"WorkingDir",
		],
		UnitType::Pod => &[
			"AddHost",
			"DNS",
			"DNSOption",
			"DNSSearch",
			"ExitPolicy",
			"GIDMap",
			"HostName",
			"IP",
			"IP6",
			"Label",
			"Network",
			"NetworkAlias",
			"PodName",
			"PublishPort",
			"ServiceName",
			"ShmSize",
			"StopTimeout",
			"SubGIDMap",
			"SubUIDMap",
			"UIDMap",
			"UserNS",
			"Volume",
		],
		UnitType::Network => &[
			"DisableDNS",
			"DNS",
			"Driver",
			"Gateway",
			"InterfaceName",
			"Internal",
			"IPAMDriver",
			"IPRange",
			"IPv6",
			"Label",
			"NetworkDeleteOnStop",
			"NetworkName",
			"Options",
			"Subnet",
		],
		UnitType::Volume => &[
			"Copy",
			"Device",
			"Driver",
			"Group",
			"Image",
			"Label",
			"Options",
			"Type",
			"User",
			"VolumeName",
		],
		UnitType::Build => &[
			"Annotation",
			"Arch",
			"AuthFile",
			"BuildArg",
			"DNS",
			"DNSOption",
			"DNSSearch",
			"Environment",
			"File",
			"ForceRM",
			"GroupAdd",
			"IgnoreFile",
			"ImageTag",
			"Label",
			"Network",
			"Pull",
			"Retry",
			"RetryDelay",
			"Secret",
			"SetWorkingDirectory",
			"Target",
			"TLSVerify",
			"Variant",
			"Volume",
		],
	};
	keys.contains(&key)
}

/// Whether a section header names a Quadlet-managed section. The `[Unit]`
/// and `[Install]`/`[Service]` sections are passed through to systemd,
/// not to Quadlet, so a key in those sections is not a Quadlet key and
/// must not be checked against the floor.
fn is_quadlet_section(name: &str) -> bool {
	matches!(name, "Container" | "Pod" | "Network" | "Volume" | "Build")
}

/// Collect every distinct `Key=` (excluding `PodmanArgs`/`GlobalArgs`) from
/// the body of a single rendered unit. The floor test asserts each one is a
/// key the floor's Quadlet reads. Lines outside a
/// Quadlet-managed section (`[Unit]`, `[Install]`, `[Service]`) are
/// ignored: those go straight to systemd, not to Quadlet, so a `Description=`
/// or `Restart=` there is not a Quadlet key and must not be checked against the floor.
pub(crate) fn keys_in_unit(contents: &str) -> Vec<String> {
	let mut out: Vec<String> = Vec::new();
	let mut in_quadlet_section = false;
	for line in contents.lines() {
		let trimmed = line.trim_start();
		if trimmed.starts_with('#') {
			continue;
		}
		if let Some(header) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
			in_quadlet_section = is_quadlet_section(header);
			continue;
		}
		if !in_quadlet_section {
			continue;
		}
		if let Some(eq) = line.find('=') {
			let key = line[..eq].trim();
			if matches!(key, "PodmanArgs" | "GlobalArgs") {
				continue;
			}
			if !out.iter().any(|k| k == key) {
				out.push(key.to_string());
			}
		}
	}
	out
}

/// Resolve the unit type a rendered file belongs to from its file name. A
/// file name ending in `.container` is a container unit; `.pod` is a pod;
/// etc. Used by the floor and coverage tests to fold a `QuadletUnit` into a
/// `UnitType` without re-deriving the suffix twice.
pub(crate) fn unit_type_from_filename(filename: &str) -> UnitType {
	if filename.ends_with(UnitType::Build.suffix()) {
		UnitType::Build
	} else if filename.ends_with(UnitType::Pod.suffix()) {
		UnitType::Pod
	} else if filename.ends_with(UnitType::Network.suffix()) {
		UnitType::Network
	} else if filename.ends_with(UnitType::Volume.suffix()) {
		UnitType::Volume
	} else {
		UnitType::Container
	}
}

#[cfg(test)]
#[path = "min_podman_tests.rs"]
mod tests;
