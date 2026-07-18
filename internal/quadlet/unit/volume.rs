//! Build the `.volume` unit for a declared named volume.

use crate::compose::types::VolumeConfig;

use super::{owner_marker, sorted_label_pairs, unit_stem, QuadletUnit, Section};

/// Build the `.volume` unit for one declared named volume. Emits a single
/// `[Volume]` section (VolumeName, then driver/driver-opts/labels), always
/// appending the `podup.project` ownership label. No `[Install]` section is
/// written: `.volume` units are pulled in as dependencies of the `.container`
/// units that reference them.
pub(crate) fn volume_unit(name: &str, project: &str, config: Option<&VolumeConfig>) -> QuadletUnit {
	let mut vol = Section::new("Volume");
	// A custom `name:` overrides Podman's resource name; Quadlet uses the literal
	// value (no prefix) when `VolumeName=` is set explicitly.
	let vol_name = config
		.and_then(|c| c.name.clone())
		.unwrap_or_else(|| format!("{project}_{name}"));
	vol.add("VolumeName", vol_name);
	if let Some(cfg) = config {
		if let Some(driver) = &cfg.driver {
			vol.add("Driver", driver.clone());
		}
		// The built-in `local` driver's opts map onto dedicated Quadlet keys:
		// `type`→Type=, `device`→Device=, `o`→Options= (already a comma-separated
		// mount-option string). Quadlet rejects Options= without a Device=, so any
		// other driver option has no [Volume] key and passes through PodmanArgs=.
		for (key, val) in sorted_label_pairs(cfg.driver_opts.clone()) {
			match key.as_str() {
				"type" => vol.add("Type", val),
				"device" => vol.add("Device", val),
				"o" => vol.add("Options", val),
				_ => vol.add("PodmanArgs", format!("--opt {key}={val}")),
			}
		}
		for (key, val) in sorted_label_pairs(cfg.labels.to_map()) {
			vol.add("Label", format!("{key}={val}"));
		}
	}
	// Ownership label, mirroring the live engine: tag every generated volume with
	// its project so it is traceable/removable by label like a running one.
	vol.add("Label", format!("podup.project={project}"));
	// No [Install] section: the spec defines none for `.volume` units, which are
	// pulled in automatically as dependencies of the `.container` units that use
	// them. Only `.container` units carry [Install].
	//
	// The unforgeable ownership marker comes first, as its own comment line;
	// see `owner_marker` for why it must stay separate from the `Label=` line.
	let mut contents = owner_marker(project);
	contents.push_str(&vol.render());
	QuadletUnit {
		filename: format!("{}.volume", unit_stem(project, name)),
		contents,
	}
}
