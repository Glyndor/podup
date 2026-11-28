//! The values of `environment:` reach a container through a private env file,
//! never through the lines of a unit that Quadlet puts on the command line.
//! The values here stand for passwords, so no failure message holds one.

use std::path::Path;

use super::{generate_at, generate_for_autostart, QuadletOutput, QuadletUnit};
use crate::compose::parse_str;

const MARKER: &str = "marker-value-4711";

fn compose(environment: &str) -> String {
	format!(
		"services:\n  web:\n    image: img\n    env_file: [base.env]\n    environment:\n{environment}"
	)
}

fn both_shapes(yaml: &str) -> [QuadletOutput; 2] {
	let file = parse_str(yaml).unwrap();
	let base = Path::new("/srv/app");
	[
		generate_at(&file, "proj", base),
		generate_for_autostart(&file, "proj", base),
	]
}

fn unit<'a>(out: &'a QuadletOutput, name: &str) -> &'a QuadletUnit {
	out.units
		.iter()
		.find(|u| u.filename == name)
		.unwrap_or_else(|| panic!("no file {name}"))
}

#[test]
fn a_value_is_in_the_env_file_and_in_no_unit() {
	for out in both_shapes(&compose(&format!(
		"      DB_PASSWORD: {MARKER}\n      EMPTY: ''\n"
	))) {
		for other in out
			.units
			.iter()
			.filter(|u| u.filename != "proj-web.podup.env")
		{
			assert!(
				!other.contents.contains(MARKER),
				"{} holds the value",
				other.filename
			);
		}
		let env = unit(&out, "proj-web.podup.env");
		assert!(
			env.contents == format!("# podup-owner: proj\nDB_PASSWORD={MARKER}\nEMPTY=\n"),
			"the env file does not hold the owner line and the two entries"
		);
		assert!(out.warnings.iter().all(|w| !w.contains(MARKER)));
	}
}

/// The last file that names a variable wins, so the file of `environment:`
/// comes after the files of `env_file:`, and it is written before the unit
/// that names it.
#[test]
fn the_unit_names_the_file_after_the_services_own_env_files() {
	for out in both_shapes(&compose("      A: one\n")) {
		let container = &unit(&out, "proj-web.container").contents;
		let named: Vec<&str> = container
			.lines()
			.filter_map(|l| l.strip_prefix("EnvironmentFile="))
			.collect();
		// The service's own file is made absolute with the separator of the
		// host, and a Windows path is written quoted.
		assert_eq!(named.len(), 2, "{named:?}");
		assert!(named[0].contains("base.env"), "{named:?}");
		assert_eq!(named[1], "proj-web.podup.env");
		let position = |name: &str| out.units.iter().position(|u| u.filename == name);
		assert!(position("proj-web.podup.env") < position("proj-web.container"));
	}
}

#[test]
fn a_value_the_file_cannot_carry_stays_in_the_unit_and_is_reported_by_name() {
	let yaml = compose(&format!(
		"      PEM: \"{MARKER}\\nsecond\"\n      '#odd': x\n      PLAIN: kept\n"
	));
	for out in both_shapes(&yaml) {
		let container = &unit(&out, "proj-web.container").contents;
		assert!(container.contains("Environment=") && container.contains("PEM="));
		assert!(!container.contains("PLAIN="));
		let env = &unit(&out, "proj-web.podup.env").contents;
		assert!(env.contains("PLAIN=kept\n") && !env.contains("PEM") && !env.contains("odd"));
		for key in ["PEM", "#odd"] {
			assert!(
				out.warnings
					.iter()
					.any(|w| w.contains(key) && w.contains("command line")),
				"no warning names {key}"
			);
		}
		assert!(out.warnings.iter().all(|w| !w.contains(MARKER)));
	}
}

#[test]
fn a_service_without_values_has_no_env_file() {
	for environment in ["      BARE:\n", "      {}\n"] {
		for out in both_shapes(&compose(environment)) {
			assert!(out
				.units
				.iter()
				.all(|u| !u.filename.ends_with(".podup.env")));
			assert!(!unit(&out, "proj-web.container")
				.contents
				.contains("podup.env"));
		}
	}
}

#[test]
fn a_scheduled_job_keeps_its_values_in_its_own_env_file() {
	let yaml = format!(
		"services:\n  web:\n    image: img\njobs:\n  backup:\n    image: img\n    environment:\n      TOKEN: {MARKER}\n    triggers:\n      schedule: ['0 3 * * *']\n"
	);
	let out = generate_at(&parse_str(&yaml).unwrap(), "proj", Path::new("/srv/app"));
	let container = &unit(&out, "proj-backup.container").contents;
	assert!(container.contains("EnvironmentFile=proj-backup.podup.env\n"));
	assert!(!container.contains(MARKER), "the unit holds the value");
	assert!(unit(&out, "proj-backup.podup.env")
		.contents
		.contains(MARKER));
}

/// The bounds of what `podman --env-file` reads back as written (measured on
/// Podman 6.2.1).
#[test]
fn only_what_the_file_gives_back_as_written_goes_into_it() {
	let long = "x".repeat(65_535 - "K=".len());
	let over = format!("{long}x");
	for (case, key, value, carried) in [
		("blank at the ends", "K", " a b ", true),
		("quotes and dollars", "K", "\"a\" $HOME %h # x \\n", true),
		("a tab", "K", "a\tb", true),
		("the longest line", "K", long.as_str(), true),
		("one byte more", "K", over.as_str(), false),
		("a line feed", "K", "a\nb", false),
		("a carriage return", "K", "a\r", false),
		("a nul", "K", "a\0", false),
		("an escape", "K", "a\u{1b}", false),
		("a comment", "#K", "v", false),
		("a leading blank", " K", "v", false),
		("a name with a line feed", "K\nL", "v", false),
		("no name", "", "v", false),
	] {
		assert_eq!(super::unit::fits_env_file(key, value), carried, "{case}");
	}
}
