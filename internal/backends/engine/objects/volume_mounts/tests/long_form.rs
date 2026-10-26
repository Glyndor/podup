use super::*;

#[test]
fn long_form_bind_read_only() {
	let svc = svc_with_volumes(vec![VolumeMount::Long {
		volume_type: VolumeType::Bind,
		source: Some("/host/path".into()),
		target: "/container/path".into(),
		read_only: Some(true),
		bind: None,
		volume: None,
		tmpfs: None,
		consistency: None,
	}]);
	let (mounts, _) = build_mounts_all(&svc, Path::new("/base"));
	assert_eq!(mounts.len(), 1);
	assert_eq!(mounts[0].mount_type, "bind");
	assert!(mounts[0].options.contains(&"ro".to_string()));
	assert_eq!(mounts[0].destination, "/container/path");
}

#[test]
fn long_form_bind_with_propagation() {
	let svc = svc_with_volumes(vec![VolumeMount::Long {
		volume_type: VolumeType::Bind,
		source: Some("/host".into()),
		target: "/cont".into(),
		read_only: Some(false),
		bind: Some(BindOptions {
			propagation: Some("rshared".into()),
			create_host_path: None,
			selinux: None,
		}),
		volume: None,
		tmpfs: None,
		consistency: None,
	}]);
	let (mounts, _) = build_mounts_all(&svc, Path::new("/base"));
	assert!(mounts[0].options.contains(&"rshared".to_string()));
}

#[test]
fn long_form_volume_nocopy() {
	let svc = svc_with_volumes(vec![VolumeMount::Long {
		volume_type: VolumeType::Volume,
		source: Some("myvolume".into()),
		target: "/data".into(),
		read_only: None,
		bind: None,
		volume: Some(VolumeOptions {
			nocopy: Some(true),
			..Default::default()
		}),
		tmpfs: None,
		consistency: None,
	}]);
	let (mounts, named) = build_mounts_all(&svc, Path::new("/base"));
	assert!(mounts.is_empty());
	assert_eq!(named.len(), 1);
	assert_eq!(named[0].name, "myvolume");
	assert!(named[0].options.contains(&"nocopy".to_string()));
}

/// The mount-hardening trio reaches the engine from the long form, matching
/// what the short form's raw options have always done (#1160). `false` and
/// absent both mean "not hardened": only an explicit `true` emits the flag.
#[test]
fn long_form_volume_hardening_options_forwarded() {
	let svc = svc_with_volumes(vec![VolumeMount::Long {
		volume_type: VolumeType::Volume,
		source: Some("myvolume".into()),
		target: "/data".into(),
		read_only: None,
		bind: None,
		volume: Some(VolumeOptions {
			noexec: Some(true),
			nosuid: Some(true),
			nodev: Some(false),
			..Default::default()
		}),
		tmpfs: None,
		consistency: None,
	}]);
	let (_, named) = build_mounts_all(&svc, Path::new("/base"));
	assert_eq!(named.len(), 1);
	assert!(named[0].options.contains(&"noexec".to_string()));
	assert!(named[0].options.contains(&"nosuid".to_string()));
	assert!(
		!named[0].options.contains(&"nodev".to_string()),
		"an explicit false must not emit the flag"
	);
}

#[test]
fn long_form_volume_subpath_forwarded() {
	let svc = svc_with_volumes(vec![VolumeMount::Long {
		volume_type: VolumeType::Volume,
		source: Some("myvolume".into()),
		target: "/data".into(),
		read_only: None,
		bind: None,
		volume: Some(VolumeOptions {
			subpath: Some("nested/dir".into()),
			..Default::default()
		}),
		tmpfs: None,
		consistency: None,
	}]);
	let (_, named) = build_mounts_all(&svc, Path::new("/base"));
	assert_eq!(named.len(), 1);
	assert_eq!(named[0].sub_path.as_deref(), Some("nested/dir"));
}
