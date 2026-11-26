use super::*;

#[test]
fn retirement_and_uninstall_check_result_after_a_successful_stop() {
	let cases: [(Option<&'static [u8]>, i32, bool); 22] = [
		(Some(b"ActiveState=failed\nResult=exit-code\n"), 0, false),
		(Some(b"ActiveState=failed\nResult=timeout\n"), 0, false),
		(Some(b"ActiveState=inactive\nResult=exit-code\n"), 0, false),
		(Some(b"ActiveState=inactive\nResult=timeout\n"), 0, false),
		(Some(b"ActiveState=active\nResult=success\n"), 0, false),
		(
			Some(b"ActiveState=deactivating\nResult=success\n"),
			0,
			false,
		),
		(Some(b"ActiveState=activating\nResult=success\n"), 0, false),
		(Some(b"ActiveState=reloading\nResult=success\n"), 0, false),
		(Some(b"ActiveState=maintenance\nResult=success\n"), 0, false),
		(Some(b"ActiveState=refreshing\nResult=success\n"), 0, false),
		(Some(b"ActiveState=inactive\nResult=success\n"), 1, false),
		(Some(b""), 0, false),
		(Some(b"Result=success\n"), 0, false),
		(Some(b"ActiveState=inactive\n"), 0, false),
		(Some(b"ActiveState=unknown\nResult=success\n"), 0, false),
		(Some(b"ActiveState=inactive\nResult=\n"), 0, false),
		(
			Some(b"ActiveState=inactive\nResult=success extra\n"),
			0,
			false,
		),
		(
			Some(b"ActiveState=inactive\nResult=success\nunknown\n"),
			0,
			false,
		),
		(Some(b"\xff"), 0, false),
		(None, 0, false),
		(Some(b"ActiveState=inactive\nResult=success\n"), 0, true),
		(Some(b"Result=success\nActiveState=inactive\n"), 0, true),
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
					// Only the query after stop returns the response under test.
					let stopped = Cell::new(false);
					let mut sc = ScriptedCtl::new(move |args| {
						if args.first() == Some(&"stop") {
							stopped.set(true);
						}
						if args.first() == Some(&"cleanup") {
							42
						} else if stopped.get()
							&& args.starts_with(&["show", "--property=ActiveState,Result"])
						{
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
						calls[stop - 1],
						["show", "--property=ActiveState,Result", service]
					);
					assert_eq!(
						calls[stop + 1],
						["show", "--property=ActiveState,Result", service]
					);
					let cleanup_expected = matches!(
						result,
						Some(b"ActiveState=failed\nResult=exit-code\n")
							| Some(b"ActiveState=failed\nResult=timeout\n")
					);
					let cleanups: Vec<_> = calls.iter().filter(|c| c[0] == "cleanup").collect();
					assert_eq!(cleanups.len(), usize::from(cleanup_expected));
					if cleanup_expected {
						assert_eq!(calls[stop + 2], *cleanups[0]);
						assert!(cleanups[0][1].contains("--filter label=podup.project=proj"));
					}
					assert!(!calls.iter().any(|c| c[0] == "reset-failed"));
					let protection =
						root.join(format!("systemd/user/{service}.d/50-podup-ownership.conf"));
					if success {
						assert!(!unit.exists() && !protection.exists());
					} else {
						let error = outcome.unwrap_err().to_string();
						let expected = if cleanup_expected {
							"scripted cleanup failure"
						} else {
							"Result"
						};
						assert!(
							error.contains(service) && error.contains(expected),
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
