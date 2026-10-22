use super::*;

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
