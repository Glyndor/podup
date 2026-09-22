//! I assert the user-namespace options reach the pod path, not just the
//! container path. The container-level test in
//! `internal/engine/container/userns_tests.rs` builds a container through
//! `create_and_start` directly and never exercises the pod, so every
//! assertion in that file passes whatever the pod path does. A regression
//! that stopped forwarding the ID-mapping options into the pod spec
//! would fail nothing there.
//!
//! This file puts a single alpine service inside a real pod and asserts
//! the same three modes the unit file covers. The size field of
//! `/proc/self/uid_map` is what separates a working option path from
//! one that silently drops the option: a path that ignored the option
//! and fell back to the default would produce the `auto` answer
//! (`0 1 1024`) too, because the default IS `0 1 1024`. The size field
//! must therefore be read, not just the presence of a map. For the
//! `auto` family the value is summed across every line, because podman
//! may split a fragmented allocation across more than one range; for
//! `keep-id` it is the first line's size because the other lines are
//! fixed plumbing. See the assertion block for the split the sum
//! protects against and how each family is checked.
//!
//! The test also asserts the container is a member of the project's pod
//! by comparing the `Pod` field on container inspect against the
//! project's pod ID. Without that, a regression that quietly created the
//! container outside the pod would leave every mapping assertion
//! passing while the pod path went unexercised.
//!
//! Defaults are the runtime's, read from Podman 5.7.0 on 2026-09-20:
//! `auto` → size 1024; `auto:size=N` → size N total across every range;
//! `keep-id:uid=N,gid=M` → size N on the first line of the mapping.

use std::fs;
use std::process::Command;

use super::*;

struct Case {
	label: &'static str,
	mode: &'static str,
	expect_size: u32,
	expect_uid: &'static str,
}

const CASES: &[Case] = &[
	Case {
		label: "userns-pod-auto",
		mode: "auto",
		expect_size: 1024,
		expect_uid: "0",
	},
	Case {
		label: "userns-pod-auto-size-2048",
		mode: "auto:size=2048",
		expect_size: 2048,
		expect_uid: "0",
	},
	Case {
		label: "userns-pod-keep-id-321-654",
		mode: "keep-id:uid=321,gid=654",
		expect_size: 321,
		expect_uid: "321",
	},
];

fn write_compose(dir: &std::path::Path, mode: &str) -> String {
	let yaml = format!(
		"x-podman-pod: true\nservices:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    userns_mode: {mode:?}\n"
	);
	let path = dir.join("docker-compose.yml");
	fs::write(&path, yaml).unwrap();
	path.to_str().unwrap().to_string()
}

// Resolve the Podman socket the way `podup::podman::connect_from_env` does,
// then turn it into `unix://...` for the `podman` CLI subprocess. Without this,
// a dev env where `CONTAINER_HOST` is unset but `PODMAN_SOCKET` points at the
// rootless socket will see `podman exec` fail with "no such container" while
// `podup up` against the same socket succeeds. The CI lane is unaffected
// because it activates the socket through systemd and `podman` CLI picks it
// up; this helper just makes the same shape work without that activation.
fn container_host_for_cli() -> Option<String> {
	let raw = std::env::var("PODMAN_SOCKET")
		.ok()
		.or_else(|| std::env::var("DOCKER_HOST").ok())?;
	let path = raw
		.strip_prefix("unix://")
		.or_else(|| raw.strip_prefix("npipe://"))
		.unwrap_or(&raw);
	Some(format!("unix://{path}"))
}

fn podman_cmd() -> Command {
	let mut cmd = Command::new("podman");
	if let Ok(host) = std::env::var("CONTAINER_HOST") {
		cmd.env("CONTAINER_HOST", host);
	} else if let Some(host) = container_host_for_cli() {
		cmd.env("CONTAINER_HOST", host);
	}
	cmd
}

struct DownGuard {
	compose: String,
	proj: String,
}

impl Drop for DownGuard {
	fn drop(&mut self) {
		// Teardown must run even when an assertion panics, otherwise a leaked
		// pod holds its subordinate ID slice and the next test exhausts the
		// pool with `not enough unused IDs in user namespace`, which reads
		// like a podup defect and is not one.
		let mut cmd = podman_cmd();
		cmd.args(["pod", "rm", "-f", &self.proj]);
		let _ = cmd.output();
		let _ = Command::new(bin())
			.args(["-f", &self.compose, "-p", &self.proj, "down", "-v"])
			.output();
	}
}

fn pod_id_for_project(proj: &str) -> Option<String> {
	let out = podman_cmd()
		.args(["pod", "inspect", proj, "--format", "{{.Id}}"])
		.output()
		.ok()?;
	if !out.status.success() {
		return None;
	}
	let id = String::from_utf8_lossy(&out.stdout).trim().to_string();
	if id.is_empty() {
		None
	} else {
		Some(id)
	}
}

fn container_pod_id(container: &str) -> Option<String> {
	let out = podman_cmd()
		.args(["inspect", container, "--format", "{{.Pod}}"])
		.output()
		.ok()?;
	if !out.status.success() {
		return None;
	}
	let id = String::from_utf8_lossy(&out.stdout).trim().to_string();
	if id.is_empty() {
		None
	} else {
		Some(id)
	}
}

fn container_uid_map(container: &str) -> Option<String> {
	let out = podman_cmd()
		.args(["exec", container, "cat", "/proc/self/uid_map"])
		.output()
		.ok()?;
	if !out.status.success() {
		return None;
	}
	Some(String::from_utf8_lossy(&out.stdout).to_string())
}

fn container_id_u(container: &str) -> Option<String> {
	let out = podman_cmd()
		.args(["exec", container, "id", "-u"])
		.output()
		.ok()?;
	if !out.status.success() {
		return None;
	}
	Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn first_mapping(map: &str) -> [u64; 3] {
	let fields: Vec<u64> = map
		.lines()
		.next()
		.expect("uid_map must not be empty")
		.split_whitespace()
		.map(|part| part.parse().expect("uid_map must contain integers"))
		.collect();
	fields.try_into().expect("uid_map must have three columns")
}

// Sum the size (third column) across every non-empty line of the map. The
// `auto` family may split a fragmented allocation across more than one
// range, and `auto:size=N` promises N usable container IDs, not N in one
// contiguous range. The split measured on this host on 2026-09-20 for
// `auto:size=2048` was two ranges of 1024 each:
//   0 1 1024
//   1024 2049 1024
// which totals 2048. Reading the first line alone, as the original
// assertion did, reported 1024 and missed the second range entirely.
//
// This helper is only used by the `auto` family. The `keep-id` case has a
// different shape (three lines, where the first is the user-specified
// range and the other two are plumbing) and asserts on the first line
// instead. See the comment at the size assertion below for the split.
fn total_size(map: &str) -> u64 {
	map.lines()
		.filter(|line| !line.trim().is_empty())
		.map(|line| {
			let fields: Vec<u64> = line
				.split_whitespace()
				.map(|part| part.parse().expect("uid_map must contain integers"))
				.collect();
			fields[2]
		})
		.sum()
}

// Snap the host state at the moment `up` fails so the assertion can tell a
// podup defect from a host whose subuid pool was already drained by some
// other caller. The error `not enough unused IDs in user namespace` reads
// exactly like a podup bug and is not one, so this is the only way to make
// the distinction from the message alone. It is only invoked from the
// `up`-failure branch, never the passing branch, so the passing path stays
// as cheap as it was.
fn userns_failure_context() -> String {
	let mut out = String::new();
	let user = std::env::var("USER").unwrap_or_default();
	for path in ["/etc/subuid", "/etc/subgid"] {
		match std::fs::read_to_string(path) {
			Ok(content) => {
				let prefix = format!("{user}:");
				let mine: Vec<&str> = content
					.lines()
					.filter(|line| line.starts_with(&prefix))
					.collect();
				if mine.is_empty() {
					out.push_str(&format!("{path}: <no entry for {user}>\n{content}\n"));
				} else {
					out.push_str(&format!("{path}: {}\n", mine.join("\n")));
				}
			}
			Err(e) => out.push_str(&format!("{path}: <read failed: {e}>\n")),
		}
	}
	match podman_cmd()
		.args(["ps", "-a", "--format", "{{.Names}} {{.Status}}"])
		.output()
	{
		Ok(o) => {
			out.push_str("podman ps -a:\n");
			out.push_str(&String::from_utf8_lossy(&o.stdout));
		}
		Err(e) => out.push_str(&format!("podman ps -a: <run failed: {e}>\n")),
	}
	match podman_cmd()
		.args(["pod", "ls", "--format", "{{.Name}}"])
		.output()
	{
		Ok(o) => {
			let count = String::from_utf8_lossy(&o.stdout)
				.lines()
				.filter(|line| !line.trim().is_empty())
				.count();
			out.push_str(&format!("live pods: {count}\n"));
		}
		Err(e) => out.push_str(&format!("live pods: <query failed: {e}>\n")),
	}
	out
}

#[cfg(all(unix, feature = "test-helpers"))]
#[tokio::test]
async fn userns_options_reach_a_pod_member() {
	// Serialize with the rest of the userns lane so two parallel runs do
	// not consume the host's subuid range faster than `down` releases it.
	// `USERNS` is the single shared mutex both userns modules lock; see
	// the comment at its declaration for the measured pool ceiling.
	let _guard = USERNS.lock().await;
	if podman().await.is_none() {
		return;
	}
	for case in CASES {
		let dir = tempfile::tempdir().unwrap();
		let proj = proj(case.label);
		let compose = write_compose(dir.path(), case.mode);
		let container = format!("{proj}-web-1");

		// `_guard` runs `down -v` on drop, covering the panic path.
		let _down = DownGuard {
			compose: compose.clone(),
			proj: proj.clone(),
		};

		let up = Command::new(bin())
			.args(["-f", &compose, "-p", &proj, "up", "-d"])
			.output()
			.unwrap();
		if !up.status.success() {
			panic!(
				"up failed for {} ({}): {}{}\nhost state at failure:\n{}",
				case.label,
				case.mode,
				String::from_utf8_lossy(&up.stdout),
				String::from_utf8_lossy(&up.stderr),
				userns_failure_context(),
			);
		}

		let uid_map = container_uid_map(&container).unwrap_or_default();
		let id_u = container_id_u(&container).unwrap_or_default();
		let pod_for_container = container_pod_id(&container);
		let pod_for_project = pod_id_for_project(&proj);

		// The size field of the map is what separates a working option path
		// from one that silently drops the option: a path that ignored the
		// option and fell back to the default would produce the `auto` answer
		// (`0 1 1024`) too, because the default IS `0 1 1024`. The size field
		// must therefore be read, not just the presence of a map. For the
		// `auto` family the value is the sum across every line because podman
		// may split a fragmented allocation; for `keep-id` it stays on the
		// first line because the other lines are fixed plumbing. See the
		// size assertion below for the exact rule per case.
		let mapping = first_mapping(&uid_map);
		// CodeQL `rust/cleartext-logging` fires on the format string of an
		// `assert_eq!` whose interpolated value has a `uid`-style name; this
		// test creates and tears down an alpine container in the same
		// function, so the value is the throwaway ID map of that container
		// (for example `0 1 1024`). The only sink for it is the assertion
		// message, and the message is the reason this control is worth
		// having: with the option dropped it reads `... got "0 1 1024"`,
		// while without the value it would read `left: 1024, right: 2048`.
		// Dropping the value to silence the alert trades a working control
		// for a heuristic false positive.
		// codeql[rust/cleartext-logging]
		assert_eq!(
			mapping[0], 0,
			"{}: container UID must start at 0, got {:?}",
			case.label, uid_map
		);
		// `mapping[1]` is the host-side offset that the kernel writes at
		// the second position of the first map line, and it is the value
		// that podman's `auto` allocator picks for the first range. That
		// value depends on what sub-ranges this host has already handed
		// out: measured on this host, `1` when no auto range has ever been
		// allocated, `61275` after several auto ranges have been handed
		// out, and increments of 1024 between consecutive `auto:size=2048`
		// allocations. Both ends of that range are correct: podup does not
		// choose the offset, the allocator does, and the choice is not
		// part of the option's contract.
		//
		// The hardcoded `assert_eq!(mapping[1], 1)` removed here passed
		// only when no auto allocation had ever happened on the host,
		// which is a host assertion, not a podup one, and therefore a
		// regression in the option path would still leave it green on a
		// pristine CI VM. The other three assertions in this loop already
		// prove the option reached the pod path: the size defaults to
		// `0 1 1024`, so a dropped option and a working option produce the
		// same first line on a pristine host, and size is the only value
		// that distinguishes them; `mapping[0] == 0` holds for both a
		// default and a user-supplied map because the kernel writes the
		// inside-namespace start only after the userns is configured at
		// the container level; and the pod-membership check at the
		// bottom of the loop proves the option travelled through the
		// pod spec rather than the container spec alone.
		// codeql[rust/cleartext-logging] see the first site for the reason.
		// For the `auto` family, the size the option promises is the TOTAL
		// count of mapped container IDs across every line, because podman
		// may split a fragmented allocation across several ranges. The split
		// measured on this host on 2026-09-20 for `auto:size=2048` was two
		// ranges of 1024 each (`0 1 1024` and `1024 2049 1024`), totalling
		// 2048. Reading the first line alone reported 1024 and missed the
		// second range; summing the third column across every line reports
		// 2048 and matches what the option actually delivered.
		//
		// For `keep-id:uid=N,gid=M` the shape is different: podman writes a
		// three-line map whose first range is the user-specified N and whose
		// other two ranges are plumbing to keep the rest of the namespace
		// usable. The sum across all lines for that case is 65537 (321 + 1
		// + 65215), not 321, so the assertion stays on the first line. The
		// two families assert what is true for each rather than forcing one
		// rule over both.
		let size_value = if case.mode.starts_with("keep-id") {
			mapping[2]
		} else {
			total_size(&uid_map)
		};
		assert_eq!(
			size_value,
			u64::from(case.expect_size),
			"{}: uid_map size must be {} for userns_mode {:?}, got {:?}",
			case.label,
			case.expect_size,
			case.mode,
			uid_map,
		);
		// codeql[rust/cleartext-logging] see the first site for the reason.
		assert_eq!(
			id_u, case.expect_uid,
			"{}: id -u must be {} for userns_mode {:?}, got {:?}",
			case.label, case.expect_uid, case.mode, id_u
		);

		// Pod membership: this is the assertion that makes the test exercise
		// the pod path. Without it, a regression that silently created the
		// container outside the pod would leave the mapping assertions above
		// passing while the pod path went unverified.
		let pod_for_container = pod_for_container.unwrap_or_default();
		let pod_for_project = pod_for_project.unwrap_or_default();
		assert!(
			!pod_for_container.is_empty(),
			"{}: container {} must be in a pod (Pod field is empty)",
			case.label,
			container,
		);
		assert_eq!(
			pod_for_container, pod_for_project,
			"{}: container {} must be in the project pod; container.Pod={:?}, project pod id={:?}",
			case.label, container, pod_for_container, pod_for_project,
		);
	}
}
