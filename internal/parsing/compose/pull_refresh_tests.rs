//! Raw clear directives must survive both inheritance paths.

fn cleared(directive: &str) {
	let dir = tempfile::tempdir().unwrap();
	let base = dir.path().join("base.yaml");
	let over = dir.path().join("over.yaml");
	std::fs::write(
		&base,
		"services:\n  web: {image: shared, pull_refresh_after: 24h}\n",
	)
	.unwrap();
	std::fs::write(
		&over,
		format!("services:\n  web: {{pull_refresh_after: {directive}}}\n"),
	)
	.unwrap();
	let file = super::parse_files_with_env_files(&[base.clone(), over.clone()], &[]).unwrap();
	assert_eq!(file.services["web"].pull_refresh_after, None);
	let same = format!("services:\n  base: {{image: shared, pull_refresh_after: 24h}}\n  web: {{extends: base, pull_refresh_after: {directive}}}\n");
	assert_eq!(
		super::parse_str(&same).unwrap().services["web"].pull_refresh_after,
		None
	);
	std::fs::write(&over, format!("services:\n  web: {{extends: {{file: base.yaml, service: web}}, pull_refresh_after: {directive}}}\n")).unwrap();
	assert_eq!(
		super::parse_file_with_env_files(&over, &[])
			.unwrap()
			.services["web"]
			.pull_refresh_after,
		None
	);
}

#[test]
fn reset_clears_refresh_interval() {
	cleared("!reset 1h");
}

#[test]
fn override_null_clears_refresh_interval() {
	cleared("!override null");
}

#[test]
fn plain_null_clears_refresh_interval() {
	cleared("null");
}

#[test]
fn override_value_replaces_refresh_interval() {
	let file = super::parse_str("services:\n  base: {image: shared, pull_refresh_after: 24h}\n  web: {extends: base, pull_refresh_after: !override 1h}\n").unwrap();
	assert_eq!(
		file.services["web"].pull_refresh_after.as_deref(),
		Some("1h")
	);
}

#[test]
fn omitted_refresh_preserves_serialization_and_hashes() {
	let file =
		super::parse_str("services:\n  web: {image: shared, pull_policy: missing}\n").unwrap();
	let service = &file.services["web"];
	let json = serde_json::to_string(service).unwrap();
	assert!(!json.contains("pull_refresh_after"));
	let before = r#"{"image":"shared","environment":null,"env_file":null,"tmpfs":null,"networks":null,"dns":null,"dns_search":null,"dns_opt":null,"depends_on":null,"labels":null,"annotations":null,"sysctls":null,"label_file":null,"pull_policy":"missing"}"#;
	assert_eq!(json, before);
	use sha2::{Digest, Sha256};
	assert_eq!(
		Sha256::digest(json.as_bytes())
			.iter()
			.map(|b| format!("{b:02x}"))
			.collect::<String>(),
		"009d2c010816915935af2cb1e2533597d2caff9262da99a74bc827f49e529b3f"
	);
	let env = crate::engine::container::effective_env(service, std::path::Path::new(".")).unwrap();
	assert_eq!(
		crate::engine::container::config_hash(
			service,
			&file,
			"p",
			std::path::Path::new("."),
			&Default::default(),
			&env
		)
		.unwrap(),
		"d25f1c34ed0e37da52f270fdd642cd25701cfbdb98e5f789d5c14e121b9dc792"
	);
}
