//! Error and alternate interpolation words must not disclose environment values.

use super::file_process::{command, run};
use super::{assert_refused, compose_dir, write};

const SECRET: &str = "PODUP_HOSTILE_SECRET";
const MISSING: &str = "PODUP_HOSTILE_MISSING";
const PRESENT: &str = "PODUP_HOSTILE_PRESENT";
const EMPTY: &str = "PODUP_HOSTILE_EMPTY";
const MARKERS: [&str; 3] = ["SECRET-HEAD-39b1", "SECRET-MIDDLE-93ac", "SECRET-TAIL-b742"];

fn diagnostic(template: &str, secret: &str) {
	let dir = compose_dir();
	write(
		dir.path(),
		"compose.yaml",
		&format!("services:\n  web:\n    image: alpine\n    command: ['{template}']\n"),
	);
	let mut cmd = command(dir.path(), &["config"]);
	// Child-only variables: parallel tests never mutate the runner's environment.
	cmd.env_clear()
		.env("PATH", "/usr/bin:/bin")
		.env("HOME", dir.path())
		.env("NO_COLOR", "1")
		.env(PRESENT, "yes")
		.env(EMPTY, "")
		.env_remove(MISSING);
	if secret.contains('\0') {
		// execve cannot carry NUL; the dotenv loader can, as its byte tests pin.
		write(dir.path(), ".env", &format!("{SECRET}='{secret}'\n"));
	} else {
		cmd.env(SECRET, secret);
	}
	let outcome = run(cmd);
	assert_refused(template, &outcome);
	assert!(!outcome.timed_out, "interpolation exceeded the deadline");
	assert!(
		!outcome.text.is_empty() && outcome.text.len() < 4096,
		"diagnostic has {} bytes",
		outcome.text.len()
	);
	// Separate canaries catch clipped and escaped disclosures as well as full ones.
	for marker in MARKERS {
		assert!(!outcome.text.contains(marker), "environment canary leaked");
	}
	assert!(!outcome.text.contains(secret), "environment value leaked");
	for control in ['\0', '\r', '\u{1b}', '\u{7}'] {
		assert!(
			!outcome.text.contains(control),
			"raw terminal control leaked"
		);
	}
}

#[test]
fn hostile_environment_values_stay_out_of_required_variable_diagnostics() {
	// The message is the author's own text and may name any variable on purpose, so
	// only templates where the secret can reach the error by accident are attacked.
	let templates = [
		"${PODUP_HOSTILE_MISSING:?required}",
		"${PODUP_HOSTILE_MISSING?required}",
		"${PODUP_HOSTILE_EMPTY:?required}",
		"${PODUP_HOSTILE_SECRET:+${PODUP_HOSTILE_MISSING:?required}}",
		"${PODUP_HOSTILE_PRESENT:+${PODUP_HOSTILE_MISSING?required}}",
	];
	for separator in ["\n", "\r\n", "\t\u{7}", "\u{1b}[2J", "\0"] {
		let secret = format!(
			"{}{separator}{}{separator}{}{separator}{}",
			MARKERS[0],
			"private-data".repeat(1024),
			MARKERS[1],
			MARKERS[2]
		);
		for template in templates {
			diagnostic(template, &secret);
		}
	}
}
