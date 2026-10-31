use super::*;

#[test]
fn restart_service_set_empty_targets_is_all() {
	let file = file_with(&["a", "b"]);
	let (full, targets) = restart_service_set(&file, &[], false);
	assert_eq!(full, targets);
	assert_eq!(full.len(), 2);
	assert!(full.contains("a") && full.contains("b"));
}

#[test]
fn restart_service_set_includes_cascade_dependents() {
	// web depends_on db with restart: true → restarting db cascades to web.
	let file = crate::parse_str(
		"services:\n  db:\n    image: x\n  web:\n    image: x\n    depends_on:\n      db:\n        condition: service_started\n        restart: true\n",
	)
	.unwrap();
	let (full, targets) = restart_service_set(&file, &["db".into()], false);
	assert!(targets.contains("db") && targets.len() == 1);
	assert!(full.contains("db") && full.contains("web"));
}

#[test]
fn restart_service_set_no_deps_excludes_cascade() {
	let file = crate::parse_str(
		"services:\n  db:\n    image: x\n  web:\n    image: x\n    depends_on:\n      db:\n        condition: service_started\n        restart: true\n",
	)
	.unwrap();
	let (full, _) = restart_service_set(&file, &["db".into()], true);
	assert!(full.contains("db"));
	assert!(!full.contains("web"));
}

/// Implicit dependencies participate in the cascade-restart set. A service
/// that joins another via `network_mode: service:zdb` cascades on a
/// `zdb` restart (matches `links` semantics); a service that only borrows
/// `volumes_from: [zdb]` does not.
#[test]
fn restart_service_set_includes_implicit_link_but_not_implicit_volumes_from() {
	let file = crate::parse_str(
		"services:\n  zdb:\n    image: x\n  web:\n    image: x\n    network_mode: \"service:zdb\"\n  app2:\n    image: x\n    volumes_from: [\"zdb\"]\n",
	)
	.unwrap();
	let (full, _) = restart_service_set(&file, &["zdb".into()], false);
	assert!(
		full.contains("web"),
		"network_mode service:zdb cascades restarts; got: {full:?}"
	);
	assert!(
		!full.contains("app2"),
		"volumes_from alone does not cascade restarts; got: {full:?}"
	);
}
