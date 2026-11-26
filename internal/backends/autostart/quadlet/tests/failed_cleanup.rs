use super::*;
use std::path::PathBuf;

fn fixture(root: &Path, pod: bool) -> (PathBuf, PathBuf, &'static str) {
	let (filename, body, service) = if pod {
		(
			"proj.pod",
			"[Pod]\nPodName=custom-pod\n",
			"proj-pod.service",
		)
	} else {
		(
			"proj-old.container",
			"[Container]\nContainerName=custom-old\n",
			"proj-old.service",
		)
	};
	let dir = root.join("containers/systemd");
	std::fs::create_dir_all(&dir).unwrap();
	let unit = dir.join(filename);
	std::fs::write(&unit, format!("# podup-owner: proj\n{body}")).unwrap();
	let protection = root.join(format!("systemd/user/{service}.d/50-podup-ownership.conf"));
	(unit, protection, service)
}

fn assert_retried(sc: &FakeCtl, service: &str) {
	let calls = sc.log();
	assert_eq!(calls[1], ["show", "--property=ActiveState,Result", service]);
	assert_eq!(calls[2], ["stop", service]);
	assert_eq!(calls[3][0], "cleanup");
	assert!(calls[3][1].contains("--filter label=podup.project=proj"));
	assert!(!calls[3][1].contains("--cidfile") && !calls[3][1].contains("--pod-id-file"));
	assert_eq!(calls[4], ["reset-failed", service]);
	assert!(!calls.iter().any(|c| c[0] == "start" || c[0] == "restart"));
}

#[test]
fn historical_failure_without_resources_can_be_uninstalled() {
	for pod in [false, true] {
		with_env(|root| {
			let (unit, protection, service) = fixture(root, pod);
			let sc = FakeCtl::new();
			sc.failed(service);
			assert!(!sc.resources.get());
			uninstall_quadlet(&sc, "proj").unwrap();
			assert!(!unit.exists() && !protection.exists());
			assert_retried(&sc, service);
			assert_eq!(sc.states.borrow()[service], ("inactive", "success"));
		});
	}
}

#[test]
fn historical_cleanup_failure_retains_units_and_retry_succeeds() {
	for pod in [false, true] {
		with_env(|root| {
			let (unit, protection, service) = fixture(root, pod);
			let original = std::fs::read(&unit).unwrap();
			let sc = FakeCtl::new();
			sc.failed(service);
			sc.resources.set(true);
			sc.cleanup_status.set(42);
			let error = uninstall_quadlet(&sc, "proj").unwrap_err().to_string();
			assert!(
				error.contains(service) && error.contains("remaining own-resource"),
				"{error}"
			);
			assert_eq!(std::fs::read(&unit).unwrap(), original);
			assert!(protection.is_file() && sc.resources.get());
			assert_eq!(sc.states.borrow()[service], ("failed", "exit-code"));
			assert!(!sc.log().iter().any(|c| c[0] == "reset-failed"));
			sc.calls.borrow_mut().clear();
			sc.cleanup_status.set(0);
			uninstall_quadlet(&sc, "proj").unwrap();
			assert_retried(&sc, service);
			assert!(!sc.resources.get() && !unit.exists() && !protection.exists());
		});
	}
}

#[test]
fn failure_from_current_stop_retains_units_and_protection() {
	for pod in [false, true] {
		with_env(|root| {
			let (unit, protection, service) = fixture(root, pod);
			let original = std::fs::read(&unit).unwrap();
			let sc = FakeCtl::new();
			sc.states
				.borrow_mut()
				.insert(service.to_string(), ("active", "success"));
			sc.stop_result.set("exit-code");
			sc.cleanup_status.set(42);
			sc.resources.set(true);
			let error = uninstall_quadlet(&sc, "proj").unwrap_err().to_string();
			assert!(
				error.contains(service) && error.contains("remaining own-resource"),
				"{error}"
			);
			assert_eq!(std::fs::read(&unit).unwrap(), original);
			assert!(protection.is_file() && sc.resources.get());
			assert_eq!(sc.states.borrow()[service], ("failed", "exit-code"));
			let calls = sc.log();
			assert_eq!(calls[3], ["show", "--property=ActiveState,Result", service]);
			assert_eq!(calls[4][0], "cleanup");
			assert!(calls[4][1].contains("--filter label=podup.project=proj"));
			assert!(!calls.iter().any(|c| c[0] == "reset-failed"));
		});
	}
}

#[test]
fn failure_from_current_stop_is_cleaned_reset_and_retired() {
	for uninstall in [false, true] {
		for pod in [false, true] {
			with_env(|root| {
				let (unit, protection, service) = fixture(root, pod);
				let sc = FakeCtl::new();
				sc.states
					.borrow_mut()
					.insert(service.to_string(), ("active", "success"));
				sc.stop_result.set("exit-code");
				sc.resources.set(true);
				if uninstall {
					uninstall_quadlet(&sc, "proj").unwrap();
				} else {
					install_quadlet(
						&sc,
						&parse_str(IMG).unwrap(),
						"proj",
						Path::new(BASE),
						true,
						false,
					)
					.unwrap();
				}
				assert!(!unit.exists() && !protection.exists() && !sc.resources.get());
				assert_eq!(sc.states.borrow()[service], ("inactive", "success"));
				let calls = sc.log();
				assert_eq!(calls[1], ["show", "--property=ActiveState,Result", service]);
				assert_eq!(calls[2], ["stop", service]);
				assert_eq!(calls[3], ["show", "--property=ActiveState,Result", service]);
				assert_eq!(calls[4][0], "cleanup");
				assert!(calls[4][1].contains("--filter label=podup.project=proj"));
				assert_eq!(calls[5], ["reset-failed", service]);
			});
		}
	}
}

#[test]
fn failed_stop_without_authorised_cleanup_retains_units_and_protection() {
	for pod in [false, true] {
		with_env(|root| {
			let (unit, protection, service) = fixture(root, pod);
			let original = std::fs::read(&unit).unwrap();
			let drop_ins = [super::super::drop_ins::DropIn {
				service: service.to_string(),
				contents: "# podup-owner: proj\n[Service]\nExecStopPost=\n".to_string(),
			}];
			super::super::drop_ins::write(&drop_ins).unwrap();
			let sc = FakeCtl::new();
			sc.states
				.borrow_mut()
				.insert(service.to_string(), ("active", "success"));
			sc.stop_result.set("exit-code");
			let error = super::super::stop::service(&sc, service, &drop_ins)
				.unwrap_err()
				.to_string();
			assert!(
				error.contains("no authorised project-label cleanup"),
				"{error}"
			);
			assert_eq!(std::fs::read(&unit).unwrap(), original);
			assert_eq!(
				std::fs::read_to_string(&protection).unwrap(),
				drop_ins[0].contents
			);
			assert!(!sc
				.log()
				.iter()
				.any(|c| c[0] == "cleanup" || c[0] == "reset-failed"));
		});
	}
}

#[test]
fn install_retires_an_obsolete_failed_unit() {
	for pod in [false, true] {
		with_env(|root| {
			let (unit, protection, service) = fixture(root, pod);
			let sc = FakeCtl::new();
			sc.failed(service);
			sc.resources.set(true);
			install_quadlet(
				&sc,
				&parse_str(IMG).unwrap(),
				"proj",
				Path::new(BASE),
				true,
				false,
			)
			.unwrap();
			assert_retried(&sc, service);
			assert!(!unit.exists() && !protection.exists() && !sc.resources.get());
			assert!(root.join("containers/systemd/proj-web.container").is_file());
		});
	}
}

#[test]
fn query_and_reset_failures_retain_unit_and_protection() {
	for verb in ["show", "reset-failed"] {
		with_env(|root| {
			let (unit, protection, _) = fixture(root, false);
			struct FailureCtl<'a>(&'a str, FakeCtl);
			impl SystemCtl for FailureCtl<'_> {
				fn systemctl(&self, args: &[&str]) -> std::io::Result<Output> {
					if args[0] == self.0 {
						return Err(std::io::Error::other("query/reset unavailable"));
					}
					self.1.systemctl(args)
				}
				fn loginctl(&self, args: &[&str]) -> std::io::Result<Output> {
					self.1.loginctl(args)
				}
				fn cleanup(&self, command: &str) -> std::io::Result<Output> {
					self.1.cleanup(command)
				}
			}
			let sc = FailureCtl(verb, FakeCtl::new());
			sc.1.failed("proj-old.service");
			assert!(uninstall_quadlet(&sc, "proj").is_err());
			assert!(unit.is_file() && protection.is_file());
			if verb == "show" {
				assert!(!sc
					.1
					.log()
					.iter()
					.any(|c| c[0] == "stop" || c[0] == "cleanup"));
			}
		});
	}
}

#[test]
fn retry_commands_keep_label_filters_and_refuse_foreign_pod_members() {
	use std::os::unix::fs::PermissionsExt;
	use std::process::Command;
	for pod in [false, true] {
		with_env(|root| {
			let (unit, _, _) = fixture(root, pod);
			let protection = super::super::drop_ins::existing("proj", &[unit]).unwrap();
			let command = protection[0].cleanup_command().unwrap();
			let podman = root.join("podman");
			let log = root.join("actions");
			std::fs::write(&podman, r#"#!/bin/sh
case "$1 $2" in
  'rm -v')
    [ "$*" = 'rm -v -f -i --filter name=^custom-old$ --filter label=podup.project=proj' ] || exit 99
    [ "$OWNED" = yes ] || exit 0 ;;
  'pod ps')
    [ "$*" = 'pod ps -q --no-trunc --filter name=^custom-pod$ --filter label=podup.project=proj' ] || exit 99
    [ "$QUERY_STATUS" = 0 ] || exit "$QUERY_STATUS"
    [ "$OWNED" = yes ] && printf '%s' own-pod
    exit 0 ;;
  'ps -aq')
    [ "$*" = 'ps -aq --filter pod=own-pod --filter label!=podup.project=proj' ] || exit 99
    printf '%s' "$FOREIGN"
    exit 0 ;;
  'pod rm') [ "$*" = 'pod rm --force own-pod' ] || exit 99 ;;
  *) exit 98 ;;
esac
printf '%s\n' "$*" >> "$LOG"
exit "$ACTION_STATUS"
"#).unwrap();
			std::fs::set_permissions(&podman, std::fs::Permissions::from_mode(0o700)).unwrap();
			let command = command.replace("/usr/bin/podman", podman.to_str().unwrap());
			for (owned, foreign, query, action, expected) in [
				("no", "", "0", "43", 0),
				("yes", "", "0", "0", 0),
				("yes", "", "0", "43", 43),
				("yes", "foreign-member", "0", "0", if pod { 1 } else { 0 }),
				("yes", "", "42", "0", if pod { 42 } else { 0 }),
			] {
				let _ = std::fs::remove_file(&log);
				let out = Command::new("/bin/sh")
					.args(["-c", &command])
					.env("OWNED", owned)
					.env("FOREIGN", foreign)
					.env("QUERY_STATUS", query)
					.env("ACTION_STATUS", action)
					.env("LOG", &log)
					.output()
					.unwrap();
				assert_eq!(out.status.code(), Some(expected), "{command}: {out:?}");
				if owned == "no" || (pod && (!foreign.is_empty() || query != "0")) {
					assert!(!log.exists(), "unauthorised removal: {command}");
				}
				if pod && !foreign.is_empty() {
					let error = String::from_utf8_lossy(&out.stderr);
					assert!(
						error.contains("own-pod") && error.contains(foreign),
						"{error}"
					);
				}
			}
		});
	}
}
