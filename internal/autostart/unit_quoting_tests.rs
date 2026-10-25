//! How the autostart units spell values that systemd would otherwise read:
//! `$`, a lone `;`, a trailing backslash and a name that looks like a flag.

use super::service::*;
use super::start::*;
use std::path::PathBuf;

/// systemd's reading of an exec line: `%%` and `$$` collapse, a lone `%` or
/// `$` is a substitution (`None`), quoted text is one word, and only a bare `;`
/// word ends the command (measured with `systemd-analyze verify`: a quoted
/// `";"` stays an argument).
fn parse_exec(line: &str) -> Option<Vec<String>> {
	let mut words = Vec::new();
	let (mut cur, mut in_word, mut quoted) = (String::new(), false, false);
	let mut chars = line.chars().peekable();
	let mut bare_semicolon = false;
	let mut cur_quoted = false;
	while let Some(c) = chars.next() {
		match c {
			'"' => {
				quoted = !quoted;
				in_word = true;
				cur_quoted = true;
			}
			'\\' if quoted => {
				let next = chars.next()?;
				cur.push(match next {
					'n' => '\n',
					't' => '\t',
					'r' => '\r',
					other => other,
				});
			}
			'%' | '$' => {
				if chars.next() != Some(c) {
					return None;
				}
				cur.push(c);
				in_word = true;
			}
			c if c.is_whitespace() && !quoted => {
				if in_word {
					bare_semicolon |= cur == ";" && !cur_quoted;
					words.push(std::mem::take(&mut cur));
					in_word = false;
					cur_quoted = false;
				}
			}
			c => {
				cur.push(c);
				in_word = true;
			}
		}
	}
	if quoted {
		return None;
	}
	if in_word {
		bare_semicolon |= cur == ";" && !cur_quoted;
		words.push(cur);
	}
	(!bare_semicolon).then_some(words)
}

fn line<'a>(unit: &'a str, key: &str) -> &'a str {
	unit.lines()
		.find(|l| l.starts_with(&format!("{key}=")))
		.unwrap_or_else(|| panic!("no {key}= in:\n{unit}"))
}

fn service_opts(dir: &str) -> ServiceUnitOpts {
	ServiceUnitOpts::new(
		PathBuf::from("/usr/bin/podup"),
		vec![PathBuf::from("/srv/app/compose.yaml")],
		"proj".into(),
		PathBuf::from(dir),
	)
}

#[test]
fn a_container_name_that_looks_like_a_flag_stays_a_name() {
	let unit = render_start_unit(&StartUnitOpts::new(
		PathBuf::from("/usr/bin/podman"),
		"p".into(),
		"--all".into(),
	));
	assert_eq!(
		line(&unit, "ExecStart"),
		"ExecStart=/usr/bin/podman start -- --all"
	);
	assert_eq!(
		line(&unit, "ExecStop"),
		"ExecStop=/usr/bin/podman stop -- --all"
	);
}

#[test]
fn both_shapes_of_a_start_unit_are_still_recognised() {
	use super::conflict::{classify_exec_start, DetectedSingleUnitMode::Start};
	assert_eq!(
		classify_exec_start("ExecStart=/usr/bin/podman start -- app"),
		Start
	);
	assert_eq!(
		classify_exec_start("ExecStart=/usr/bin/podman start app"),
		Start
	);
}

#[test]
fn a_working_directory_ending_in_a_backslash_does_not_fold_the_next_line() {
	for unit in [
		render_service_unit(&service_opts("/srv/app\\")),
		render_update_service_unit(&service_opts("/srv/app\\")),
	] {
		assert_eq!(
			line(&unit, "WorkingDirectory"),
			"WorkingDirectory=/srv/app\\/"
		);
		assert!(unit.lines().any(|l| l.starts_with("ExecStart=")), "{unit}");
	}
}

#[test]
fn a_dollar_in_an_argument_is_written_doubled() {
	for awkward in ["$HOME", "a$b", "${X}"] {
		let unit =
			render_service_unit(&service_opts("/srv/app").with_profiles(vec![awkward.into()]));
		let words = parse_exec(line(&unit, "ExecStart").strip_prefix("ExecStart=").unwrap());
		assert!(
			words.is_some_and(|w| w.contains(&awkward.to_string())),
			"profile {awkward:?} does not reach podup unchanged"
		);
	}
}

#[test]
fn a_dollar_in_an_env_file_path_is_written_doubled() {
	let unit =
		render_service_unit(&service_opts("/srv/app").with_env_files(vec!["/e/$USER.env".into()]));
	assert!(line(&unit, "ExecStart").contains("/e/$$USER.env"), "{unit}");
}

#[test]
fn a_lone_semicolon_profile_stays_an_argument() {
	let unit = render_service_unit(&service_opts("/srv/app").with_profiles(vec![";".into()]));
	let words = parse_exec(line(&unit, "ExecStart").strip_prefix("ExecStart=").unwrap());
	assert!(
		words.is_some_and(|w| w.contains(&";".to_string())),
		"{unit}"
	);
}

/// Hand the unit to `systemd-analyze verify` (it parses; it starts nothing).
/// `None` when the tool is missing.
#[cfg(unix)]
fn verify(unit: &str) -> Option<String> {
	let dir = tempfile::tempdir().ok()?;
	let path = dir.path().join("t.service");
	std::fs::write(&path, unit).ok()?;
	let run = std::process::Command::new("systemd-analyze")
		.args(["--user", "verify"])
		.arg(&path)
		.output()
		.ok()?;
	Some(String::from_utf8_lossy(&run.stderr).into_owned() + &String::from_utf8_lossy(&run.stdout))
}

#[cfg(unix)]
#[test]
fn systemd_reads_a_semicolon_profile_and_a_backslash_directory_as_one_command() {
	let mut opts = service_opts("/srv/app\\").with_profiles(vec![";".into(), "up".into()]);
	opts.exe = PathBuf::from("/usr/bin/true");
	let unit = render_service_unit(&opts);
	let Some(out) = verify(&unit) else {
		return;
	};
	// A bare `;` would make systemd check `up` as a second executable, and a
	// folded line would leave the unit without an `ExecStart=`.
	assert!(!out.contains("is not executable"), "{out}");
	assert!(!out.contains("no ExecStart"), "{out}");
}
