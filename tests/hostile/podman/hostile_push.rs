//! A registry that answers a push with a failure, a cut-off body or an endless
//! line must make `push` fail loudly, never report `Pushed` for that image, and
//! never hang or grow without bound. `--ignore-push-failures` keeps going past
//! one such image and still fails when none got through.

// The fake engine is a Unix domain socket server.
#![cfg(unix)]

use std::fs;
use std::time::{Duration, Instant};

#[path = "../support/fake_engine.rs"]
mod fake_engine;

use fake_engine::{FakeEngine, Reply};

/// What each `bad<N>` image's registry answers; `good` always succeeds. The
/// handler is a plain function, so the case travels in the image name and the
/// tests can run in parallel.
const BAD_BODIES: &[(&str, &str)] = &[
	(
		"denied after progress",
		"{\"stream\":\"x\"}\n{\"error\":\"denied: requested access\"}\n",
	),
	("cut-off json", "{\"stream\":\"x\"}\n{\"stream\":\"do"),
	("error object only", "{\"error\":\"unauthorized\"}\n"),
	("garbage", "not json at all\n"),
	("healthy", "{\"stream\":\"ok\"}\n"),
];

/// Index of the case that sends one 64 MiB line with no newline.
const ENDLESS: usize = 99;

/// The progress a healthy registry streams back.
const PROGRESS: &str = "{\"stream\":\"Copying blob\"}\n";

fn compose(case: usize) -> String {
	format!("services:\n  a:\n    image: good\n  b:\n    image: bad{case}\n")
}

fn engine() -> FakeEngine {
	FakeEngine::start(|line, _| {
		if !line.starts_with("POST") || !line.contains("/push") {
			return None;
		}
		let body = match line.split("/images/bad").nth(1) {
			Some(rest) => {
				let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
				match digits.parse::<usize>() {
					Ok(ENDLESS) => "a".repeat(64 * 1024 * 1024),
					Ok(n) => BAD_BODIES[n].1.to_string(),
					Err(_) => String::new(),
				}
			}
			None => format!("{PROGRESS}{{\"stream\":\"Pushed\"}}\n"),
		};
		Some(Reply { status: 200, body })
	})
}

fn push(fake: &FakeEngine, case: usize, extra: &[&str]) -> (Option<i32>, String) {
	fs::write(fake.dir().join("compose.yaml"), compose(case)).unwrap();
	let mut args = vec!["-f", "compose.yaml", "-p", "leak", "push"];
	args.extend_from_slice(extra);
	fake.run_with_status(None, &[], &args)
}

const HEALTHY: usize = 4;

/// Control: every image pushes and the exit code is 0.
#[test]
fn a_healthy_registry_pushes_every_image() {
	let (code, text) = push(&engine(), HEALTHY, &[]);
	assert_eq!(code, Some(0), "{text}");
	assert!(
		text.contains("Image bad4  Pushed"),
		"control printed no Pushed line:\n{text}"
	);
}

#[test]
fn a_failing_push_fails_the_command_and_is_never_reported_pushed() {
	for (case, (name, _)) in BAD_BODIES.iter().enumerate().take(HEALTHY) {
		let (code, text) = push(&engine(), case, &[]);
		assert_ne!(code, Some(0), "{name}: the command must fail\n{text}");
		assert!(
			!text.contains(&format!("Image bad{case}  Pushed")),
			"{name}: the failed image was reported pushed\n{text}"
		);
	}
}

#[test]
fn ignoring_failures_continues_past_one_and_still_fails_when_all_fail() {
	for (case, (name, _)) in BAD_BODIES.iter().enumerate().take(HEALTHY) {
		let (code, text) = push(&engine(), case, &["--ignore-push-failures"]);
		assert_eq!(code, Some(0), "{name}: one survivor is a success\n{text}");
		assert!(
			text.contains("Image good  Pushed"),
			"{name}: the survivor was not pushed\n{text}"
		);
		assert!(
			!text.contains(&format!("Image bad{case}  Pushed")),
			"{name}: the failed image was reported pushed\n{text}"
		);
	}
	// Both images fail: nothing reached the registry, which is not a success.
	let fake = FakeEngine::start(|line, _| {
		(line.starts_with("POST") && line.contains("/push")).then(|| Reply {
			status: 200,
			body: "{\"error\":\"denied\"}\n".to_string(),
		})
	});
	let (code, text) = push(&fake, 0, &["--ignore-push-failures"]);
	assert_ne!(code, Some(0), "every push failed\n{text}");
	assert!(!text.contains("Pushed"), "{text}");
}

#[test]
fn quiet_hides_progress_but_not_the_failure() {
	let (code, text) = push(&engine(), 0, &["--quiet"]);
	assert_ne!(code, Some(0), "{text}");
	assert!(
		text.contains("denied"),
		"the reason must survive --quiet:\n{text}"
	);
}

/// A single line with no newline far past any sane progress record: the call
/// ends, fails, and does so in bounded time instead of buffering forever.
#[test]
fn an_endless_line_ends_in_bounded_time() {
	let started = Instant::now();
	let (code, text) = push(&engine(), ENDLESS, &[]);
	assert!(
		started.elapsed() < Duration::from_secs(60),
		"took {:?}",
		started.elapsed()
	);
	assert_ne!(
		code,
		Some(0),
		"a 64 MiB line is no progress record\n{}",
		&text[..text.len().min(300)]
	);
}
