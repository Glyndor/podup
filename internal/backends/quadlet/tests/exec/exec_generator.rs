//! Run Podman's real Quadlet generator (`-dryrun`, no service is created or
//! started) over the exported unit and check the `ExecStart=` it builds. The
//! tests return early, with a `SKIP:` line, where the generator is not installed
//! or is older than the floor (see `generator.rs`).

#![cfg(unix)]

use std::process::Command;

use crate::backends::quadlet::tests::generator::generator;
use crate::parse_str;
use crate::quadlet::{generate_at, validate_for_quadlet};

/// The `ExecStart=` line Quadlet builds for the `proj-web` container, if it
/// builds one (it drops a command it cannot read).
fn exec_start(generator: &std::path::Path, yaml: &str) -> Option<String> {
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "proj", std::path::Path::new("/srv/app"));
	let dir = tempfile::tempdir().unwrap();
	crate::quadlet::write_units(dir.path(), &out.units).unwrap();
	let run = Command::new(generator)
		.args(["-dryrun", "-user"])
		.env("QUADLET_UNIT_DIRS", dir.path())
		.output()
		.unwrap();
	let text = String::from_utf8_lossy(&run.stdout).into_owned();
	text.lines()
		.find(|l| l.starts_with("ExecStart="))
		.map(str::to_string)
}

#[test]
fn quadlet_hands_systemd_a_command_line_with_no_specifier_or_variable() {
	let Some(generator) = generator() else {
		return;
	};
	let line = exec_start(
		&generator,
		"services:\n  web:\n    image: docker.io/library/nginx\n    command: [date, \"+%H:%M\", \"$$HOME\", \"a b\"]\n    environment:\n      K: \"$$HOME %x\"\n",
	)
	.expect("no ExecStart line");
	assert!(
		line.contains("+%%H:%%M") && line.contains("$$HOME"),
		"{line}"
	);
	let rest = line.replace("%%", "").replace("$$", "");
	// `%t`, `%N` and `%n` are the specifiers Quadlet itself writes into the line.
	let rest = rest.replace("%t", "").replace("%N", "").replace("%n", "");
	assert!(!rest.contains('%') && !rest.contains('$'), "{line}");
}

fn words_yaml(words: &[&str]) -> String {
	format!(
		"services:\n  web:\n    image: docker.io/library/nginx\n    command: {}\n",
		serde_json::to_string(words).unwrap()
	)
}

/// Commands whose words never include a bare `;` once Quadlet has read them: a
/// `;` inside a word, escaped, or inside quotes the word itself carries. A
/// string `command:` cannot hold a bare `;` at all, since Compose splits it
/// there, so these are lists.
const KEPT: &[&[&str]] = &[
	&["a;b"],
	&["a ;b"],
	&["\"a;b\""],
	&["a\\;b"],
	&["'a;b'"],
	&["a\"b;c\"d"],
	&["\\\\;"],
	&["\\x3b"],
	&["\\073"],
	&["x;", "y"],
	&["a ';'x"],
];

/// Commands in which Quadlet hands systemd a bare `;` word.
const REFUSED: &[&[&str]] = &[&["a", ";", "b"], &[";"], &["find", "{}", ";"]];

#[test]
fn the_semicolon_check_agrees_with_what_quadlet_emits() {
	let Some(generator) = generator() else {
		return;
	};
	let bare = |yaml: &str| {
		exec_start(&generator, yaml).is_some_and(|l| l.split_whitespace().any(|w| w == ";"))
	};
	for command in KEPT {
		let yaml = words_yaml(command);
		let file = parse_str(&yaml).unwrap();
		assert!(
			validate_for_quadlet(&file).is_ok(),
			"{command:?} was refused"
		);
		assert!(!bare(&yaml), "{command:?} reaches systemd with a bare ;");
	}
	for command in REFUSED {
		let yaml = words_yaml(command);
		let file = parse_str(&yaml).unwrap();
		assert!(
			validate_for_quadlet(&file).is_err(),
			"{command:?} was accepted"
		);
		assert!(
			bare(&yaml),
			"{command:?}: Quadlet keeps no bare ; to refuse"
		);
	}
}

#[test]
fn quadlet_puts_the_runtime_before_run_and_keeps_platform_and_gpus() {
	let Some(generator) = generator() else {
		return;
	};
	let line = exec_start(
		&generator,
		"services:\n  web:\n    image: docker.io/library/nginx\n    runtime: runc\n    platform: linux/arm64/v8\n    gpus: all\n",
	)
	.expect("no ExecStart line");
	let (before_run, after_run) = line.split_once(" run ").expect("no run subcommand");
	assert!(before_run.ends_with("--runtime=runc"), "{line}");
	assert!(after_run.contains("--platform=linux/arm64/v8"), "{line}");
	assert!(after_run.contains("--device nvidia.com/gpu=all"), "{line}");
}
