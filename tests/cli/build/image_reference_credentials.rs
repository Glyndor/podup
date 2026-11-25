//! An image reference written with credentials (`user:secret@registry/img`) is
//! refused by Podman, which repeats it in its message. The reference is also
//! the name of a row on the board. Neither shows the credentials, at any log
//! level, for a pull, a push and `up`. The engine is a fake Unix socket.

#![cfg(unix)]

use std::fs;

#[path = "../../hostile/support/fake_engine.rs"]
mod fake_engine;

use fake_engine::{FakeEngine, Reply};

// Pieces joined by characters that are valid in a password, each looked for on
// its own.
const PASSWORD: &str = "pwA1xx,pwB2xx(pwC3xx)pwD4xx'pwE5xx";
const USER: &str = "usrZ9zz";

fn reference() -> String {
	format!("{USER}:{PASSWORD}@127.0.0.1:1/team/img:tag")
}

fn compose() -> String {
	format!("services:\n  app:\n    image: \"{}\"\n", reference())
}

fn leaks(output: &str) -> usize {
	let mut count = 0;
	for secret in [PASSWORD, USER] {
		count += usize::from(output.contains(secret));
		count += secret
			.split([',', '(', ')', '\''])
			.filter(|piece| output.contains(piece))
			.count();
	}
	count
}

/// Podman does not take the reference and says so the way it does, with the
/// reference it was given.
fn refuses_the_reference(line: &str, _: &str) -> Option<Reply> {
	if line.starts_with("GET") && line.contains("/images/") && line.contains("/json") {
		return Some(Reply {
			status: 404,
			body: r#"{"message":"no such image"}"#.to_string(),
		});
	}
	if line.starts_with("POST") && (line.contains("/images/pull") || line.contains("/push")) {
		return Some(Reply {
			status: 500,
			body: format!(
				r#"{{"cause":"invalid reference format","message":"parsing reference \"{}\": invalid reference format: unknown transport \"{USER}\"","response":500}}"#,
				reference()
			),
		});
	}
	None
}

fn run(level: Option<&str>, command: &[&str]) -> String {
	let engine = FakeEngine::start(refuses_the_reference);
	fs::write(engine.dir().join("compose.yaml"), compose()).unwrap();
	let mut args = vec!["-f", "compose.yaml", "-p", "proj"];
	args.extend_from_slice(command);
	let (_, output) = engine.run_with_status(level, &[], &args);
	output
}

#[test]
fn a_pull_a_push_and_an_up_do_not_repeat_the_credentials_of_a_reference() {
	// The commands, and whether the output still names the image.
	let commands: [(&[&str], bool); 6] = [
		(&["pull", "--policy", "always"], true),
		(&["pull", "--ignore-pull-failures"], true),
		(&["push"], false),
		(&["push", "--quiet"], false),
		(&["push", "--ignore-push-failures"], true),
		(&["up", "-d"], true),
	];
	// No level at all is the one that draws the progress rows.
	for level in [
		None,
		Some("error"),
		Some("info"),
		Some("debug"),
		Some("trace"),
	] {
		for (command, names_the_image) in commands {
			let output = run(level, command);
			// The messages count the pieces and never show them: the output
			// would hold the credentials.
			assert!(
				leaks(&output) == 0,
				"{level:?} {command:?} showed {} piece(s) of a credential",
				leaks(&output)
			);
			// What names the image stays, so the line is still useful.
			assert!(
				!names_the_image || output.contains("127.0.0.1:1/team/img:tag"),
				"{level:?} {command:?} lost the reference without its credentials"
			);
		}
	}
}

#[test]
fn the_row_of_a_pull_and_of_a_push_names_the_image_without_its_credentials() {
	for command in [&["pull", "--policy", "always"][..], &["push"][..]] {
		let output = run(None, command);
		assert!(
			output.contains(" Image 127.0.0.1:1/team/img:tag"),
			"{command:?} drew no row for the image"
		);
	}
}
