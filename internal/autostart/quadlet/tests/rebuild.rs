use super::*;

#[test]
fn rebuild_is_scoped_by_ownership_label_and_ignores_sibling_build_units() {
	with_env(|_root| {
		// 'app-extra' shares the `app-` filename prefix with 'app', so a naive
		// prefix match would treat its `.build`/container as buildable under
		// 'app' too.
		install_quadlet(
			&FakeCtl::new(),
			&parse_str(BUILD).unwrap(),
			"app",
			Path::new(BASE),
			true,
			false,
		)
		.unwrap();
		install_quadlet(
			&FakeCtl::new(),
			&parse_str(BUILD).unwrap(),
			"app-extra",
			Path::new(BASE),
			true,
			false,
		)
		.unwrap();
		let sc = FakeCtl::new();
		rebuild_quadlet(&sc, "app", None).unwrap();
		assert_eq!(
			sc.log(),
			vec![
				vec!["restart".to_string(), "app-web-build.service".to_string()],
				vec!["restart".to_string(), "app-web.service".to_string()],
			]
		);
	});
}

#[test]
fn rebuild_restarts_build_then_container() {
	with_env(|_root| {
		install_quadlet(
			&FakeCtl::new(),
			&parse_str(BUILD).unwrap(),
			"proj",
			Path::new(BASE),
			true,
			false,
		)
		.unwrap();
		let sc = FakeCtl::new();
		rebuild_quadlet(&sc, "proj", Some("web")).unwrap();
		assert_eq!(
			sc.log(),
			vec![
				vec!["restart".to_string(), "proj-web-build.service".to_string()],
				vec!["restart".to_string(), "proj-web.service".to_string()],
			]
		);
	});
}

#[test]
fn rebuild_unknown_service_errors_and_lists_valid_ones() {
	with_env(|_root| {
		install_quadlet(
			&FakeCtl::new(),
			&parse_str(BUILD).unwrap(),
			"proj",
			Path::new(BASE),
			true,
			false,
		)
		.unwrap();
		let err = rebuild_quadlet(&FakeCtl::new(), "proj", Some("nope")).unwrap_err();
		let msg = format!("{err}");
		assert!(msg.contains("has no build unit"), "{msg}");
		assert!(msg.contains("web"), "{msg}");
	});
}
