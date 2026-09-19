use super::{build_mounts_all, ensure_bind_source, BindSource};
use crate::compose::types::{BindOptions, Service, VolumeMount, VolumeOptions, VolumeType};
use std::path::Path;

fn svc_with_volumes(vols: Vec<VolumeMount>) -> Service {
	Service {
		volumes: vols,
		..Default::default()
	}
}

#[test]
fn short_form_bind_passthrough() {
	let svc = svc_with_volumes(vec![VolumeMount::Short("./data:/app/data".into())]);
	let (mounts, named) = build_mounts_all(&svc, Path::new("/base"));
	assert_eq!(mounts.len(), 1);
	assert!(named.is_empty());
	assert_eq!(mounts[0].mount_type, "bind");
	assert_eq!(mounts[0].destination, "/app/data");
}

#[test]
fn short_form_named_volume() {
	let svc = svc_with_volumes(vec![VolumeMount::Short("myvolume:/data".into())]);
	let (mounts, named) = build_mounts_all(&svc, Path::new("/base"));
	assert!(mounts.is_empty());
	assert_eq!(named.len(), 1);
	assert_eq!(named[0].name, "myvolume");
	assert_eq!(named[0].dest, "/data");
}

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

#[test]
fn npipe_type_becomes_npipe_mount() {
	// A long-form `type: npipe` mount maps straight to an npipe OCI mount,
	// carrying its source/target with no extra options.
	let svc = svc_with_volumes(vec![VolumeMount::Long {
		volume_type: VolumeType::Npipe,
		source: Some(r"\\.\pipe\docker_engine".into()),
		target: r"\\.\pipe\docker_engine".into(),
		read_only: None,
		bind: None,
		volume: None,
		tmpfs: None,
		consistency: None,
	}]);
	let (mounts, named) = build_mounts_all(&svc, Path::new("/base"));
	assert!(named.is_empty());
	assert_eq!(mounts.len(), 1);
	assert_eq!(mounts[0].mount_type, "npipe");
	assert_eq!(mounts[0].source.as_deref(), Some(r"\\.\pipe\docker_engine"));
	assert!(mounts[0].options.is_empty());
}

#[test]
fn cluster_type_becomes_cluster_mount() {
	let svc = svc_with_volumes(vec![VolumeMount::Long {
		volume_type: VolumeType::Cluster,
		source: Some("my-cluster-vol".into()),
		target: "/data".into(),
		read_only: None,
		bind: None,
		volume: None,
		tmpfs: None,
		consistency: None,
	}]);
	let (mounts, _) = build_mounts_all(&svc, Path::new("/base"));
	assert_eq!(mounts.len(), 1);
	assert_eq!(mounts[0].mount_type, "cluster");
	assert_eq!(mounts[0].destination, "/data");
	assert_eq!(mounts[0].source.as_deref(), Some("my-cluster-vol"));
}

#[test]
fn tmpfs_type_becomes_tmpfs_mount() {
	use crate::compose::types::TmpfsOptions;
	let svc = svc_with_volumes(vec![VolumeMount::Long {
		volume_type: VolumeType::Tmpfs,
		source: None,
		target: "/tmp/cache".into(),
		read_only: None,
		bind: None,
		volume: None,
		tmpfs: Some(TmpfsOptions {
			size: Some(65536),
			mode: Some(0o700),
		}),
		consistency: None,
	}]);
	let (mounts, _) = build_mounts_all(&svc, Path::new("/base"));
	assert_eq!(mounts.len(), 1);
	assert_eq!(mounts[0].mount_type, "tmpfs");
	assert_eq!(mounts[0].destination, "/tmp/cache");
	assert!(mounts[0].options.iter().any(|o| o.starts_with("size=")));
	assert!(mounts[0].options.iter().any(|o| o.starts_with("mode=")));
}

#[test]
fn create_host_path_creates_directory() {
	let dir = tempfile::tempdir().unwrap();
	let rel = "subdir/nested";
	let svc = svc_with_volumes(vec![VolumeMount::Long {
		volume_type: VolumeType::Bind,
		source: Some(rel.into()),
		target: "/cont".into(),
		read_only: None,
		bind: Some(BindOptions {
			propagation: None,
			create_host_path: Some(true),
			selinux: None,
		}),
		volume: None,
		tmpfs: None,
		consistency: None,
	}]);
	build_mounts_all(&svc, dir.path());
	assert!(dir.path().join(rel).exists());
}

#[test]
fn short_form_bind_creates_missing_host_path() {
	// A short-form bind whose relative source is missing must have its host
	// directory created (compose-spec implies create_host_path for short
	// syntax), anchored to the project base dir, not left to fail with a raw
	// podman statfs 500.
	let dir = tempfile::tempdir().unwrap();
	let rel = "missing-dir";
	let svc = svc_with_volumes(vec![VolumeMount::Short(format!("./{rel}:/app/data"))]);
	let (mounts, named) = build_mounts_all(&svc, dir.path());
	assert!(named.is_empty());
	assert_eq!(mounts.len(), 1);
	assert_eq!(mounts[0].mount_type, "bind");
	assert!(
		dir.path().join(rel).is_dir(),
		"short-form bind source directory should be created"
	);
}

#[test]
fn top_level_tmpfs_shorthand() {
	use crate::compose::types::StringOrList;
	let svc = Service {
		tmpfs: StringOrList::List(vec!["/tmp".into(), "/run".into()]),
		..Default::default()
	};
	let (mounts, _) = build_mounts_all(&svc, Path::new("/base"));
	assert_eq!(mounts.len(), 2);
	assert_eq!(mounts[0].mount_type, "tmpfs");
	assert_eq!(mounts[0].destination, "/tmp");
	assert_eq!(mounts[1].destination, "/run");
}

#[test]
fn top_level_tmpfs_single_string() {
	use crate::compose::types::StringOrList;
	let svc = Service {
		tmpfs: StringOrList::Single("/tmp".into()),
		..Default::default()
	};
	let (mounts, _) = build_mounts_all(&svc, Path::new("/base"));
	assert_eq!(mounts.len(), 1);
	assert_eq!(mounts[0].mount_type, "tmpfs");
	assert_eq!(mounts[0].destination, "/tmp");
}

/// Existing regular file at the source path: the helper's decision must be
/// `Present`, the file must keep its bytes, and nothing else must be
/// created at or beside it.
#[test]
fn ensure_bind_source_existing_file_is_present() {
	let dir = tempfile::tempdir().unwrap();
	let path = dir.path().join("probe");
	std::fs::write(&path, b"keep-me").unwrap();
	let outcome = ensure_bind_source(path.to_str().unwrap());
	assert_eq!(outcome, BindSource::Present);
	let meta = std::fs::symlink_metadata(&path).unwrap();
	assert!(meta.is_file(), "source must remain a regular file");
	assert_eq!(std::fs::read(&path).unwrap(), b"keep-me");
}

/// Existing directory at the source path: the helper's decision must be
/// `Present`, and the directory's mtime must be untouched (no second
/// `create_dir_all`, no chmod).
#[test]
fn ensure_bind_source_existing_directory_is_present() {
	let dir = tempfile::tempdir().unwrap();
	let path = dir.path().join("already-here");
	std::fs::create_dir(&path).unwrap();
	let original_meta = std::fs::symlink_metadata(&path).unwrap();
	let outcome = ensure_bind_source(path.to_str().unwrap());
	assert_eq!(outcome, BindSource::Present);
	let after_meta = std::fs::symlink_metadata(&path).unwrap();
	assert_eq!(
		original_meta.modified().unwrap(),
		after_meta.modified().unwrap(),
		"existing directory mtime must be untouched"
	);
}

/// Dangling symlink at the source path: `symlink_metadata` returns `Ok`
/// for it, so the helper's decision must be `Present`, the symlink itself
/// must stay, and the target must NOT be created.
#[cfg(unix)]
#[test]
fn ensure_bind_source_dangling_symlink_is_present() {
	use std::os::unix::fs::symlink;
	let dir = tempfile::tempdir().unwrap();
	let link = dir.path().join("link");
	let target = dir.path().join("never");
	symlink(&target, &link).unwrap();
	let outcome = ensure_bind_source(link.to_str().unwrap());
	assert_eq!(outcome, BindSource::Present);
	let meta = std::fs::symlink_metadata(&link).unwrap();
	assert!(meta.file_type().is_symlink(), "link must remain a symlink");
	assert_eq!(std::fs::read_link(&link).unwrap(), target);
	assert!(
		!target.exists(),
		"symlink target must not be created by ensure_bind_source"
	);
}

/// Missing path under an existing directory: the helper's decision must be
/// `Created`, and the path must now be a directory.
#[test]
fn ensure_bind_source_missing_path_is_created() {
	let dir = tempfile::tempdir().unwrap();
	let path = dir.path().join("fresh");
	assert!(!path.exists());
	let outcome = ensure_bind_source(path.to_str().unwrap());
	assert_eq!(outcome, BindSource::Created);
	assert!(
		path.is_dir(),
		"missing path should be created as a directory"
	);
}

/// Missing path whose immediate parent is a regular FILE: `create_dir_all`
/// must fail because the parent is not a directory. The helper's decision
/// must be `Failed(msg)`, the message must mention the path, and nothing
/// must have been created where the helper could not.
#[cfg(unix)]
#[test]
fn ensure_bind_source_failure_under_file_is_failed() {
	let dir = tempfile::tempdir().unwrap();
	let blocker = dir.path().join("blocker");
	std::fs::write(&blocker, b"i-am-a-file").unwrap();
	let target = blocker.join("nested");
	let target_str = target.to_str().unwrap().to_string();
	assert!(!target.exists());
	let outcome = ensure_bind_source(&target_str);
	match outcome {
		BindSource::Failed(msg) => {
			assert!(
				msg.contains(&target_str),
				"warning text should mention the failing path, got: {msg}"
			);
		}
		other => panic!("expected Failed, got {other:?}"),
	}
	assert!(!target.exists(), "nothing must be created on failure");
	assert_eq!(
		std::fs::read(&blocker).unwrap(),
		b"i-am-a-file",
		"the blocking file must remain unchanged"
	);
}
