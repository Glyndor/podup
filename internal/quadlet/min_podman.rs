//! The minimum Podman release each Quadlet key we can emit first appeared in.
//!
//! podup promises Podman 5.0 or newer (the floor), and Quadlet silently drops
//! a whole unit at daemon-reload if any of its keys were added after the
//! running Quadlet release. The table below is the regression guard against
//! that: `floor_required(key)` returns the lowest Podman release that knows
//! the key, and a test verifies that every `Key=` line we render is either
//! at the floor or absent. New keys above the floor must add a row, or the
//! floor test names the missing entry.
//!
//! Sources: the Podman 5.0 reference at `podman-systemd.unit.5.md` (a key
//! listed there for its unit type is 5.0), the 5.2 reference for the
//! `[Build]` section (the unit type landed in 5.2.0, so every key there is
//! 5.2.0), and the per-release notes for every later release where a key was
//! introduced (`Retry=`/`RetryDelay=` arrived in 5.5.0,
//! `BuildArg=`/`IgnoreFile=` in 5.7.0). Entries left out of the table are
//! entries the maintainer should not have rendered at all.
//!
//! The whole module is gated to tests: production code never reads the
//! table, only the test infrastructure does. The renderer already routes
//! every key listed in the table through `PodmanArgs=`, so the table is
//! the regression guard against a future emitter that forgets to.

use std::collections::BTreeMap;

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
	/// `.build` units (the `[Build]` section; the whole unit type appeared in
	/// 5.2.0, so every entry below is 5.2.0).
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

/// The floor: Podman 5.0.0. Keys whose `floor_required` returns a version
/// above this are rendered as `PodmanArgs=` flags instead.
const FLOOR: PodmanVersion = PodmanVersion::new(5, 0);

/// A Podman release, encoded as major + minor. Patch numbers do not gate
/// Quadlet keys, so the table records `5.x` versions rather than the
/// precise `5.x.y` a release note might cite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct PodmanVersion {
	pub major: u16,
	pub minor: u16,
}

impl PodmanVersion {
	pub(crate) const fn new(major: u16, minor: u16) -> Self {
		Self { major, minor }
	}

	/// Compact display, `5.0` / `5.3`. Used by the floor test failure
	/// message so the regression report names the offending version.
	pub(crate) const fn fmt(self) -> &'static str {
		match (self.major, self.minor) {
			(5, 0) => "5.0",
			(5, 1) => "5.1",
			(5, 2) => "5.2",
			(5, 3) => "5.3",
			(5, 4) => "5.4",
			(5, 5) => "5.5",
			(5, 6) => "5.6",
			_ => "?",
		}
	}
}

/// Build the static `(unit_type, key) -> floor_version` table by hand.
///
/// Reading the 5.0 reference for each section first, then walking the
/// release notes for every later release where a key was introduced, is the
/// only way to produce a row per emitted key: there is no machine-readable
/// inventory of Quadlet keys in podman-source. A typo here is caught by
/// the floor test (the key it names then has no floor) and by the
/// coverage test (every emitted key must resolve here).
fn min_podman_table() -> BTreeMap<(UnitType, &'static str), PodmanVersion> {
	let mut t: BTreeMap<(UnitType, &'static str), PodmanVersion> = BTreeMap::new();

	// `[Container]` keys added after 5.0. Every other `[Container]` key
	// we emit is in the 5.0 reference and so exempt from the floor.
	t.insert((UnitType::Container, "GroupAdd"), PodmanVersion::new(5, 1));
	t.insert((UnitType::Container, "LogOpt"), PodmanVersion::new(5, 2));
	t.insert(
		(UnitType::Container, "StopSignal"),
		PodmanVersion::new(5, 2),
	);
	t.insert(
		(UnitType::Container, "NetworkAlias"),
		PodmanVersion::new(5, 2),
	);
	t.insert((UnitType::Container, "AddHost"), PodmanVersion::new(5, 3));

	// `[Pod]` keys added after 5.0.
	t.insert((UnitType::Pod, "AddHost"), PodmanVersion::new(5, 3));
	t.insert((UnitType::Pod, "Label"), PodmanVersion::new(5, 6));

	// `[Build]` unit type itself appeared in 5.2.0 ("Quadlet now has support
	// for `.build` files"). The keys that landed with the unit type are
	// listed in `is_5_2_build_key` below; the table here only carries keys
	// that arrived in a later release and so require a row to be
	// floor-compliant.
	t.insert((UnitType::Build, "Retry"), PodmanVersion::new(5, 5));
	t.insert((UnitType::Build, "RetryDelay"), PodmanVersion::new(5, 5));
	t.insert((UnitType::Build, "BuildArg"), PodmanVersion::new(5, 7));
	t.insert((UnitType::Build, "IgnoreFile"), PodmanVersion::new(5, 7));

	t
}

/// The minimum Podman release that knows the given key on the given unit
/// type. Returns `None` when the key is in the 5.0 reference for that unit
/// type (the floor); `PodmanArgs=` and `GlobalArgs=` are also exempt, since
/// they appear on every unit type in 5.0.
pub(crate) fn floor_required(unit_type: UnitType, key: &str) -> Option<PodmanVersion> {
	// Exemptions: keys that exist on every unit type in 5.0, so they do
	// not gate the floor. They are listed here rather than in the table
	// to keep the table readable as a list of post-5.0 additions.
	if matches!(key, "PodmanArgs" | "GlobalArgs") {
		return None;
	}
	min_podman_table().get(&(unit_type, key)).copied()
}

/// The 5.0 reference: keys podup may render on each unit type that the
/// Podman 5.0 systemd.unit man page already lists. Anything emitted that
/// is NOT in this set must be either in `min_podman_table` (because it
/// post-dates 5.0) or absent from the renderer entirely. The coverage
/// test combines these two sets into the floor-compatibility check.
///
/// Listed by hand from `podman-systemd.unit.5.0.5.md` (the snapshot under
/// `scratchpad/q1972/`). `.build` is absent on purpose: the unit type
/// itself appeared in 5.2.0, so its keys cannot be 5.0; they live in
/// `is_5_2_build_key` instead.
fn is_5_0_key(unit_type: UnitType, key: &str) -> bool {
	if matches!(key, "PodmanArgs" | "GlobalArgs") {
		return true;
	}
	match unit_type {
		UnitType::Container => {
			matches!(
				key,
				"AddCapability"
					| "AddDevice" | "Annotation"
					| "AutoUpdate" | "ContainerName"
					| "ContainersConfModule"
					| "DNS" | "DNSOption"
					| "DNSSearch" | "DropCapability"
					| "Entrypoint" | "Environment"
					| "EnvironmentFile"
					| "EnvironmentHost"
					| "Exec" | "ExposeHostPort"
					| "GIDMap" | "Group"
					| "HealthCmd" | "HealthInterval"
					| "HealthOnFailure"
					| "HealthRetries"
					| "HealthStartPeriod"
					| "HealthStartupCmd"
					| "HealthStartupInterval"
					| "HealthStartupRetries"
					| "HealthStartupSuccess"
					| "HealthStartupTimeout"
					| "HealthTimeout"
					| "HostName" | "Image"
					| "IP" | "IP6" | "Label"
					| "LogDriver" | "Mask"
					| "Mount" | "Network"
					| "NoNewPrivileges"
					| "Notify" | "PidsLimit"
					| "Pod" | "PublishPort"
					| "Pull" | "ReadOnly"
					| "ReadOnlyTmpfs"
					| "Rootfs" | "RunInit"
					| "SeccompProfile"
					| "Secret" | "SecurityLabelDisable"
					| "SecurityLabelFileType"
					| "SecurityLabelLevel"
					| "SecurityLabelNested"
					| "SecurityLabelType"
					| "ShmSize" | "StopTimeout"
					| "SubGIDMap" | "SubUIDMap"
					| "Sysctl" | "Timezone"
					| "Tmpfs" | "UIDMap"
					| "Ulimit" | "Unmask"
					| "User" | "UserNS"
					| "Volume" | "WorkingDir"
			)
		}
		UnitType::Pod => matches!(
			key,
			"ContainersConfModule" | "Network" | "PodName" | "PublishPort" | "Volume"
		),
		UnitType::Network => matches!(
			key,
			"ContainersConfModule"
				| "DisableDNS"
				| "DNS" | "Driver"
				| "Gateway" | "IPAMDriver"
				| "IPRange" | "IPv6"
				| "Internal" | "Label"
				| "NetworkName"
				| "Options" | "Subnet"
		),
		UnitType::Volume => matches!(
			key,
			"ContainersConfModule"
				| "Copy" | "Device"
				| "Driver" | "Group"
				| "Image" | "Label"
				| "Options" | "Type"
				| "User" | "VolumeName"
		),
		// `.build` did not exist in 5.0; the unit type itself was added
		// in 5.2.0. Its keys live in `is_5_2_build_key`.
		UnitType::Build => false,
	}
}

/// The 5.2 reference for `[Build]` units: every key the `.build` section
/// carried when the unit type landed in Podman 5.2.0. Listed by hand from
/// `podman-systemd.unit.5.md` tagged `v5.2.0` (the snapshot under
/// `scratchpad/q1972/podman-5.2-systemd.unit.md`). A key the renderer
/// emits on `[Build]` that is NOT in this set must be either in
/// `min_podman_table` (because it was added in 5.5.0 or 5.7.0) or absent
/// from the renderer entirely. `PodmanArgs`/`GlobalArgs` are exempt by
/// the same rule as everywhere else.
fn is_5_2_build_key(key: &str) -> bool {
	matches!(
		key,
		"Annotation"
			| "Arch" | "AuthFile"
			| "ContainersConfModule"
			| "DNS" | "DNSOption"
			| "DNSSearch"
			| "Environment"
			| "File" | "ForceRM"
			| "GroupAdd"
			| "ImageTag"
			| "Label" | "Network"
			| "Pull" | "Secret"
			| "SetWorkingDirectory"
			| "Target"
			| "TLSVerify"
			| "Variant"
			| "Volume"
	)
}

/// True when the (unit_type, key) pair is at the floor: in the floor
/// allowlist (`is_5_0_key` for `[Container]`/`[Pod]`/`[Network]`/`[Volume]`,
/// `is_5_2_build_key` for `[Build]`) or is a floor-exempt passthrough
/// (`PodmanArgs`/`GlobalArgs`). Distinct from [`is_floor_compliant`],
/// which also accepts keys that landed in a later release and are tracked
/// in the post-floor table: this function is the strict "at the floor"
/// check used by the source-inventory test, which sees every literal key
/// the renderer passes to the unit builder (regardless of whether it
/// would be routed through `PodmanArgs=` at render time).
pub(crate) fn is_at_floor(unit_type: UnitType, key: &str) -> bool {
	if matches!(key, "PodmanArgs" | "GlobalArgs") {
		return true;
	}
	match unit_type {
		UnitType::Build => is_5_2_build_key(key),
		_ => is_5_0_key(unit_type, key),
	}
}

/// True when the (unit_type, key) pair is allowed by the floor: at the
/// floor (`is_at_floor`), in the post-5.0/5.2 table, or exempted. The
/// `.build` unit type is gated by its own allowlist (`is_5_2_build_key`,
/// the keys the unit type landed with in 5.2.0): every emitted `.build`
/// key must be in that set, in the post-5.2 table, or be a
/// floor-exempt passthrough (`PodmanArgs`/`GlobalArgs`). A future emitter
/// that writes `Retry=` directly to a `.build` unit fails the floor test
/// with `Podman 5.5.0` named; the per-project warning in `mod.rs` still
/// gates the unit type itself.
pub(crate) fn is_floor_compliant(unit_type: UnitType, key: &str) -> bool {
	is_at_floor(unit_type, key) || min_podman_table().contains_key(&(unit_type, key))
}

/// The podup floor: Podman 5.0.0. Keys whose `floor_required` returns a
/// version above this are rendered as `PodmanArgs=` flags instead.
pub(crate) const fn floor() -> PodmanVersion {
	FLOOR
}

/// Whether a section header names a Quadlet-managed section. The `[Unit]`
/// and `[Install]`/`[Service]` sections are passed through to systemd,
/// not to Quadlet, so a key in those sections is not a Quadlet key and
/// must not gate the floor.
fn is_quadlet_section(name: &str) -> bool {
	matches!(name, "Container" | "Pod" | "Network" | "Volume" | "Build")
}

/// Collect every distinct `Key=` (excluding `PodmanArgs`/`GlobalArgs`) from
/// the body of a single rendered unit. The floor test asserts each one is
/// either at the floor or registered in the table; the coverage test
/// asserts each one is registered somewhere. Lines outside a
/// Quadlet-managed section (`[Unit]`, `[Install]`, `[Service]`) are
/// ignored: those go straight to systemd, not to Quadlet, so a `Description=`
/// or `Restart=` there is not a Quadlet key and must not gate the floor.
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
