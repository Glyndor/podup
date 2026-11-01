//! Hostile bytes and duplicate assignments through each real config loader.
#![cfg(unix)]

use file_process::{command, run, CAP};
use std::fs;

#[path = "../support/file_process.rs"]
mod file_process;

#[derive(Clone, Copy, Debug)]
enum Loader {
	DotEnv,
	Flag,
	Service,
}

#[derive(Clone, Copy, Debug)]
enum Payload {
	Nul,
	CarriageReturn,
	InvalidUtf8,
	Duplicate,
	OverLimit,
}

fn bytes(payload: Payload) -> Vec<u8> {
	match payload {
		Payload::Nul => b"K=a\0b\n".to_vec(),
		Payload::CarriageReturn => b"K=a\rb\n".to_vec(),
		Payload::InvalidUtf8 => b"K=\xff\n".to_vec(),
		Payload::Duplicate => b"K=first\nK=last\n".to_vec(),
		Payload::OverLimit => {
			let mut value = b"K=".to_vec();
			value.resize(CAP + 1, b'x');
			value.push(b'\n');
			value
		}
	}
}

fn run_case(loader: Loader, payload: Payload) -> file_process::Outcome {
	let dir = tempfile::tempdir().unwrap();
	let root = dir.path();
	fs::write(root.join("compose.yaml"), compose(loader)).unwrap();
	let name = match loader {
		Loader::DotEnv => ".env",
		Loader::Flag => "flag.env",
		Loader::Service => "service.env",
	};
	fs::write(root.join(name), bytes(payload)).unwrap();
	let args: Vec<String> = match loader {
		Loader::DotEnv | Loader::Service => vec!["config".into(), "--format".into(), "json".into()],
		Loader::Flag => vec![
			"--env-file".into(),
			name.into(),
			"config".into(),
			"--format".into(),
			"json".into(),
		],
	};
	let refs: Vec<&str> = args.iter().map(String::as_str).collect();
	let mut cmd = command(root, &refs);
	cmd.env_remove("K");
	let outcome = run(cmd);
	assert!(
		!outcome.timed_out,
		"{loader:?}/{payload:?} hung: {}",
		outcome.text
	);
	assert!(
		outcome.text.len() < 4096,
		"unbounded diagnostic: {} bytes",
		outcome.text.len()
	);
	if !outcome.success {
		for fragment in ["K=a", "K=first", "K=last", "a\rb", "�"] {
			assert!(
				!outcome.text.contains(fragment),
				"diagnostic echoed {fragment:?}"
			);
		}
	}
	outcome
}

fn compose(loader: Loader) -> &'static str {
	match loader {
		Loader::DotEnv | Loader::Flag => {
			"services:\n  web:\n    image: alpine\n    environment:\n      K: ${K}\n"
		}
		Loader::Service => "services:\n  web:\n    image: alpine\n    env_file: [service.env]\n",
	}
}

fn environment(outcome: &file_process::Outcome) -> serde_json::Value {
	let json = outcome
		.text
		.split("\npodup:")
		.next()
		.unwrap_or(&outcome.text);
	serde_json::from_str(json).unwrap_or_else(|e| {
		panic!(
			"expected JSON config; parse error {e}; output: {}",
			outcome.text
		)
	})
}

fn assert_rejected(outcome: file_process::Outcome, what: &str) {
	assert!(!outcome.success, "{what} was accepted: {}", outcome.text);
	assert!(!outcome.text.is_empty(), "{what} had no bounded diagnostic");
}

#[test]
fn hostile_bytes_follow_each_loader_policy_through_config() {
	for payload in [
		Payload::Nul,
		Payload::CarriageReturn,
		Payload::InvalidUtf8,
		Payload::Duplicate,
		Payload::OverLimit,
	] {
		for loader in [Loader::DotEnv, Loader::Flag, Loader::Service] {
			let outcome = run_case(loader, payload);
			eprintln!(
				"{loader:?}/{payload:?}: success={} bytes={}",
				outcome.success,
				outcome.text.len()
			);
			match (loader, payload) {
				(Loader::DotEnv, Payload::Duplicate) => {
					assert!(outcome.success, "{}", outcome.text);
					let model = environment(&outcome);
					assert_eq!(model["services"]["web"]["environment"]["K"], "last");
				}
				(Loader::DotEnv, Payload::CarriageReturn) => {
					assert!(outcome.success, "{}", outcome.text);
					let model = environment(&outcome);
					assert_eq!(model["services"]["web"]["environment"]["K"], "a\rb");
				}
				(Loader::DotEnv, Payload::Nul) => {
					assert!(outcome.success, "{}", outcome.text);
					let model = environment(&outcome);
					assert_eq!(model["services"]["web"]["environment"]["K"], "a\u{0}b");
				}
				(Loader::DotEnv, Payload::InvalidUtf8 | Payload::OverLimit) => {
					assert!(
						outcome.success,
						"optional .env should be ignored: {}",
						outcome.text
					);
					let model = environment(&outcome);
					assert_eq!(model["services"]["web"]["environment"]["K"], "");
				}
				(Loader::Flag, Payload::Duplicate) => {
					assert!(outcome.success, "{}", outcome.text);
					let model = environment(&outcome);
					assert_eq!(model["services"]["web"]["environment"]["K"], "last");
				}
				(Loader::Flag, Payload::CarriageReturn) => {
					assert!(outcome.success, "{}", outcome.text);
					let model = environment(&outcome);
					assert_eq!(model["services"]["web"]["environment"]["K"], "a\rb");
				}
				(Loader::Flag, Payload::Nul | Payload::InvalidUtf8 | Payload::OverLimit) => {
					assert_rejected(outcome, "explicit --env-file payload");
				}
				(Loader::Service, Payload::Duplicate) => {
					assert!(outcome.success, "{}", outcome.text);
					let model = environment(&outcome);
					assert_eq!(model["services"]["web"]["environment"]["K"], "last");
				}
				(Loader::Service, Payload::CarriageReturn) => {
					assert!(outcome.success, "{}", outcome.text);
					let model = environment(&outcome);
					assert_eq!(model["services"]["web"]["environment"]["K"], "a\rb");
				}
				(Loader::Service, Payload::Nul) => {
					assert!(outcome.success, "{}", outcome.text);
					let model = environment(&outcome);
					assert_eq!(model["services"]["web"]["environment"]["K"], "a\u{0}b");
				}
				(Loader::Service, Payload::InvalidUtf8 | Payload::OverLimit) => {
					assert_rejected(outcome, "service env_file payload");
				}
			}
		}
	}
}
