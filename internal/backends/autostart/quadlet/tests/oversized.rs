use super::*;
use std::io::Write;

fn oversized_unit(root: &Path, filename: &str, header: &str) -> std::path::PathBuf {
	let dir = root.join("containers/systemd");
	std::fs::create_dir_all(&dir).unwrap();
	let unit = dir.join(filename);
	let mut file = std::fs::File::create(&unit).unwrap();
	file.write_all(header.as_bytes()).unwrap();
	file.write_all(&b"# padding\n".repeat(crate::filesystem::MAX_FILE_BYTES as usize / 10 + 1))
		.unwrap();
	unit
}

#[test]
fn oversized_units_without_a_complete_usable_identity_fail_before_stopping() {
	for uninstall in [false, true] {
		for (filename, key) in [
			("proj-old.container", "ContainerName"),
			("proj.pod", "PodName"),
		] {
			for identity in [
				String::new(),
				format!("{key}=\n"),
				format!("{key}=invalid name\n"),
				{
					// The name fits in the read, but its newline lies outside it.
					let line = format!("{key}=custom\n");
					let prefix = "# podup-owner: proj\n#";
					format!("#{}\n{line}", "x".repeat(4096 - prefix.len() - line.len()))
				},
			] {
				with_env(|root| {
					let unit =
						oversized_unit(root, filename, &format!("# podup-owner: proj\n{identity}"));
					let sc = FakeCtl::new();
					let result = if uninstall {
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
					let error = result.unwrap_err().to_string();
					assert!(
						error.contains(key) && error.contains("cannot protect"),
						"{error}"
					);
					assert!(
						sc.log().is_empty(),
						"no reload or stop before identity is established"
					);
					assert!(unit.is_file());
					assert!(!root.join("systemd/user").exists());
				});
			}
		}
	}
}

#[test]
fn oversized_units_get_real_identity_protection_loaded_before_uninstall_stops() {
	for (filename, service, runtime_name, key) in [
		(
			"proj-web.container",
			"proj-web.service",
			"custom-web",
			"ContainerName",
		),
		("proj.pod", "proj-pod.service", "custom-pod", "PodName"),
	] {
		super::with_env(|root| {
			let section = if key == "ContainerName" {
				"Container"
			} else {
				"Pod"
			};
			let unit = oversized_unit(
				root,
				filename,
				&format!("# podup-owner: proj\n[{section}]\n{key}={runtime_name}\n"),
			);
			let path = root.join(format!("systemd/user/{service}.d/50-podup-ownership.conf"));
			let loaded = std::cell::Cell::new(false);
			let sc = ScriptedCtl::new(move |args| {
				if args == ["daemon-reload"] && !loaded.replace(true) {
					let text = std::fs::read_to_string(&path).unwrap();
					assert!(text.contains("ExecStop=\n"), "{text}");
					assert!(text.contains("ExecStopPost=\n"), "{text}");
					assert!(
						text.contains(&format!("--filter name=^{runtime_name}$$")),
						"{text}"
					);
					assert!(text.contains("--filter label=podup.project=proj"), "{text}");
					let inferred = filename
						.strip_suffix(".container")
						.or_else(|| filename.strip_suffix(".pod"))
						.unwrap();
					assert!(!text.contains(&format!("name=^{inferred}$$")), "{text}");
				}
				if args.first() == Some(&"stop") {
					assert!(loaded.get(), "stop before protective reload: {args:?}");
				}
				0
			});
			uninstall_quadlet(&sc, "proj").unwrap();
			assert!(!unit.exists());
			assert_eq!(sc.log()[0], ["daemon-reload"]);
			assert_eq!(sc.log()[1], ["stop", service]);
		});
	}
}
