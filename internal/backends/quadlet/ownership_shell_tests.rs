//! Execute the generated shells with an isolated Podman fake, without systemd or storage.

use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Output};

use super::{installed, lines, pod_drop_in, POD};

fn command(key: &str) -> String {
	if key == "ExecStartPre=" {
		return lines(&installed(POD, "proj"), "proj.pod", key)[0].to_string();
	}
	pod_drop_in("proj", "[Pod]\nPodName=proj\n")
		.lines()
		.rfind(|line| line.starts_with(key) && line.contains("/bin/sh"))
		.unwrap()
		.to_string()
}

fn run(
	key: &str,
	ps: &str,
	ids: &str,
	members: &str,
	member_status: &str,
	action: &str,
) -> (Output, String) {
	let dir = tempfile::tempdir().unwrap();
	let podman = dir.path().join("podman");
	let log = dir.path().join("actions");
	std::fs::write(
		&podman,
		r#"#!/bin/sh
case "$1 $2" in
  'pod ps') printf '%s' "$POD_IDS"; exit "$PS_STATUS" ;;
  'ps -aq')
    [ "$*" = 'ps -aq --filter pod=own-pod --filter label!=podup.project=proj' ] || exit 99
    printf '%s' "$MEMBERS"; exit "$MEMBER_STATUS" ;;
  'pod stop'|'pod rm') printf '%s\n' "$*" >> "$LOG"; exit "$ACTION_STATUS" ;;
  *) exit 98 ;;
esac
"#,
	)
	.unwrap();
	std::fs::set_permissions(&podman, std::fs::Permissions::from_mode(0o700)).unwrap();
	let line = command(key);
	assert!(
		!line.starts_with(&format!("{key}-")),
		"failure must be reported: {line}"
	);
	let shell = line
		.split_once("/bin/sh -c '")
		.unwrap()
		.1
		.strip_suffix('\'')
		.unwrap();
	let shell = shell
		.replace("$$", "$")
		.replace("/usr/bin/podman", podman.to_str().unwrap());
	let output = Command::new("/bin/sh")
		.args(["-c", &shell])
		.env("POD_IDS", ids)
		.env("PS_STATUS", ps)
		.env("MEMBERS", members)
		.env("MEMBER_STATUS", member_status)
		.env("ACTION_STATUS", action)
		.env("LOG", &log)
		.output()
		.unwrap();
	(output, std::fs::read_to_string(log).unwrap_or_default())
}

#[test]
fn discovery_failure_is_reported_without_stopping_or_removing() {
	for key in ["ExecStartPre=", "ExecStop=", "ExecStopPost="] {
		let (out, log) = run(key, "42", "", "", "0", "0");
		assert_eq!(out.status.code(), Some(42), "{key}");
		assert!(log.is_empty(), "{key}: {log}");
	}
}

#[test]
fn no_match_succeeds_and_actions_preserve_their_exit_status() {
	for key in ["ExecStartPre=", "ExecStop=", "ExecStopPost="] {
		let (out, log) = run(key, "0", "", "", "0", "43");
		assert!(out.status.success(), "{key}");
		assert!(log.is_empty(), "{key}: {log}");
		for status in ["0", "43"] {
			let (out, log) = run(key, "0", "own-pod", "", "0", status);
			assert_eq!(
				out.status.code(),
				Some(status.parse::<i32>().unwrap()),
				"{key}"
			);
			assert!(log.contains("own-pod"), "{key}: {log}");
		}
	}
}

#[test]
fn any_unlabelled_member_blocks_prestart_and_poststop_removal() {
	// Includes stopped containers (the fake requires -a) and an unlabelled infra container.
	for key in ["ExecStartPre=", "ExecStopPost="] {
		for member in ["user-container", "stopped-container", "infra-container"] {
			let (out, log) = run(key, "0", "own-pod", member, "0", "0");
			assert_eq!(out.status.code(), Some(1), "{key}: {member}");
			assert!(String::from_utf8_lossy(&out.stderr).contains("refusing pod cleanup"));
			assert!(log.is_empty(), "{key}: {log}");
		}
		let (out, log) = run(key, "0", "own-pod", "", "44", "0");
		assert_eq!(out.status.code(), Some(44), "{key}");
		assert!(log.is_empty(), "{key}: {log}");
	}
}

#[test]
fn poststop_has_no_unchecked_id_file_removal() {
	let text = pod_drop_in("proj", "[Pod]\nPodName=proj\n");
	assert!(
		!text.contains("pod rm --ignore --force --pod-id-file"),
		"{text}"
	);
	let text = pod_drop_in("proj", "[Pod]\n");
	assert!(text.contains("ExecStopPost=/bin/false\n"), "{text}");
}
