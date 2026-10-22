use super::*;

/// `bind_mounts` reports only the bind-shaped entries (short and long
/// forms) with the parsed `read_only` flag. The named volume is excluded
/// because the feedback check only cares about bind-shaped mounts.
#[test]
fn bind_mounts_filters_binds_and_read_only_flag() {
	let svc = svc_with_volumes(vec![
		VolumeMount::Short("./src:/app:ro".into()),
		VolumeMount::Short("./a:/a".into()),
		VolumeMount::Short("named:/n".into()),
		VolumeMount::Short("/abs:/b:rw".into()),
	]);
	let mounts = bind_mounts(&svc);
	assert_eq!(mounts.len(), 3);
	assert_eq!(mounts[0].source, "./src");
	assert_eq!(mounts[0].target, "/app");
	assert!(
		mounts[0].read_only,
		"the `:ro` short form must report read_only = true"
	);
	assert_eq!(mounts[1].source, "./a");
	assert_eq!(mounts[1].target, "/a");
	assert!(!mounts[1].read_only);
	assert_eq!(mounts[2].source, "/abs");
	assert_eq!(mounts[2].target, "/b");
	assert!(
		!mounts[2].read_only,
		"the `:rw` short form must report read_only = false"
	);
}

/// The Windows-source short form carries its read-only flag as a third
/// field separated by colons, but a `splitn(3, ':')` would split on the
/// drive-letter colon and treat `/app:ro` as the options. The parser
/// used here (`parse_volume_string`) already knows about Windows drive
/// prefixes, so `ro` lands in the parsed options and the read-only flag
/// is reported correctly.
#[test]
fn bind_mounts_windows_short_form_read_only() {
	let svc = svc_with_volumes(vec![VolumeMount::Short("C:\\src:/app:ro".into())]);
	let mounts = bind_mounts(&svc);
	assert_eq!(mounts.len(), 1);
	assert_eq!(mounts[0].target, "/app");
	assert!(
		mounts[0].read_only,
		"`C:\\src:/app:ro` must report read_only = true"
	);

	let svc = svc_with_volumes(vec![VolumeMount::Short("C:\\src:/app".into())]);
	let mounts = bind_mounts(&svc);
	assert_eq!(mounts.len(), 1);
	assert_eq!(mounts[0].target, "/app");
	assert!(
		!mounts[0].read_only,
		"`C:\\src:/app` with no options must report read_only = false"
	);
}

/// The full container-side targets of a service's mounts in declaration
/// order: bind/named volumes first (the destination of the parsed
/// short-form or `target` of the long form), then every tmpfs entry
/// split at its first colon. The same parser that produces
/// `BindMountRef::target` for binds produces each entry here.
#[test]
fn mount_targets_returns_volumes_then_tmpfs() {
	let svc = Service {
		volumes: vec![
			VolumeMount::Short("./a:/a".into()),
			VolumeMount::Short("named:/n".into()),
			VolumeMount::Short("D:\\c:/app/cache".into()),
		],
		tmpfs: crate::compose::types::StringOrList::List(vec!["/t:size=1m".into()]),
		..Default::default()
	};
	let targets = mount_targets(&svc);
	assert_eq!(targets, vec!["/a", "/n", "/app/cache", "/t"]);
}
