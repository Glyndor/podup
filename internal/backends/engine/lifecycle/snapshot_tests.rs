//! The bulk project snapshot `up` builds from one container-list response.
//!
//! Two facts about it are pinned here: it carries the `podup.oneoff` label
//! through, and the scale reconciliation leaves a one-off container alone.

use crate::engine::lifecycle::ExistingContainer;
use crate::libpod::types::container::ContainerListEntry;

/// The snapshot must record which containers are one-off `run` containers.
///
/// The label is the only marker that says so: the name carries the run's PID
/// instead of a replica index, and the service label is the same one the
/// replicas carry.
#[test]
fn snapshot_records_the_oneoff_label() {
	let entries: Vec<ContainerListEntry> = serde_json::from_str(
		r#"[
			{"Names":["/proj-web-1"],"Labels":{"podup.service":"web"}},
			{"Names":["/proj-web-run-4242"],"Labels":{"podup.service":"web","podup.oneoff":"true"}}
		]"#,
	)
	.expect("the container list must deserialize");

	let existing = ExistingContainer::snapshot(&entries);

	let replica = existing
		.get("proj-web-1")
		.expect("the replica is in the snapshot");
	assert!(!replica.oneoff, "a replica is not a one-off container");
	let oneoff = existing
		.get("proj-web-run-4242")
		.expect("the one-off container is in the snapshot");
	assert!(
		oneoff.oneoff,
		"the `podup.oneoff=true` label must reach the snapshot"
	);
}

/// A container created by `run` is not a replica of the service it names, so
/// reconciling the replica count must leave it running.
///
/// `run -d web` creates `proj-web-run-<pid>` with the same `podup.service`
/// label as the replicas. Scaling the service reads every container carrying
/// that label as a replica candidate, and the one-off's name is not in the
/// desired set, so it used to be stopped and deleted — losing the writable
/// layer of a task that was still running.
#[cfg(unix)]
#[tokio::test]
async fn scale_leaves_a_one_off_container_running() {
	use crate::engine::fake_podman;
	use crate::engine::Engine;

	// The container list the `up` walk reads: one replica of `web` and one
	// one-off container of the same service.
	let live = r#"[
		{"Names":["/proj-web-1"],"State":"running","Labels":{"podup.service":"web"}},
		{"Names":["/proj-web-run-4242"],"State":"running","Labels":{"podup.service":"web","podup.oneoff":"true"}}
	]"#;
	let fake = fake_podman::start(move |method, target| {
		if method == "GET" && target.contains("/containers/json") {
			(200, live.to_string())
		} else {
			(200, String::new())
		}
	});
	let engine = Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir());

	let entries: Vec<ContainerListEntry> =
		serde_json::from_str(live).expect("the list deserializes");
	let existing = ExistingContainer::snapshot(&entries);

	// Scale `web` down to zero: every replica is surplus, and the one-off
	// container must not be treated as one.
	engine
		.remove_surplus_replicas(
			"web",
			&crate::compose::types::Service::default(),
			0,
			&existing,
		)
		.await
		.expect("the reconciliation must succeed");

	let seen = fake.requests.lock().unwrap();
	assert!(
		!seen.iter().any(|r| r.contains("proj-web-run-4242")),
		"a one-off container must not be stopped or removed by the scale reconciliation: {seen:?}"
	);
	assert!(
		seen.iter().any(|r| r.contains("proj-web-1")),
		"the surplus replica must still be reclaimed: {seen:?}"
	);
}
