//! systemd reads `%` and `$` in command lines and splits commands on a lone
//! `;`. These tests pin how a literal one of each reaches the unit.

use crate::parse_str;
use crate::quadlet::tests::unit_named;
use crate::quadlet::{generate_at, validate_for_quadlet};

fn container(yaml: &str) -> String {
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "proj", std::path::Path::new("/srv/app"));
	unit_named(&out, "proj-web.container").contents.clone()
}

fn line<'a>(unit: &'a str, key: &str) -> &'a str {
	unit.lines()
		.find(|l| l.starts_with(&format!("{key}=")))
		.unwrap_or_else(|| panic!("no {key}= in:\n{unit}"))
}

#[test]
fn a_percent_in_a_command_is_written_doubled() {
	let unit = container(
		"services:\n  web:\n    image: nginx\n    command: [date, \"+%H:%M\"]\n    entrypoint: [\"sh\", \"-c\", \"echo %h\"]\n",
	);
	assert!(line(&unit, "Exec").contains("+%%H:%%M"), "{unit}");
	assert!(line(&unit, "Entrypoint").contains("echo %%h"), "{unit}");
}

#[test]
fn a_percent_in_a_shell_form_command_is_written_doubled() {
	let unit = container("services:\n  web:\n    image: nginx\n    command: echo %h\n");
	assert_eq!(line(&unit, "Exec"), "Exec=echo %%h");
}

#[test]
fn a_dollar_is_doubled_in_every_container_value_that_reaches_a_command_line() {
	let unit = container(
		"services:\n  web:\n    image: nginx\n    command: [sh, -c, \"echo $$HOME\"]\n    environment:\n      K: \"$$HOME\"\n    labels:\n      l: \"$$HOME\"\n    volumes: [\"/srv/$$HOME:/d\"]\n",
	);
	for key in ["Exec", "Environment", "Volume"] {
		assert!(line(&unit, key).contains("$$HOME"), "{key}: {unit}");
	}
	let stray: Vec<&str> = unit
		.lines()
		.filter(|l| l.replace("$$", "").contains('$'))
		.collect();
	assert!(stray.is_empty(), "lone dollar left in: {stray:?}");
}

#[test]
fn a_dollar_is_left_alone_in_unit_and_service_sections() {
	let unit = container("services:\n  web:\n    image: nginx\n    restart: always\n");
	assert!(!unit.contains("$$"), "{unit}");
}

fn rejects(yaml: &str) -> bool {
	let file = parse_str(yaml).unwrap();
	validate_for_quadlet(&file).is_err()
}

#[test]
fn a_lone_semicolon_argument_is_refused() {
	for command in [
		"command: [find, /, -exec, ls, \"{}\", \";\"]",
		"entrypoint: [\";\"]",
		"command: find . -exec ls {} ;",
		"command: \"find . -exec ls {} ';'\"",
		"command: \"ls \\\";\\\"\"",
	] {
		let yaml = format!("services:\n  web:\n    image: nginx\n    {command}\n");
		assert!(rejects(&yaml), "{command} should be refused");
	}
}

#[test]
fn a_semicolon_inside_a_word_is_kept() {
	for command in [
		"command: [sh, -c, \"a;b\"]",
		"command: [echo, \"x;\", \";y\"]",
		"command: \"sh -c 'a ; b'\"",
	] {
		let yaml = format!("services:\n  web:\n    image: nginx\n    {command}\n");
		assert!(!rejects(&yaml), "{command} should be accepted");
	}
}
