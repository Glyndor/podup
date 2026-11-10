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

fn short_opts(spec: &str) -> Vec<String> {
	let svc = svc_with_volumes(vec![VolumeMount::Short(spec.into())]);
	let (mounts, named) = build_mounts_all(&svc, Path::new("/base"));
	match mounts.into_iter().next() {
		Some(m) => m.options,
		None => named.into_iter().next().unwrap().options,
	}
}

#[test]
fn short_form_consistency_hints_are_dropped() {
	// Docker accepts these on Linux and ignores them; Podman rejects them.
	for hint in ["cached", "delegated", "consistent"] {
		assert!(short_opts(&format!("./x:/y:{hint}")).is_empty(), "{hint}");
		assert_eq!(short_opts(&format!("./x:/y:ro,{hint}")), ["ro"], "{hint}");
		assert_eq!(short_opts(&format!("./x:/y:{hint},z")), ["z"], "{hint}");
		assert!(short_opts(&format!("vol:/y:{hint}")).is_empty(), "{hint}");
	}
}

#[test]
fn short_form_relabel_options_reach_the_engine() {
	assert_eq!(short_opts("./x:/y:ro,z"), ["ro", "z"]);
	assert_eq!(short_opts("./x:/y:Z"), ["Z"]);
	assert_eq!(short_opts("./x:/y:cached,Z"), ["Z"]);
}
