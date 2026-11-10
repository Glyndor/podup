use super::*;

// --- render_volume --------------------------------------------------------

/// The compose base directory the renderer resolves a relative source against.
fn base() -> std::path::PathBuf {
	std::path::PathBuf::from("/srv/app")
}

/// The external volumes the compose file declares, keyed by their compose key.
fn external_volumes() -> ExternalNames<'static> {
	ExternalNames::from([("data", "shared_data")])
}

// The expected paths are POSIX paths joined with a POSIX base.
#[cfg(unix)]
#[test]
fn render_volume_short_declared_uses_dot_volume_with_options() {
	let out = render_volume(
		&VolumeMount::Short("data:/app:ro".into()),
		"proj",
		&["data"],
		&external_volumes(),
		&base(),
	);
	// A declared named volume becomes `<project>-<name>.volume:<target>:<opts>`.
	assert_eq!(out, "proj-data.volume:/app:ro");
	// An undeclared source is a host path, and is made absolute against the
	// compose base directory: systemd would resolve it against the unit
	// file's own directory instead.
	assert_eq!(
		render_volume(
			&VolumeMount::Short("./host:/app".into()),
			"proj",
			&["data"],
			&external_volumes(),
			&base()
		),
		"/srv/app/./host:/app"
	);
}

#[test]
fn render_volume_external_source_uses_the_name_it_really_has() {
	// An external volume is named by the name it really has on the host,
	// which a compose `name:` may have changed. The compose key would
	// point at a resource that does not exist.
	let out = render_volume(
		&VolumeMount::Short("data:/app".into()),
		"proj",
		&[],
		&external_volumes(),
		&base(),
	);
	assert_eq!(out, "shared_data:/app");
}

#[test]
fn render_volume_short_colonless_entry_is_an_anonymous_volume() {
	// A colon-less entry names an in-container path, not a host source, so
	// there is nothing to resolve against the base directory.
	assert_eq!(
		render_volume(
			&VolumeMount::Short("/data".into()),
			"proj",
			&[],
			&external_volumes(),
			&base()
		),
		"/data"
	);
}

#[test]
fn render_volume_long_collects_options_and_handles_empty_source() {
	use crate::compose::types::VolumeOptions;
	let nocopy = VolumeMount::Long {
		volume_type: VolumeType::Volume,
		source: Some("vol".into()),
		target: "/data".into(),
		read_only: Some(true),
		bind: None,
		volume: Some(VolumeOptions {
			nocopy: Some(true),
			..Default::default()
		}),
		tmpfs: None,
		consistency: None,
	};
	// Declared → project-prefixed `.volume` suffix; ro + nocopy folded into
	// the options field.
	assert_eq!(
		render_volume(&nocopy, "proj", &["vol"], &external_volumes(), &base()),
		"proj-vol.volume:/data:ro,nocopy"
	);

	// The hardening trio survives the Quadlet export (#1160): a compose
	// file that hardens a mount must not export a unit that does not.
	let hardened = VolumeMount::Long {
		volume_type: VolumeType::Volume,
		source: Some("vol".into()),
		target: "/data".into(),
		read_only: None,
		bind: None,
		volume: Some(VolumeOptions {
			noexec: Some(true),
			nosuid: Some(true),
			nodev: Some(true),
			..Default::default()
		}),
		tmpfs: None,
		consistency: None,
	};
	assert_eq!(
		render_volume(&hardened, "proj", &["vol"], &external_volumes(), &base()),
		"proj-vol.volume:/data:noexec,nosuid,nodev"
	);

	// An empty source renders as just the target (anonymous mount).
	let anon = VolumeMount::Long {
		volume_type: VolumeType::Volume,
		source: None,
		target: "/scratch".into(),
		read_only: None,
		bind: None,
		volume: None,
		tmpfs: None,
		consistency: None,
	};
	assert_eq!(
		render_volume(&anon, "proj", &[], &external_volumes(), &base()),
		"/scratch"
	);
}
