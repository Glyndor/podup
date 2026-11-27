//! What `up` checks when it creates and mounts a secret, the export checks too,
//! or does not export it: the mode the container sees, the names, the modes
//! that are refused, and the text that is printed for the operator to copy; what
//! a hostile or careless `file:` source can do (a link or a pipe where the file
//! should be, a path that is more than a name, two sources, a mode that changed).

use std::path::Path;

use super::tests::{creates_nothing, secrets_unit, yaml_path};
use crate::compose::parse_str;
use crate::quadlet::{
	generate_at, generate_for_autostart, validate_for_quadlet, validate_for_quadlet_in,
	QuadletOutput,
};

fn container<'a>(out: &'a QuadletOutput, name: &str) -> &'a str {
	&out.units
		.iter()
		.find(|u| u.filename == format!("proj-{name}.container"))
		.unwrap()
		.contents
}

fn generate_in(dir: &Path, yaml: &str) -> QuadletOutput {
	generate_at(&parse_str(yaml).unwrap(), "proj", dir)
}

#[cfg(unix)]
fn file_with_mode(dir: &Path, name: &str, mode: u32) {
	use std::os::unix::fs::PermissionsExt;
	let path = dir.join(name);
	std::fs::write(&path, "x").unwrap();
	std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
}

// --- the mode of a mounted file --------------------------------------------

/// `up` mounts a `file:` secret or config with the permission bits of the host
/// file (measured on Podman 6.2.0: 0600 stays 0600, 0640 stays 0640, 0644 stays
/// 0644); `Secret=` with no `mode=` gives 0444. The export reads the bits when it
/// generates and writes them, so it is never more open than `up`.
#[cfg(unix)]
#[test]
fn a_file_is_mounted_with_the_mode_it_has_on_the_host() {
	let dir = tempfile::tempdir().unwrap();
	file_with_mode(dir.path(), "s600", 0o600);
	file_with_mode(dir.path(), "c640", 0o640);
	file_with_mode(dir.path(), "d644", 0o644);
	let out = generate_in(
		dir.path(),
		"services:\n  web:\n    image: img\n    secrets: [s600]\n    configs: [c640, d644]\nsecrets:\n  s600:\n    file: ./s600\nconfigs:\n  c640:\n    file: ./c640\n  d644:\n    file: ./d644\n",
	);
	let web = container(&out, "web");
	assert!(web.contains("Secret=proj_secret_s600,target=s600,mode=600\n"));
	assert!(web.contains("Secret=proj_config_c640,type=mount,target=/c640,mode=640\n"));
	assert!(web.contains("Secret=proj_config_d644,type=mount,target=/d644,mode=644\n"));
}

#[cfg(unix)]
#[test]
fn a_mode_the_service_gives_wins() {
	let dir = tempfile::tempdir().unwrap();
	file_with_mode(dir.path(), "s", 0o600);
	let out = generate_in(
		dir.path(),
		"services:\n  web:\n    image: img\n    secrets:\n      - source: s\n        mode: 0o444\nsecrets:\n  s:\n    file: ./s\n",
	);
	let line = container(&out, "web")
		.lines()
		.find(|l| l.starts_with("Secret="))
		.unwrap()
		.to_string();
	assert_eq!(line.matches("mode=").count(), 1);
}

/// The units `autostart --mode quadlet` installs mount secrets that `up` made, and
/// give them the same mode `up` does: the bits of the host file, unless the
/// service says otherwise.
#[cfg(unix)]
#[test]
fn the_installed_units_keep_the_mode_of_a_secret_file_and_the_mode_a_service_gives() {
	let dir = tempfile::tempdir().unwrap();
	file_with_mode(dir.path(), "s600", 0o600);
	file_with_mode(dir.path(), "s640", 0o640);
	file_with_mode(dir.path(), "given", 0o600);
	let yaml = "services:\n  web:\n    image: img\n    secrets:\n      - s600\n      - s640\n      - source: given\n        mode: 0o440\nsecrets:\n  s600:\n    file: ./s600\n  s640:\n    file: ./s640\n  given:\n    file: ./given\n";
	let out = generate_for_autostart(&parse_str(yaml).unwrap(), "proj", dir.path());
	let web = container(&out, "web");
	for line in [
		"Secret=proj_secret_s600,target=s600,mode=600\n",
		"Secret=proj_secret_s640,target=s640,mode=640\n",
		"Secret=proj_secret_given,target=given,mode=440\n",
	] {
		assert!(web.contains(line), "{line}: {web}");
	}
}

/// A file that cannot be read cannot be asked for its mode: the narrowest one
/// is used, and the operator is told.
#[test]
fn a_file_that_cannot_be_read_is_mounted_with_the_narrowest_mode_and_said_so() {
	let out = generate_at(
		&parse_str("services:\n  web:\n    image: img\n    secrets: [s]\nsecrets:\n  s:\n    file: ./absent\n").unwrap(),
		"proj",
		Path::new("/nonexistent/project"),
	);
	assert!(container(&out, "web").contains("Secret=proj_secret_s,target=s,mode=400\n"));
	assert!(out
		.warnings
		.iter()
		.any(|w| w.contains("could not be read") && w.contains("0400")));
}

/// A file with an execute bit is a mistake or an attack, and `up` refuses a mode
/// like that; the export does not make a secret of it.
#[cfg(unix)]
#[test]
fn a_file_with_an_execute_or_setuid_bit_is_not_exported() {
	for mode in [0o755, 0o700, 0o4644, 0o2644] {
		let dir = tempfile::tempdir().unwrap();
		file_with_mode(dir.path(), "s", mode);
		let out = generate_in(
			dir.path(),
			"services:\n  web:\n    image: img\n    secrets: [s]\nsecrets:\n  s:\n    file: ./s\n",
		);
		assert!(
			!out.units
				.iter()
				.any(|u| u.filename.ends_with("podup-secrets.service")
					&& u.contents.contains("secret create")),
			"was exported"
		);
		assert!(out.warnings.iter().any(|w| w.contains("not exported")));
		// The container still requires the unit, whose command for the name fails,
		// and is given nothing more open than 0400.
		let web = container(&out, "web");
		assert!(web.contains("Requires=proj-podup-secrets.service"));
		assert!(web.contains("Secret=proj_secret_s,target=s,mode=400\n"));
	}
}

// --- the text printed for the operator -------------------------------------------

fn advice_for(var: &str, name: &str) -> Vec<String> {
	let yaml = format!(
		"services:\n  web:\n    image: img\n    secrets: [{name_key}]\nsecrets:\n  {name_key}:\n    environment: {var}\n",
		name_key = serde_json::to_string(name).unwrap(),
		var = serde_json::to_string(var).unwrap()
	);
	generate_at(&parse_str(&yaml).unwrap(), "proj", Path::new("/srv/app")).warnings
}

#[test]
fn a_plain_variable_gets_a_command() {
	let warnings = advice_for("DB_PASSWORD", "db").join("\n");
	assert!(warnings.contains("printenv DB_PASSWORD | podman secret create"));
}

#[test]
fn a_variable_name_that_is_more_than_a_name_gets_no_command() {
	for var in [
		"A;rm -rf ~",
		"A$(id)",
		"A`id`",
		"A\nB",
		"A\u{1b}[31m",
		"A B",
		"1A",
		"",
		"A|B",
	] {
		let joined = advice_for(var, "db").join("\n");
		assert!(!joined.contains("printenv"));
		assert!(
			!joined.chars().any(|c| c.is_control() && c != '\n'),
			"a control character was printed for"
		);
		assert!(!joined.contains('\u{1b}'));
	}
}

#[test]
fn a_secret_name_that_is_not_a_name_gets_no_command_whatever_its_source() {
	for source in ["environment: DB_PASSWORD", "content: x"] {
		let yaml = format!(
			"services:\n  web:\n    image: img\n    secrets: ['a b; reboot']\nsecrets:\n  'a b; reboot':\n    {source}\n"
		);
		let out = generate_at(&parse_str(&yaml).unwrap(), "proj", Path::new("/srv/app"));
		let joined = out.warnings.join("\n");
		assert!(joined.contains("rename it"));
		assert!(!joined.contains("podman secret create"));
		assert!(!joined.contains("reboot"));
	}
}

// --- what the export refuses -------------------------------------------------

fn refused(yaml: &str) -> bool {
	validate_for_quadlet_in(&parse_str(yaml).unwrap(), "proj", Path::new("/srv/app")).is_err()
}

#[test]
fn a_config_field_with_a_separator_a_blank_a_quote_or_a_control_character_is_refused() {
	for bad in [
		"a,b", "a=b", "a b", "a\"b", "a'b", "a\nb", "a\tb", "a\u{1b}b",
	] {
		let quoted = serde_json::to_string(bad).unwrap();
		for field in ["source", "target", "uid", "gid"] {
			let source = if field == "source" {
				quoted.as_str()
			} else {
				"app"
			};
			let extra = if field == "source" {
				String::new()
			} else {
				format!("        {field}: {quoted}\n")
			};
			let second = if source == "app" {
				String::new()
			} else {
				format!("  {source}:\n    content: x\n")
			};
			let yaml = format!(
				"services:\n  web:\n    image: img\n    configs:\n      - source: {source}\n{extra}configs:\n  app:\n    content: x\n{second}"
			);
			assert!(refused(&yaml), "a config was accepted");
		}
	}
}

#[test]
fn the_short_form_of_a_config_is_checked_too() {
	assert!(refused(
		"services:\n  web:\n    image: img\n    configs: ['a b']\nconfigs:\n  'a b':\n    content: x\n"
	));
}

#[test]
fn the_name_of_an_external_config_or_secret_is_checked() {
	assert!(refused(
		"services:\n  web:\n    image: img\n    configs: [c]\nconfigs:\n  c:\n    external: true\n    name: 'x,mode=777'\n"
	));
	assert!(refused(
		"services:\n  web:\n    image: img\n    secrets: [c]\nsecrets:\n  c:\n    external: true\n    name: 'x,mode=777'\n"
	));
	assert!(!refused(
		"services:\n  web:\n    image: img\n    configs: [c]\nconfigs:\n  c:\n    external: true\n    name: pre-made.cfg\n"
	));
}

#[test]
fn a_dangerous_mode_on_a_secret_or_a_config_is_refused_as_up_refuses_it() {
	for mode in ["0755", "0111", "04444", "02444", "01444"] {
		for kind in ["secrets", "configs"] {
			let yaml = format!(
				"services:\n  web:\n    image: img\n    {kind}:\n      - source: x\n        mode: {mode}\n{kind}:\n  x:\n    content: x\n"
			);
			assert!(refused(&yaml), "mode was accepted");
		}
	}
	assert!(!refused(
		"services:\n  web:\n    image: img\n    secrets:\n      - source: x\n        mode: 0400\nsecrets:\n  x:\n    content: x\n"
	));
}

/// The units `autostart --mode quadlet` installs mount no config and have no
/// secrets unit, so a file that installs today keeps installing: the checks
/// above are made by the command that writes the units, and not by
/// `validate_for_quadlet`, which autostart calls.
#[test]
fn what_autostart_validates_is_not_made_stricter() {
	let file = parse_str(
		"services:\n  web:\n    image: img\n    configs: ['a b']\nconfigs:\n  'a b':\n    content: x\n",
	)
	.unwrap();
	assert!(validate_for_quadlet_in(&file, "proj", Path::new("/srv/app")).is_err());
	assert!(validate_for_quadlet(&file).is_ok());
}

/// A file mode the export refuses says which file, which mode and what to do.
#[cfg(unix)]
#[test]
fn a_refused_file_mode_names_the_file_the_mode_and_the_fix() {
	let dir = tempfile::tempdir().unwrap();
	file_with_mode(dir.path(), "s", 0o755);
	let out = generate_in(
		dir.path(),
		"services:\n  web:\n    image: img\n    secrets: [s]\nsecrets:\n  s:\n    file: ./s\n",
	);
	let warning = out
		.warnings
		.iter()
		.find(|w| w.contains("not exported"))
		.unwrap();
	let file = dir.path().join("s");
	assert!(warning.contains(file.to_str().unwrap()));
	assert!(warning.contains("0755"));
	assert!(warning.contains("chmod 0600"));
	assert!(warning.contains("generate the units again"));
}

fn validate_in(file: &crate::compose::types::ComposeFile) -> crate::error::Result<()> {
	validate_for_quadlet_in(file, "proj", Path::new("/srv/app"))
}

fn one_secret(path: &str) -> String {
	format!(
		"services:\n  web:\n    image: img\n    secrets: [token]\nsecrets:\n  token:\n    file: {}\n",
		yaml_path(path)
	)
}

// --- what is at the path ---------------------------------------------------

/// `up` reads a `file:` source without following a link at its last component and
/// refuses a link, a directory, a pipe and a device (measured on Podman 6.2.0:
/// `refusing to read link`, `directory`, `FIFO`, `character device`). The export
/// refuses them too, whatever they point at, and says what it found.
#[cfg(unix)]
#[test]
fn a_link_a_directory_a_pipe_and_a_device_are_not_exported() {
	use std::os::unix::fs::symlink;
	let dir = tempfile::tempdir().unwrap();
	let real = dir.path().join("real");
	std::fs::write(&real, "x").unwrap();
	let folder = dir.path().join("folder");
	std::fs::create_dir(&folder).unwrap();
	symlink(&real, dir.path().join("to-file")).unwrap();
	symlink("/etc/shadow", dir.path().join("to-shadow")).unwrap();
	symlink(&folder, dir.path().join("to-dir")).unwrap();
	symlink("/nowhere/at/all", dir.path().join("dangling")).unwrap();
	let fifo = dir.path().join("pipe");
	assert!(std::process::Command::new("mkfifo")
		.arg(&fifo)
		.status()
		.unwrap()
		.success());
	for (name, what) in [
		("to-file", "link"),
		("to-shadow", "link"),
		("to-dir", "link"),
		("dangling", "link"),
		("folder", "directory"),
		("pipe", "FIFO"),
	] {
		let path = dir.path().join(name);
		let out = generate_in(dir.path(), &one_secret(path.to_str().unwrap()));
		assert!(creates_nothing(&out), "was exported");
		assert!(out
			.warnings
			.iter()
			.any(|w| w.contains(&format!("it is a {what}")) && w.contains("`up` refuses it")));
	}
	let out = generate_in(dir.path(), &one_secret("/dev/null"));
	assert!(creates_nothing(&out));
	assert!(out.warnings.iter().any(|w| w.contains("it is a device")));
	// A regular file, and a link in a directory above it (which `up` follows), are
	// exported.
	symlink(dir.path(), dir.path().join("via")).unwrap();
	let ok = dir.path().join("via").join("real");
	let out = generate_in(dir.path(), &one_secret(ok.to_str().unwrap()));
	assert!(secrets_unit(&out).is_some());
}

// --- the path in a command the operator will copy --------------------------

/// A command is printed only for a path made of characters no shell reads as
/// more; any other path is shown with no command.
#[cfg(unix)]
#[test]
fn the_advice_prints_a_command_only_for_a_path_a_shell_reads_as_it_is() {
	use std::os::unix::fs::PermissionsExt;
	let dir = tempfile::tempdir().unwrap();
	let advice_for = |name: &str| {
		let path = dir.path().join(name);
		std::fs::write(&path, "x").unwrap();
		std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
		let out = generate_in(dir.path(), &one_secret(path.to_str().unwrap()));
		assert!(creates_nothing(&out));
		out.warnings.join("\n")
	};
	let plain = advice_for("plain-file_1.txt");
	assert!(plain.contains("`chmod 0600 "));
	for hostile in [
		"a b",
		"a;touch pwned",
		"a$(id)",
		"a`id`",
		"a'b",
		"a\"b",
		"a&b",
		"a|b",
		"a\u{202e}b",
		"a>b",
	] {
		let text = advice_for(hostile);
		assert!(!text.contains("chmod"));
		assert!(!text.contains("0600 "));
	}
	// A file that cannot be read is told in the same way.
	let text = generate_in(dir.path(), &one_secret("/srv/does not;exist/$(id)"))
		.warnings
		.join("\n");
	assert!(text.contains("could not be read"));
	assert!(!text.contains('`'));
}

// --- one source ------------------------------------------------------------

fn two_sources(kind: &str, first: &str, second: &str) -> String {
	let (section, mount) = if kind == "secret" {
		("secrets", "secrets: [x]")
	} else {
		("configs", "configs: [x]")
	};
	format!(
		"services:\n  web:\n    image: img\n    {mount}\n{section}:\n  x:\n    {first}\n    {second}\n"
	)
}

/// `up` takes `content`, then `environment`, then `file`, and the Compose
/// Specification allows one. A definition with two is refused by the export, and
/// when the check is not run it is left to the operator and nothing is exported.
#[test]
fn a_definition_with_more_than_one_source_is_refused() {
	let sources = ["file: ./f", "environment: HOME", "content: hello"];
	for kind in ["secret", "config"] {
		for (i, first) in sources.iter().enumerate() {
			for second in &sources[i + 1..] {
				let yaml = two_sources(kind, first, second);
				let file = parse_str(&yaml).unwrap();
				let error = validate_in(&file).unwrap_err().to_string();
				assert!(
					error.contains("more than one of `file`, `environment` and `content`")
						&& error.contains(kind)
						&& error.contains("service 'web'")
				);
				// The units `autostart` installs have none of this.
				validate_for_quadlet(&file).unwrap();
				// And with the check not run, nothing is exported.
				let out = generate_at(&file, "proj", Path::new("/srv/app"));
				assert!(creates_nothing(&out));
				assert!(out.warnings.iter().any(|w| w.contains("more than one of")));
			}
		}
	}
	// One source is fine, and so is a definition no service mounts.
	validate_in(&parse_str(&two_sources("secret", "file: ./f", "external: true")).unwrap())
		.unwrap();
	let unmounted =
		"services:\n  web:\n    image: img\nsecrets:\n  x:\n    file: ./f\n    content: hello\n";
	validate_in(&parse_str(unmounted).unwrap()).unwrap();
}

// --- the mode the file had -------------------------------------------------

/// The unit carries the permission bits the file had when it was generated, as
/// the last word of each command, for the script to compare; when the file could
/// not be read there is nothing to compare and the word is empty.
#[cfg(unix)]
#[test]
fn the_unit_carries_the_mode_the_file_had() {
	use std::os::unix::fs::PermissionsExt;
	let dir = tempfile::tempdir().unwrap();
	let tight = dir.path().join("tight");
	std::fs::write(&tight, "x").unwrap();
	std::fs::set_permissions(&tight, std::fs::Permissions::from_mode(0o600)).unwrap();
	let out = generate_in(dir.path(), &one_secret(tight.to_str().unwrap()));
	let unit = secrets_unit(&out).unwrap();
	assert!(unit.ends_with(&format!("\"{}\" \"0600\"\n", tight.display())));
	let out = generate_in(dir.path(), &one_secret("/srv/not/there"));
	let unit = secrets_unit(&out).unwrap();
	assert!(unit.ends_with("\"/srv/not/there\" \"\"\n"));
	// What the script does with them is run against a real shell below.
	assert!(super::CREATE_SCRIPT.contains("$((0$m)) -eq $((0$4))"));
}

/// A word with a control character is not made, whoever asks: the path is checked
/// before, and this is the check that holds when it is not.
#[test]
fn a_word_with_a_control_character_is_not_made() {
	for text in ["a\nb", "a\rb", "a\0b", "a\u{1b}b", "a\tb"] {
		assert!(super::exec_word(text).is_none());
	}
	assert_eq!(super::exec_word("a b").as_deref(), Some("\"a b\""));
}

// --- jobs ---------------------------------------------------------------------

/// A job with a schedule is written as a container unit by the code that writes a
/// service, so what only it mounts is created by the same unit, and its container
/// requires the unit.
#[test]
fn a_secret_used_only_by_a_job_gets_its_unit_and_the_job_requires_it() {
	let yaml = "jobs:\n  nightly:\n    image: docker.io/library/alpine\n    command: [echo, hi]\n    triggers:\n      schedule: ['0 3 * * *']\n    secrets: [token]\n    configs: [conf]\nsecrets:\n  token:\n    file: /srv/app/token.txt\nconfigs:\n  conf:\n    file: /srv/app/conf.txt\n";
	let out = generate_at(&parse_str(yaml).unwrap(), "proj", Path::new("/srv/app"));
	let unit = secrets_unit(&out).expect("a job's secrets have no unit");
	assert!(unit.contains("\"proj_secret_token\""));
	assert!(unit.contains("\"proj_config_conf\""));
	let job = out
		.units
		.iter()
		.find(|u| u.filename.ends_with("nightly.container"))
		.expect("no job unit");
	assert!(job
		.contents
		.contains("Requires=proj-podup-secrets.service\n"));
	// A job with no schedule writes no unit, and what only it mounts is not created.
	let manual = yaml.replace("schedule: ['0 3 * * *']", "manual: true");
	let out = generate_at(&parse_str(&manual).unwrap(), "proj", Path::new("/srv/app"));
	assert!(secrets_unit(&out).is_none());
}

// --- what was refused ------------------------------------------------------

/// A source `up` would refuse is told in the warnings, is an error for the command
/// that writes the units, and in the unit a command for its name fails, so the
/// container that mounts it does not start with whatever secret has that name.
#[cfg(unix)]
#[test]
fn a_refused_source_fails_closed_in_the_command_in_the_unit_and_in_the_container() {
	use std::os::unix::fs::{symlink, PermissionsExt};
	let dir = tempfile::tempdir().unwrap();
	let real = dir.path().join("real");
	std::fs::write(&real, "x").unwrap();
	symlink(&real, dir.path().join("link")).unwrap();
	std::fs::create_dir(dir.path().join("folder")).unwrap();
	let exec = dir.path().join("exec");
	std::fs::write(&exec, "x").unwrap();
	std::fs::set_permissions(&exec, std::fs::Permissions::from_mode(0o755)).unwrap();
	for name in ["link", "folder", "exec"] {
		let yaml = one_secret(dir.path().join(name).to_str().unwrap());
		let file = parse_str(&yaml).unwrap();
		let error = validate_in(&file).unwrap_err().to_string();
		assert!(error.contains("not exported"));
		let out = generate_in(dir.path(), &yaml);
		assert!(creates_nothing(&out));
		let unit = secrets_unit(&out).expect("no command for the refused name");
		assert!(unit.contains("was not exported"));
		let web = out
			.units
			.iter()
			.find(|u| u.filename == "proj-web.container")
			.unwrap();
		assert!(web
			.contents
			.contains("Requires=proj-podup-secrets.service\n"));
		assert!(web
			.contents
			.contains("Secret=proj_secret_token,target=token,mode=400\n"));
	}
	// The command for the name fails and says which.
	let yaml = one_secret(dir.path().join("link").to_str().unwrap());
	let out = generate_in(dir.path(), &yaml);
	let words = super::real_tests::exec_lines(secrets_unit(&out).unwrap()).remove(0);
	let run = std::process::Command::new(&words[0])
		.args(&words[1..])
		.output()
		.unwrap();
	assert!(!run.status.success());
	assert!(String::from_utf8_lossy(&run.stderr).contains("token was not exported"));
	// A file that is not there yet is not refused: the unit's command fails at
	// the start, which is when it matters.
	let later = one_secret(dir.path().join("later").to_str().unwrap());
	validate_in(&parse_str(&later).unwrap()).unwrap();
}

/// The unit has a timeout, because a path that became a pipe after it was checked
/// would make the open wait for a writer.
#[test]
fn the_unit_does_not_wait_for_ever() {
	let out = generate_in(Path::new("/srv/app"), &one_secret("/srv/app/token"));
	assert!(secrets_unit(&out)
		.unwrap()
		.contains("TimeoutStartSec=120\n"));
}

/// A path in a unit is the same on every host that generates it: the parts that
/// mean nothing are dropped, `..` is not resolved.
#[test]
fn a_path_is_written_without_the_parts_that_mean_nothing() {
	for (given, want) in [
		("/srv//app/./x/../y/", "/srv/app/x/../y"),
		("/", "/"),
		("/srv/app/./tok", "/srv/app/tok"),
		("rel/./a", "rel/a"),
		// A `\` is a character of a POSIX name, not a separator.
		("/srv/my dir/a\\d", "/srv/my dir/a\\d"),
	] {
		assert_eq!(super::tidy_path(given), want);
	}
}
