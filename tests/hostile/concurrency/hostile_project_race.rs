//! Two `podup` processes on one project must not interleave.
//!
//! `up`, `down` and the other lifecycle commands delete and recreate
//! containers, so two of them at once can undo each other's steps. They take a
//! per-project lock. Each case starts one process, waits until it is inside a
//! slow container create, which it does while holding the lock, and starts a
//! second one. The second has to report that it is waiting for the lock and has
//! to make its changes after the first one's. Resources use the `vzh-` prefix
//! and live only in the fake engine.

#![cfg(unix)]

use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[path = "../support/fake_engine.rs"]
mod fake_engine;
#[path = "../support/host_files.rs"]
mod host_files;

use fake_engine::{FakeEngine, Reply};
use host_files::Running;

/// Set once the first container create is being served.
static CREATING: AtomicBool = AtomicBool::new(false);
static SLOWED: AtomicBool = AtomicBool::new(false);

/// The first container create takes long enough for a second process to start.
fn slow_create(line: &str, _: &str) -> Option<Reply> {
	if line.starts_with("POST") && line.ends_with("/containers/create HTTP/1.1") {
		CREATING.store(true, Ordering::SeqCst);
		if !SLOWED.swap(true, Ordering::SeqCst) {
			std::thread::sleep(Duration::from_millis(700));
		}
	}
	None
}

/// The cases share the flags above, so they take turns.
static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn start(fake: &FakeEngine, compose: &str, args: &[&str]) -> Running {
	let mut command = Command::new(env!("CARGO_BIN_EXE_podup"));
	command
		.current_dir(fake.dir())
		.args(["--socket", fake.dir().join("podman.sock").to_str().unwrap()])
		.args(["-f", compose, "-p", "vzh-race"])
		.args(args)
		.env("HOME", fake.dir())
		.env("XDG_CONFIG_HOME", fake.dir().join("config"))
		.env("XDG_DATA_HOME", fake.dir().join("data"))
		.env("XDG_RUNTIME_DIR", fake.dir())
		.env("NO_COLOR", "1")
		// The wait for the lock is logged at info level.
		.env("RUST_LOG", "info");
	Running::command(command)
}

fn wait_until_creating() {
	let begun = Instant::now();
	while !CREATING.load(Ordering::SeqCst) {
		assert!(begun.elapsed() < Duration::from_secs(10), "no create seen");
		std::thread::sleep(Duration::from_millis(5));
	}
}

fn setup() -> FakeEngine {
	CREATING.store(false, Ordering::SeqCst);
	SLOWED.store(false, Ordering::SeqCst);
	let fake = FakeEngine::start(slow_create);
	// The first file has two services, so the process that runs it has several
	// requests still to make after the slow create.
	for (file, services) in [
		("one.yaml", &["web1a", "web1b"][..]),
		("two.yaml", &["web2"][..]),
	] {
		let body: String = services
			.iter()
			.map(|s| format!("  {s}:\n    image: nginx\n"))
			.collect();
		std::fs::write(fake.dir().join(file), format!("services:\n{body}")).unwrap();
	}
	fake
}

/// The request lines that mention `needle`, in the order the engine saw them.
fn order(fake: &FakeEngine, needles: &[&str]) -> Vec<usize> {
	fake.requests()
		.iter()
		.filter_map(|r| {
			let text = String::from_utf8_lossy(r);
			needles.iter().position(|n| text.contains(n))
		})
		.collect()
}

#[test]
fn two_up_on_one_project_do_not_interleave_their_requests() {
	let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
	let fake = setup();
	let mut first = start(&fake, "one.yaml", &["up", "-d"]);
	wait_until_creating();
	let mut second = start(&fake, "two.yaml", &["up", "-d"]);
	assert!(first.wait().success(), "{}", first.text());
	assert!(second.wait().success(), "{}", second.text());
	assert!(
		second.text().contains("waiting for project lock"),
		"the second up did not wait: {}",
		second.text()
	);
	// 0 for a request about the first file's service, 1 for the second's. One
	// run after the other reads 0..0 1..1; a process that did not wait puts a 1
	// between two 0s.
	let seen = order(&fake, &["web1", "web2"]);
	assert!(seen.contains(&0) && seen.contains(&1), "{seen:?}");
	let changes = seen.windows(2).filter(|w| w[0] != w[1]).count();
	assert_eq!(changes, 1, "the requests interleave: {seen:?}");
}

#[test]
fn down_started_during_up_waits_for_it() {
	let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
	let fake = setup();
	let mut up = start(&fake, "one.yaml", &["up", "-d"]);
	wait_until_creating();
	let mut down = start(&fake, "one.yaml", &["down"]);
	assert!(up.wait().success(), "{}", up.text());
	assert!(down.wait().success(), "{}", down.text());
	assert!(
		down.text().contains("waiting for project lock"),
		"down did not wait: {}",
		down.text()
	);
	// The network removal is down's; the start of the container is up's last
	// step. Down has to come after it.
	let seen = order(&fake, &["/start HTTP", "DELETE /v5.0.0/libpod/networks/"]);
	let last_start = seen
		.iter()
		.rposition(|&i| i == 0)
		.expect("up never started");
	let first_delete = seen
		.iter()
		.position(|&i| i == 1)
		.expect("down removed nothing");
	assert!(last_start < first_delete, "down ran inside up: {seen:?}");
}
