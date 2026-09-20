//! How many times a warm `up` asks libpod what a service's tag resolves to.
//!
//! Every test here drives a whole `up` against the recording fake and counts
//! `GET /images/{tag}/json`. The fixtures use `pull_policy: never` unless they
//! say otherwise: the prefetch stage's presence check reads the same URL, and
//! `never` is the one policy under which nothing but the replica loop does, so
//! the number is that loop's alone.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, MutexGuard};

use crate::compose::types::ComposeFile;
use crate::engine::container::config_hash;
use crate::engine::fake_podman::{self, FakePodman};
use crate::engine::Engine;

const CURRENT: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const STALE: &str = "2222222222222222222222222222222222222222222222222222222222222222";

fn engine_with(client: crate::libpod::Client) -> Engine {
	Engine::with_base_dir(client, "proj".into(), std::env::temp_dir())
}

/// The container list of a project that is already up: every replica of every
/// service, carrying the config hash `up` is about to compute, bound to the
/// image ID `bound_to` picks for it. A fixture that got the hash wrong would
/// be recreated before the image question is ever asked, so the hash comes
/// from the same function `up` uses.
fn running(file: &ComposeFile, bound_to: impl Fn(&str) -> &'static str) -> String {
	let e = engine_with(crate::libpod::Client::new("/nonexistent.sock"));
	let mut entries = Vec::new();
	for (name, service) in &file.services {
		let digests = e
			.uploaded_file_digests
			.lock()
			.expect("uploaded_file_digests mutex poisoned");
		let hash =
			config_hash(service, file, &e.project, &e.base_dir, &digests).expect("config hash");
		for container in e.replica_names(name, service) {
			entries.push(serde_json::json!({
				"Id": container,
				"Names": [container],
				"ImageID": bound_to(&container),
				"Labels": {
					"podup.project": "proj",
					"podup.service": name,
					"podup.config-hash": hash,
				},
			}));
		}
	}
	serde_json::Value::Array(entries).to_string()
}

/// A host whose project is already up (`containers`), answering every image
/// inspect through `inspect`, which is told how many it has answered before.
fn warm_host(
	containers: String,
	inspect: impl Fn(usize) -> (u16, String) + Send + Sync + 'static,
) -> FakePodman {
	let asked = Arc::new(AtomicUsize::new(0));
	fake_podman::start(move |method, target| {
		if method == "GET" && is_image_inspect(target) {
			inspect(asked.fetch_add(1, Ordering::SeqCst))
		} else if method == "GET" && target.contains("/containers/json") {
			(200, containers.clone())
		} else if method == "POST" && target.contains("/containers/create") {
			(200, "{}".to_string())
		} else if method == "POST" && target.contains("/start") {
			(200, String::new())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	})
}

fn is_image_inspect(target: &str) -> bool {
	target.contains("/images/") && target.ends_with("/json")
}

fn resolves_to(id: &'static str) -> impl Fn(usize) -> (u16, String) + Send + Sync + 'static {
	move |_| (200, format!(r#"{{"Id":"{id}"}}"#))
}

/// The requests the fake answered whose line contains `needle`.
fn seen(fake: &FakePodman, needle: &str) -> Vec<String> {
	let requests = fake.requests.lock().unwrap();
	requests
		.iter()
		.filter(|r| r.contains(needle))
		.cloned()
		.collect()
}

fn image_inspects(fake: &FakePodman) -> Vec<String> {
	let requests = fake.requests.lock().unwrap();
	requests
		.iter()
		.filter(|r| r.starts_with("GET ") && is_image_inspect(r))
		.cloned()
		.collect()
}

async fn up(fake: &FakePodman, file: &ComposeFile) -> crate::error::Result<()> {
	engine_with(fake.client())
		.up_with_options(file, false, &[], &[], false, false, false, false)
		.await
}

/// Lock + `PODUP_MAX_REPLICAS` pin taken by every test in this file.
///
/// `up()` calls `check_replica_limit` which reads `PODUP_MAX_REPLICAS`, so
/// the three-replica fixtures below need it above 3 for the duration of
/// each test, including the `await`. Without the lock, the body of
/// `scale_tests::replica_limit_default_and_env_override` can `set_var(2)`
/// between this test's setup and its `up()` call and the three replicas
/// fail. The lock is the same one that test takes, so the two files
/// serialise on the env var and cannot interleave a write mid-call.
struct MaxReplicasGuard {
	_lock: MutexGuard<'static, ()>,
	prev: Option<String>,
}

impl Drop for MaxReplicasGuard {
	fn drop(&mut self) {
		match self.prev.take() {
			Some(v) => std::env::set_var("PODUP_MAX_REPLICAS", v),
			None => std::env::remove_var("PODUP_MAX_REPLICAS"),
		}
	}
}

fn pin_max_replicas_for_test() -> MaxReplicasGuard {
	let lock = crate::engine::lifecycle::scale_tests::MAX_REPLICAS_TEST_LOCK
		.lock()
		.unwrap();
	let prev = std::env::var("PODUP_MAX_REPLICAS").ok();
	std::env::set_var(
		"PODUP_MAX_REPLICAS",
		crate::engine::lifecycle::scale::DEFAULT_MAX_REPLICAS.to_string(),
	);
	MaxReplicasGuard { _lock: lock, prev }
}

const THREE_REPLICAS: &str =
	"services:\n  web:\n    image: shared\n    pull_policy: never\n    deploy:\n      replicas: 3\n";

/// Three replicas of one service ask what their tag resolves to once, not once
/// each. The three starts and zero creates are the control on the fixture: they
/// say all three replicas reached the image comparison and were kept by it, so
/// a count of one is not three replicas that never asked.
#[tokio::test]
async fn three_replicas_of_one_image_inspect_it_once() {
	let _replicas_guard = pin_max_replicas_for_test();
	let file = crate::parse_str(THREE_REPLICAS).unwrap();
	let fake = warm_host(running(&file, |_| CURRENT), resolves_to(CURRENT));

	up(&fake, &file).await.expect("a warm up must succeed");

	let inspects = image_inspects(&fake);
	assert_eq!(
		inspects.len(),
		1,
		"three replicas of one service must inspect their image once: {inspects:?}"
	);
	assert_eq!(seen(&fake, "/start").len(), 3, "every replica is kept");
	assert_eq!(seen(&fake, "/containers/create").len(), 0);
}

/// The default `missing` policy adds the prefetch stage's one presence check on
/// the same URL and nothing else: two requests for three replicas, where it
/// used to be four.
#[tokio::test]
async fn the_default_policy_adds_only_the_prefetch_presence_check() {
	let _replicas_guard = pin_max_replicas_for_test();
	let file =
		crate::parse_str("services:\n  web:\n    image: shared\n    deploy:\n      replicas: 3\n")
			.unwrap();
	let fake = warm_host(running(&file, |_| CURRENT), resolves_to(CURRENT));

	up(&fake, &file).await.expect("a warm up must succeed");

	let inspects = image_inspects(&fake);
	assert_eq!(
		inspects.len(),
		2,
		"one presence check plus one lookup for the replicas: {inspects:?}"
	);
	assert_eq!(seen(&fake, "/images/pull").len(), 0);
	assert_eq!(seen(&fake, "/start").len(), 3);
}

/// Two services on one image ask once EACH, two in total, and that is on
/// purpose. An answer shared between services would outlive the other
/// service's pull or build of that very tag, which is the moment the tag moves;
/// the answer is only shared between the replicas of one service, after that
/// service's own image is in place.
#[tokio::test]
async fn two_services_sharing_an_image_inspect_it_once_each() {
	let _replicas_guard = pin_max_replicas_for_test();
	let file = crate::parse_str(
		"services:\n  a:\n    image: shared\n    pull_policy: never\n    deploy:\n      replicas: 3\n  b:\n    image: shared\n    pull_policy: never\n    deploy:\n      replicas: 3\n",
	)
	.unwrap();
	let fake = warm_host(running(&file, |_| CURRENT), resolves_to(CURRENT));

	up(&fake, &file).await.expect("a warm up must succeed");

	let inspects = image_inspects(&fake);
	assert_eq!(
		inspects.len(),
		2,
		"six replicas across two services must inspect once per service: {inspects:?}"
	);
	assert_eq!(seen(&fake, "/start").len(), 6);
}

/// Two services on two images: one request per tag, each naming its own.
#[tokio::test]
async fn two_images_are_inspected_once_each() {
	let _replicas_guard = pin_max_replicas_for_test();
	let file = crate::parse_str(
		"services:\n  a:\n    image: first\n    pull_policy: never\n    deploy:\n      replicas: 3\n  b:\n    image: second\n    pull_policy: never\n    deploy:\n      replicas: 3\n",
	)
	.unwrap();
	let fake = warm_host(running(&file, |_| CURRENT), resolves_to(CURRENT));

	up(&fake, &file).await.expect("a warm up must succeed");

	let inspects = image_inspects(&fake);
	assert_eq!(inspects.len(), 2, "one inspect per image: {inspects:?}");
	for tag in ["/images/first/json", "/images/second/json"] {
		assert_eq!(
			inspects.iter().filter(|r| r.contains(tag)).count(),
			1,
			"{tag} must be asked exactly once: {inspects:?}"
		);
	}
}

/// Asking once must not stop `up` noticing a moved tag. Every replica is bound
/// to the image the tag used to name, so every replica is recreated.
#[tokio::test]
async fn a_moved_tag_still_recreates_every_replica() {
	let _replicas_guard = pin_max_replicas_for_test();
	let file = crate::parse_str(THREE_REPLICAS).unwrap();
	let fake = warm_host(running(&file, |_| STALE), resolves_to(CURRENT));

	up(&fake, &file)
		.await
		.expect("a recreating up must succeed");

	assert_eq!(image_inspects(&fake).len(), 1);
	assert_eq!(
		seen(&fake, "/containers/create").len(),
		3,
		"every replica bound to the old image is recreated"
	);
}

/// The shared answer is compared per replica: the one replica still bound to
/// the old image is recreated and the two current ones are left alone.
#[tokio::test]
async fn only_the_replica_bound_to_a_stale_image_is_recreated() {
	let _replicas_guard = pin_max_replicas_for_test();
	let file = crate::parse_str(THREE_REPLICAS).unwrap();
	let containers = running(&file, |container| {
		if container == "proj-web-2" {
			STALE
		} else {
			CURRENT
		}
	});
	let fake = warm_host(containers, resolves_to(CURRENT));

	up(&fake, &file).await.expect("a mixed up must succeed");

	assert_eq!(image_inspects(&fake).len(), 1);
	let created = seen(&fake, "/containers/create");
	assert_eq!(created.len(), 1, "only the stale replica: {created:?}");
	let started = seen(&fake, "/start");
	assert!(
		started.iter().any(|r| r.contains("proj-web-1/start"))
			&& started.iter().any(|r| r.contains("proj-web-3/start")),
		"the current replicas are kept and started: {started:?}"
	);
}

/// A first `up` has no container to compare, so it asks nothing. The lookup is
/// made by the first replica that needs it, not ahead of the loop.
#[tokio::test]
async fn a_cold_up_inspects_nothing() {
	let _replicas_guard = pin_max_replicas_for_test();
	let file = crate::parse_str(THREE_REPLICAS).unwrap();
	let fake = warm_host("[]".to_string(), resolves_to(CURRENT));

	up(&fake, &file).await.expect("a cold up must succeed");

	let inspects = image_inspects(&fake);
	assert_eq!(
		inspects.len(),
		0,
		"nothing to compare against: {inspects:?}"
	);
	assert_eq!(seen(&fake, "/containers/create").len(), 3);
}

/// A failed inspect fails `up` with the transport error it always did, and no
/// replica is kept or replaced on the strength of an answer nobody got.
#[tokio::test]
async fn a_failing_inspect_fails_up_with_the_podman_error() {
	let _replicas_guard = pin_max_replicas_for_test();
	let file = crate::parse_str(THREE_REPLICAS).unwrap();
	let fake = warm_host(running(&file, |_| CURRENT), |_| {
		(500, r#"{"message":"storage is on fire"}"#.to_string())
	});

	let err = up(&fake, &file)
		.await
		.expect_err("an inspect that fails must fail up");

	assert!(
		matches!(&err, crate::error::ComposeError::Podman(e) if e.is_status(500)),
		"the 500 surfaces as the Podman error it is, got {err:?}"
	);
	assert_eq!(seen(&fake, "/start").len(), 0);
	assert_eq!(seen(&fake, "/containers/create").len(), 0);
}

/// A failure is not remembered. The first replica's inspect fails and fails
/// that replica; the next one asks again, gets an answer, and that answer
/// serves the rest: two requests, two replicas kept, and `up` still reports the
/// first replica's error.
#[tokio::test]
async fn a_failed_inspect_is_not_remembered_for_the_next_replica() {
	let _replicas_guard = pin_max_replicas_for_test();
	let file = crate::parse_str(THREE_REPLICAS).unwrap();
	let fake = warm_host(running(&file, |_| CURRENT), |asked_before| {
		if asked_before == 0 {
			(500, r#"{"message":"try again"}"#.to_string())
		} else {
			(200, format!(r#"{{"Id":"{CURRENT}"}}"#))
		}
	});

	let err = up(&fake, &file)
		.await
		.expect_err("the replica whose inspect failed must fail up");

	assert!(
		matches!(&err, crate::error::ComposeError::Podman(e) if e.is_status(500)),
		"got {err:?}"
	);
	let inspects = image_inspects(&fake);
	assert_eq!(
		inspects.len(),
		2,
		"one failure, then one answer shared by the rest: {inspects:?}"
	);
	assert_eq!(seen(&fake, "/start").len(), 2, "the other two are kept");
}
