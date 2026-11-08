use super::*;

// --- render_tmpfs_mount ---------------------------------------------------

#[test]
fn render_tmpfs_mount_with_and_without_options() {
	use crate::compose::types::TmpfsOptions;
	let with_opts = VolumeMount::Long {
		volume_type: VolumeType::Tmpfs,
		source: None,
		target: "/cache".into(),
		read_only: None,
		bind: None,
		volume: None,
		tmpfs: Some(TmpfsOptions {
			size: Some(4096),
			mode: Some(0o700),
		}),
		consistency: None,
	};
	assert_eq!(
		render_tmpfs_mount(&with_opts).as_deref(),
		Some("/cache:size=4096,mode=700")
	);

	// A tmpfs mount with no size/mode renders just the target.
	let bare = VolumeMount::Long {
		volume_type: VolumeType::Tmpfs,
		source: None,
		target: "/run".into(),
		read_only: None,
		bind: None,
		volume: None,
		tmpfs: None,
		consistency: None,
	};
	assert_eq!(render_tmpfs_mount(&bare).as_deref(), Some("/run"));

	// A non-tmpfs mount returns None so the caller emits a normal Volume=.
	assert!(render_tmpfs_mount(&VolumeMount::Short("a:/b".into())).is_none());
}

#[test]
fn render_tmpfs_mount_bare_decimal_mode_is_not_octal_re_encoded() {
	// Regression for #917: a long-form tmpfs with a bare `mode: 700` must
	// render `mode=700`, not `mode=1274` (700 octal-encoded a second time).
	let yaml = "type: tmpfs\ntarget: /run\ntmpfs:\n  mode: 700\n";
	let mount: VolumeMount = serde_yaml::from_str(yaml).expect("parse tmpfs mount");
	assert_eq!(render_tmpfs_mount(&mount).as_deref(), Some("/run:mode=700"));
}
