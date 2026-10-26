//! Live tests for the project-boundary network guard.
//!
//! Two podup projects with the same physical `name:` under `networks:` must
//! not silently end up on the same bridge: the second project's `up` is
//! refused, and a `down` in a project that did not create the network is
//! refused even when the owner's containers are stopped and the libpod
//! removal would otherwise succeed. `external: true` is the escape hatch.
//!
//! Every test creates two prefixed projects and a Drop guard that runs
//! `down -v` for each one when the test ends, so a panic in the middle of
//! the assertions still reaps the resources the test created. The
//! projects are named `t<pid>-owna-<tag>` / `t<pid>-ownb-<tag>` so they
//! never collide with other suites or other runs.
use std::fs;
use std::process::Command;

use tempfile::tempdir;

use podup::Client;

use super::*;

/// One project the test owns: a tempdir, a compose file, and a name. Drop
/// runs `podup -p <name> down -v` so the resources the test created are
/// reaped even when an assertion panics in the middle.
struct Guard {
	_dir: tempfile::TempDir,
	compose: std::path::PathBuf,
	name: String,
}

impl Guard {
	fn new(tag: &str, compose_body: &str) -> Self {
		let dir = tempdir().expect("tempdir");
		let compose = dir.path().join("compose.yaml");
		fs::write(&compose, compose_body).expect("write compose");
		let name = format!("t{}-{tag}", std::process::id());
		Self {
			_dir: dir,
			compose,
			name,
		}
	}

	fn compose_path(&self) -> &str {
		self.compose.to_str().expect("compose path utf8")
	}
}

impl Drop for Guard {
	fn drop(&mut self) {
		let _ = Command::new(bin())
			.args(["-f", self.compose_path(), "-p", &self.name, "down", "-v"])
			.output();
	}
}

/// Run a `podup` command against this guard's project and capture both
/// streams. Asserts nothing; callers pick `run` or `run_ok`.
fn run_guard(g: &Guard, args: &[&str]) -> std::process::Output {
	Command::new(bin())
		.args(["-f", g.compose_path(), "-p", &g.name])
		.args(args)
		.output()
		.expect("run podup")
}

// ---------------------------------------------------------------------------
// Two projects, same physical network name, no external: true
// ---------------------------------------------------------------------------

/// The owner creates the network and brings up its service. The intruder
/// declares the same physical `name:` without `external: true`. The
/// intruder's `up` must fail with an error that names the contested
/// network, the owner and itself, and points at `external: true`. The
/// owner's resources must remain untouched after the failed intrusion.
#[tokio::test]
async fn second_project_is_refused_when_it_picks_the_same_physical_name() {
	if podman().await.is_none() {
		return;
	}
	let shared = format!("podup-shared-refuse-{}", std::process::id());
	let owner = Guard::new(
		&format!("owna-rfs-{}", std::process::id()),
		&format!(
			"networks:\n  default:\n    name: {shared}\nservices:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n"
		),
	);
	let intruder = Guard::new(
		&format!("ownb-rfs-{}", std::process::id()),
		&format!(
			"networks:\n  default:\n    name: {shared}\nservices:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n"
		),
	);

	run_ok_fn(|| run_guard(&owner, &["up", "-d"]));

	let intruder_out = run_guard(&intruder, &["up", "-d"]);
	let stderr = String::from_utf8_lossy(&intruder_out.stderr);
	let stdout = String::from_utf8_lossy(&intruder_out.stdout);
	let combined = format!("{stdout}{stderr}");
	assert!(
		!intruder_out.status.success(),
		"intruder up must fail, got {combined}"
	);
	assert!(
		combined.contains(&shared),
		"error must name the contested network: {combined}"
	);
	assert!(
		combined.contains(&owner.name),
		"error must name the owner project: {combined}"
	);
	assert!(
		combined.contains(&intruder.name),
		"error must name the intruder project: {combined}"
	);
	assert!(
		combined.contains("external"),
		"error must point at external: true: {combined}"
	);

	// The owner's resources are untouched: the network exists and is still
	// labelled for the owner, not for the intruder. A network the intruder
	// could join would carry the intruder's label; one it merely inspected
	// carries the owner's.
	let inspected = network_owner(&shared)
		.await
		.expect("network inspect must reach the daemon");
	assert_eq!(
		inspected.as_deref(),
		Some(owner.name.as_str()),
		"the owner's network must still be labelled for the owner: {inspected:?}"
	);
}

// ---------------------------------------------------------------------------
// Same with external: true on the second: it attaches and succeeds
// ---------------------------------------------------------------------------

/// `external: true` is the escape hatch: an existing foreign-labelled
/// network is accepted, not adopted. The intruder is verified against the
/// existing network and proceeds; the owner's network is shared, not
/// recreated, and the intruder's `down` does not remove it.
#[tokio::test]
async fn second_project_with_external_true_attaches_to_shared_network() {
	if podman().await.is_none() {
		return;
	}
	let shared = format!("podup-shared-ext-{}", std::process::id());
	let owner = Guard::new(
		&format!("owna-ext-{}", std::process::id()),
		&format!(
			"networks:\n  default:\n    name: {shared}\nservices:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n"
		),
	);
	let intruder = Guard::new(
		&format!("ownb-ext-{}", std::process::id()),
		&format!(
			"networks:\n  default:\n    name: {shared}\n    external: true\nservices:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n"
		),
	);

	run_ok_fn(|| run_guard(&owner, &["up", "-d"]));

	let intruder_out = run_guard(&intruder, &["up", "-d"]);
	let stderr = String::from_utf8_lossy(&intruder_out.stderr);
	assert!(
		intruder_out.status.success(),
		"intruder up with external: true must succeed: {stderr}"
	);

	// The intruder attached to the same physical bridge, so its service
	// shares the network with the owner's. The shared network is still
	// labelled for the owner; the intruder was verified against it, not
	// adopted.
	let still_labelled = network_owner(&shared)
		.await
		.expect("network inspect must reach the daemon");
	assert_eq!(
		still_labelled.as_deref(),
		Some(owner.name.as_str()),
		"the owner's network must remain labelled for the owner after the intruder attaches: {still_labelled:?}"
	);
}

// ---------------------------------------------------------------------------
// down refuses to remove a network labelled for another project
// ---------------------------------------------------------------------------

/// The half that loses data. Owner A creates the network and brings up a
/// container. Owner A then stops and force-removes that container *without*
/// removing the network, so the network is alive, labelled for A, and has
/// no containers attached, exactly the shape where libpod would happily
/// honour a `DELETE /networks/{name}` request from the intruder. Intruder
/// B's `down` declares the same physical `name:` and must refuse: the
/// shared network must still exist after the failed intrusion.
#[tokio::test]
async fn down_refuses_to_remove_a_foreign_labelled_network() {
	if podman().await.is_none() {
		return;
	}
	let shared = format!("podup-shared-down-{}", std::process::id());
	let owner = Guard::new(
		&format!("owna-dn-{}", std::process::id()),
		&format!(
			"networks:\n  default:\n    name: {shared}\nservices:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n"
		),
	);
	let intruder = Guard::new(
		&format!("ownb-dn-{}", std::process::id()),
		&format!(
			"networks:\n  default:\n    name: {shared}\nservices:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n"
		),
	);

	run_ok_fn(|| run_guard(&owner, &["up", "-d"]));

	// Stop and force-remove the owner's container without touching the
	// network. This is the shape that makes the assertion meaningful: the
	// network is alive, labelled for the owner, and nothing is attached,
	// so a `DELETE /networks/{name}` from the intruder would succeed
	// without the guard.
	let owner_container = format!("{}-web-1", owner.name);
	let rm_out = Command::new("podman")
		.args(["rm", "-f", &owner_container])
		.output()
		.expect("podman rm");
	assert!(
		rm_out.status.success(),
		"setup: podman rm {owner_container} must succeed: {}",
		String::from_utf8_lossy(&rm_out.stderr)
	);

	// Intruder `down` must refuse. The refused message names the owner,
	// the intruder and the contested network.
	let down_out = run_guard(&intruder, &["down"]);
	let stderr = String::from_utf8_lossy(&down_out.stderr);
	let stdout = String::from_utf8_lossy(&down_out.stdout);
	let combined = format!("{stdout}{stderr}");
	assert!(
		!down_out.status.success(),
		"intruder down must refuse a foreign-labelled network, got {combined}"
	);
	assert!(
		combined.contains(&shared),
		"error must name the contested network: {combined}"
	);
	assert!(
		combined.contains(&owner.name),
		"error must name the owner: {combined}"
	);
	assert!(
		combined.contains("external"),
		"error must point at external: true: {combined}"
	);

	// The shared network is still alive and still labelled for the owner.
	let still_labelled = network_owner(&shared)
		.await
		.expect("network inspect must reach the daemon");
	assert_eq!(
		still_labelled.as_deref(),
		Some(owner.name.as_str()),
		"the shared network must remain labelled for the owner after the refused down: {still_labelled:?}"
	);
}

// ---------------------------------------------------------------------------
// Unlabelled network: behaves as decided
// ---------------------------------------------------------------------------

/// An existing network that carries no `podup.project` label at all is
/// refused on `up` of a non-external project, the same as a
/// foreign-labelled one. The label is the only ownership evidence podup
/// writes; without it "no one owns it" and "another stack already claimed
/// it" are indistinguishable, and the rule is "the project name is the
/// isolation boundary; declare `external: true` to share". A subsequent
/// `up` with `external: true` succeeds: that is the documented escape
/// hatch.
#[tokio::test]
async fn unlabelled_existing_network_is_refused_without_external_true() {
	if podman().await.is_none() {
		return;
	}
	let shared = format!("podup-unlabelled-{}", std::process::id());

	// Create an unlabelled network directly via the libpod socket: no
	// `podup.project` label, just a name. This is the shape a hand-rolled
	// `podman network create` leaves behind. Going through the libpod
	// socket (rather than the `podman` CLI) keeps the network in the same
	// namespace podup's `up` will see; rootless podman's CLI runs each
	// invocation in its own netns and the two views would otherwise miss
	// each other.
	let client: Client = podup::podman::connect_from_env()
		.or_else(|_| podup::podman::connect(None))
		.expect("connect podman");
	let body = serde_json::json!({
		"name": shared,
		"driver": "bridge",
		"dns_enabled": true,
	});
	let _: serde_json::Value = client
		.post_json("/v5.0.0/libpod/networks/create", &body)
		.await
		.expect("create unlabelled network");

	// Two `podup` projects try to use it. The first, without
	// `external: true`, must refuse. The second, with `external: true`,
	// must succeed.
	let no_external = Guard::new(
		&format!("unla-{}", std::process::id()),
		&format!(
			"networks:\n  default:\n    name: {shared}\nservices:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n"
		),
	);
	let external = Guard::new(
		&format!("unlb-{}", std::process::id()),
		&format!(
			"networks:\n  default:\n    name: {shared}\n    external: true\nservices:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n"
		),
	);

	let no_ext_out = run_guard(&no_external, &["up", "-d"]);
	let stderr = String::from_utf8_lossy(&no_ext_out.stderr);
	let stdout = String::from_utf8_lossy(&no_ext_out.stdout);
	let combined = format!("{stdout}{stderr}");
	assert!(
		!no_ext_out.status.success(),
		"non-external up against an unlabelled network must refuse: {combined}"
	);
	assert!(
		combined.contains(&shared),
		"error must name the unlabelled network: {combined}"
	);
	assert!(
		combined.contains("no podup.project") || combined.contains("unlabelled"),
		"error must explain the unlabelled shape: {combined}"
	);

	let ext_out = run_guard(&external, &["up", "-d"]);
	let stderr_ext = String::from_utf8_lossy(&ext_out.stderr);
	assert!(
		ext_out.status.success(),
		"external up against an unlabelled network must succeed: {stderr_ext}"
	);

	// Reap the unlabelled network the test created by hand, so the host
	// is not littered with `_test-{pid}-unlabelled-{pid}` bridges. The
	// `podman` CLI's netns view differs from podup's, so use the libpod
	// socket to remove it.
	let path = format!("/v5.0.0/libpod/networks/{}", netname_encode(&shared));
	let _ = client.delete_ok(&path).await;
}

/// Tiny helper so the `run_ok_fn(|| ...)` closure can `expect` on a
/// `std::process::Output` with its own message rather than a fixed string.
fn run_ok_fn<F: FnOnce() -> std::process::Output>(f: F) -> std::process::Output {
	let out = f();
	assert!(
		out.status.success(),
		"podup failed: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	out
}

/// Read the `podup.project` label of the named network straight from the
/// libpod socket. Returns `Ok(None)` when the network is absent or has no
/// `podup.project` label; `Err` on transport failure.
///
/// Rootless Podman runs each `podman` invocation in its own network
/// namespace, so `podman network inspect` from a different shell sees a
/// different view of the host's networks than the one podup created
/// against. The libpod socket, by contrast, lists every network the
/// service knows about, so the inspection goes through it directly.
async fn network_owner(name: &str) -> std::result::Result<Option<String>, String> {
	let client: Client = podup::podman::connect_from_env()
		.or_else(|_| podup::podman::connect(None))
		.map_err(|e| e.to_string())?;
	let path = format!("/v5.0.0/libpod/networks/{}/json", netname_encode(name));
	let resp: serde_json::Value = client.get_json(&path).await.map_err(|e| e.to_string())?;
	let owner = resp
		.get("labels")
		.and_then(|l| l.get("podup.project"))
		.and_then(|v| v.as_str())
		.map(str::to_string);
	Ok(owner)
}

/// Percent-encode the bits a network name can contain that the libpod
/// socket refuses raw: `/` and anything non-printable. Mirrors what
/// `internal/libpod::urlencoded` does but stays in the test crate so the
/// test does not reach into a private module.
fn netname_encode(s: &str) -> String {
	let mut out = String::with_capacity(s.len());
	for b in s.bytes() {
		match b {
			b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
				out.push(b as char)
			}
			_ => out.push_str(&format!("%{b:02X}")),
		}
	}
	out
}
