use super::*;

#[test]
fn dep_closure_includes_transitive_dependencies() {
	let file = crate::parse_str(
		"services:\n  web:\n    image: a\n    depends_on:\n      - api\n  api:\n    image: b\n    depends_on:\n      - db\n  db:\n    image: c\n  lone:\n    image: d\n",
	)
	.unwrap();
	let mut got: Vec<String> = pull_dep_closure(&file, &["web".to_string()])
		.into_iter()
		.collect();
	got.sort();
	assert_eq!(got, vec!["api", "db", "web"]);
}

#[test]
fn dep_closure_of_leaf_is_just_itself() {
	let file = crate::parse_str("services:\n  db:\n    image: c\n").unwrap();
	let got: Vec<String> = pull_dep_closure(&file, &["db".to_string()])
		.into_iter()
		.collect();
	assert_eq!(got, vec!["db"]);
}

#[tokio::test]
async fn pull_unknown_service_is_rejected() {
	// `pull bogus` must error on the unknown name instead of silently exiting 0.
	let file = crate::parse_str("services:\n  web:\n    image: a\n").unwrap();
	let e = crate::engine::Engine::new(
		crate::libpod::Client::new("/nonexistent.sock"),
		"proj".into(),
	);
	let err = e
		.pull_services(&file, &["nope".to_string()])
		.await
		.expect_err("unknown service must be rejected");
	assert!(
		matches!(err, crate::error::ComposeError::ServiceNotFound(_)),
		"unexpected error: {err:?}"
	);
}

// `pull_rejects_an_unknown_pull_policy_without_panicking` lives in the
// sibling `pull_typo_tests.rs` to keep this file below the source-line
// limit (#1450).
