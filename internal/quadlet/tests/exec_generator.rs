//! Run Podman's real Quadlet generator (`-dryrun`, no service is created or
//! started) over the exported unit and check the `ExecStart=` it builds. The
//! tests return early where the generator is not installed.

#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

use crate::parse_str;
use crate::quadlet::{generate_at, validate_for_quadlet};

fn generator() -> Option<PathBuf> {
	["/usr/libexec/podman/quadlet", "/usr/lib/podman/quadlet"]
		.iter()
		.map(PathBuf::from)
		.find(|p| p.is_file())
}

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
	// `%N` and `%n` are the specifiers Quadlet itself writes into the line.
	let rest = rest.replace("%N", "").replace("%n", "");
	assert!(!rest.contains('%') && !rest.contains('$'), "{line}");
}

fn shell_form_yaml(command: &str) -> String {
	format!(
		"services:\n  web:\n    image: docker.io/library/nginx\n    command: '{}'\n",
		command.replace('\'', "''")
	)
}

/// Shell-form commands whose words never include a bare `;` once Quadlet has
/// read them: a `;` inside a word, escaped, or in text that never closes.
const KEPT: &[&str] = &[
	r#"a;b"#,
	r#"a ;b"#,
	r#""a;b""#,
	r#"a\;b"#,
	r#"'a;b'"#,
	r#"a"b;c"d"#,
	r#"\\;"#,
	r#"\x3b"#,
	r#"\073"#,
	r#"x; y"#,
	r#"a ';'x"#,
	r#""a;b"#,
	r#"a "b ; c"#,
];

/// Shell-form commands in which Quadlet hands systemd a bare `;` word.
const REFUSED: &[&str] = &[
	r#"a ; b"#,
	r#";"#,
	r#"";""#,
	r#"';'"#,
	r#"a ";" b"#,
	r#";"""#,
];

#[test]
fn the_semicolon_check_agrees_with_what_quadlet_emits() {
	let Some(generator) = generator() else {
		return;
	};
	let bare = |yaml: &str| {
		exec_start(&generator, yaml).is_some_and(|l| l.split_whitespace().any(|w| w == ";"))
	};
	for command in KEPT {
		let yaml = shell_form_yaml(command);
		let file = parse_str(&yaml).unwrap();
		assert!(
			validate_for_quadlet(&file).is_ok(),
			"{command:?} was refused"
		);
		assert!(!bare(&yaml), "{command:?} reaches systemd with a bare ;");
	}
	for command in REFUSED {
		let yaml = shell_form_yaml(command);
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
