use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use super::*;
use crate::engine::fake_podman::{self, FakePodman};
use crate::engine::Engine;

fn kills(fake: &FakePodman) -> Vec<String> {
	fake.requests
		.lock()
		.unwrap()
		.iter()
		.filter(|r| r.contains("/exec/abc/kill"))
		.cloned()
		.collect()
}

/// A fake that answers the kill route with `answers[n]` for the n-th request
/// (the last one repeated) and anything else with 404.
fn fake_kill(answers: Vec<u16>) -> (FakePodman, Arc<AtomicUsize>) {
	let seen = Arc::new(AtomicUsize::new(0));
	let counter = seen.clone();
	let fake = fake_podman::start(move |method, target| {
		if method == "POST" && target.contains("/exec/abc/kill") {
			let n = counter.fetch_add(1, Ordering::SeqCst);
			let status = answers[n.min(answers.len() - 1)];
			return (status, String::new());
		}
		(404, r#"{"message":"not found"}"#.to_string())
	});
	(fake, seen)
}

/// A session that ends once the fake has seen `wanted` kill requests, or after
/// two seconds of paused time.
async fn ends_after(seen: Arc<AtomicUsize>, wanted: usize) -> crate::error::Result<()> {
	for _ in 0..200 {
		if seen.load(Ordering::SeqCst) >= wanted {
			return Ok(());
		}
		tokio::time::sleep(Duration::from_millis(10)).await;
	}
	Ok(())
}

fn engine(fake: &FakePodman) -> Engine {
	Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir())
}

#[tokio::test(start_paused = true)]
async fn each_signal_is_sent_to_the_exec_by_name() {
	for (which, name) in [
		(Relayed::Term, "SIGTERM"),
		(Relayed::Int, "SIGINT"),
		(Relayed::Hup, "SIGHUP"),
	] {
		let (fake, seen) = fake_kill(vec![200]);
		let (tx, mut rx) = mpsc::unbounded_channel();
		tx.send(which).unwrap();
		engine(&fake)
			.relay_signals("abc", &mut rx, ends_after(seen, 1))
			.await
			.unwrap();
		let sent = kills(&fake);
		assert_eq!(sent.len(), 1, "{sent:?}");
		assert!(sent[0].contains(&format!("kill?signal={name}")), "{sent:?}");
	}
}

#[tokio::test(start_paused = true)]
async fn nothing_is_sent_when_no_signal_arrives() {
	let (fake, seen) = fake_kill(vec![200]);
	let (_tx, mut rx) = mpsc::unbounded_channel::<Relayed>();
	engine(&fake)
		.relay_signals("abc", &mut rx, async {
			tokio::time::sleep(Duration::from_millis(500)).await;
			Ok(())
		})
		.await
		.unwrap();
	assert_eq!(seen.load(Ordering::SeqCst), 0);
}

/// A signal that comes before the start request has been served finds an exec
/// that is not running: the daemon says 409, and the signal is tried again.
#[tokio::test(start_paused = true)]
async fn a_signal_that_comes_before_the_command_runs_is_tried_again() {
	let (fake, seen) = fake_kill(vec![409, 409, 200]);
	let (tx, mut rx) = mpsc::unbounded_channel();
	tx.send(Relayed::Term).unwrap();
	engine(&fake)
		.relay_signals("abc", &mut rx, ends_after(seen.clone(), 3))
		.await
		.unwrap();
	assert_eq!(kills(&fake).len(), 3);
}

#[tokio::test(start_paused = true)]
async fn a_failure_that_is_not_about_timing_is_not_repeated() {
	let (fake, seen) = fake_kill(vec![500]);
	let (tx, mut rx) = mpsc::unbounded_channel();
	tx.send(Relayed::Term).unwrap();
	engine(&fake)
		.relay_signals("abc", &mut rx, async {
			tokio::time::sleep(Duration::from_secs(2)).await;
			Ok(())
		})
		.await
		.unwrap();
	assert_eq!(seen.load(Ordering::SeqCst), 1);
}

/// The relay does not end the session itself: what the session returns, its
/// error included, is what the caller gets.
#[tokio::test(start_paused = true)]
async fn the_result_is_the_sessions() {
	let (fake, _) = fake_kill(vec![200]);
	let (_tx, mut rx) = mpsc::unbounded_channel::<Relayed>();
	let error = engine(&fake)
		.relay_signals("abc", &mut rx, async {
			Err(crate::error::ComposeError::RunExited(42))
		})
		.await
		.unwrap_err();
	assert!(matches!(error, crate::error::ComposeError::RunExited(42)));
}

#[test]
fn the_names_are_the_ones_the_route_takes() {
	assert_eq!(Relayed::Term.name(), "SIGTERM");
	assert_eq!(Relayed::Int.name(), "SIGINT");
	assert_eq!(Relayed::Hup.name(), "SIGHUP");
}
