// --- build-context ignore loading -------------------------------------------

#[test]
fn local_build_context_patterns_loads_dockerignore() {
	// End-to-end: the build context's `.dockerignore` must be loaded by
	// `local_build_context_patterns` and made available to the
	// per-event pipeline. A `.log` file under the rule's path is then
	// ignored by the build-context pattern.
	let tmp = tempfile::tempdir().unwrap();
	let build_dir = tmp.path();
	std::fs::write(build_dir.join(".dockerignore"), b"*.log\n").unwrap();
	let service_yaml = format!("services:\n  web:\n    build: {}\n", build_dir.display());
	let file = crate::compose::parse_str(&service_yaml).unwrap();
	let service = file.services.get("web").unwrap();
	let (ctx_abs, ctx_patterns) =
		crate::engine::watch::ignore_filter::local_build_context_patterns(tmp.path(), service);
	assert!(
		ctx_abs.is_some(),
		"local build context must be resolved for a service with build: <local path>"
	);
	assert_eq!(
		ctx_patterns,
		vec!["*.log".to_string()],
		"patterns loaded from the build context's `.dockerignore`"
	);
}

#[test]
fn local_build_context_patterns_omits_for_image_only_service() {
	// A service with no `build:` block (image-based) must not pretend
	// to have a build context or load any `.dockerignore`.
	let yaml = "services:\n  web:\n    image: alpine:latest\n";
	let file = crate::compose::parse_str(yaml).unwrap();
	let service = file.services.get("web").unwrap();
	let (ctx_abs, ctx_patterns) = crate::engine::watch::ignore_filter::local_build_context_patterns(
		std::path::Path::new("/tmp"),
		service,
	);
	assert!(ctx_abs.is_none(), "image-only service has no build context");
	assert!(ctx_patterns.is_empty());
}
