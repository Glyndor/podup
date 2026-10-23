use super::*;

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
