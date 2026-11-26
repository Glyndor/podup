use super::*;

#[test]
fn retirement_and_uninstall_check_result_after_a_successful_stop() {
	let cases: [(Option<&'static [u8]>, i32, bool); 8] = [
		(Some(b"exit-code\n"), 0, false),
		(Some(b"timeout\n"), 0, false),
		(Some(b"success\n"), 1, false),
		(Some(b""), 0, false),
		(Some(b"success\nunknown\n"), 0, false),
		(Some(b"\xff"), 0, false),
		(None, 0, false),
		(Some(b"success\n"), 0, true),
	];
	for uninstall in [false, true] {
		for (filename, body, service) in [
			(
				"proj-old.container",
				"[Container]\nContainerName=custom-old\n",
				"proj-old.service",
			),
			(
				"proj.pod",
				"[Pod]\nPodName=custom-pod\n",
				"proj-pod.service",
			),
		] {
			for (result, code, success) in cases {
				with_env(|root| {
					let dir = root.join("containers/systemd");
					std::fs::create_dir_all(&dir).unwrap();
					let unit = dir.join(filename);
					let body = format!("# podup-owner: proj\n{body}");
					std::fs::write(&unit, &body).unwrap();
					// No start call: also covers a never-started unit with Result=success.
					let mut sc = ScriptedCtl::new(move |args| {
						if args.first() == Some(&"show") {
							code
						} else {
							0
						}
					});
					sc.result = result;
					let outcome = if uninstall {
						uninstall_quadlet(&sc, "proj")
					} else {
						install_quadlet(
							&sc,
							&parse_str(IMG).unwrap(),
							"proj",
							Path::new(BASE),
							true,
							false,
						)
					};
					assert_eq!(outcome.is_ok(), success, "{outcome:?}");
					let calls = sc.log();
					let stop = calls.iter().position(|c| c == &["stop", service]).unwrap();
					assert_eq!(
						calls[stop + 1],
						["show", "--property=Result", "--value", service]
					);
					let protection =
						root.join(format!("systemd/user/{service}.d/50-podup-ownership.conf"));
					if success {
						assert!(!unit.exists() && !protection.exists());
					} else {
						let error = outcome.unwrap_err().to_string();
						assert!(
							error.contains(service) && error.contains("Result"),
							"{error}"
						);
						assert_eq!(std::fs::read_to_string(&unit).unwrap(), body);
						assert!(protection.is_file());
					}
				});
			}
		}
	}
}
