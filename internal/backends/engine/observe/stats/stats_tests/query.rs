use super::*;

// --- the query and the services it names ---------------------------------

#[test]
fn containers_query_repeats_param_per_container() {
	// libpod wants `containers` repeated, not comma-joined: a comma-joined
	// list is read as a single container name and 404s.
	let mut wanted = HashSet::new();
	wanted.insert("proj-web-1".to_string());
	wanted.insert("proj-db-1".to_string());
	assert_eq!(
		containers_query(&wanted),
		"&containers=proj-db-1&containers=proj-web-1"
	);
}

#[test]
fn containers_query_empty_when_none_wanted() {
	assert_eq!(containers_query(&HashSet::new()), "");
}

#[test]
fn first_unknown_service_flags_typos() {
	let file =
		crate::parse_str("services:\n  web:\n    image: nginx\n  db:\n    image: postgres\n")
			.unwrap();
	assert_eq!(first_unknown_service(&file, &[]), None);
	assert_eq!(
		first_unknown_service(&file, &["web".into(), "db".into()]),
		None
	);
	assert_eq!(
		first_unknown_service(&file, &["web".into(), "bogus".into()]),
		Some("bogus")
	);
}
