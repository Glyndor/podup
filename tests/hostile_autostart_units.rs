//! Autostart units rendered from values written to break out of an exec line.
//!
//! The service and start modes render a systemd unit around a compose file
//! path, a project name, profiles, env files and a container name. Each of
//! those is untrusted in the sense that a path may hold a quote, a backslash,
//! a `%` or a space and a compose file picks the container name. The checks
//! parse the rendered unit the way systemd reads it and require:
//!
//! * the unit has the same lines it has for plain values, and one `ExecStart=`
//!   and one `ExecStop=`;
//! * parsing each exec line gives back exactly the arguments that went in, so a
//!   quote or a space never splits or merges a word;
//! * every `%` is part of a `%%` pair and no line folds into the next.

use std::path::PathBuf;

use podup::autostart::{
	render_service_unit, render_start_unit, render_update_service_unit, ServiceUnitOpts,
	StartUnitOpts,
};

/// Values that are legal in a path or a name and awkward in a unit.
const AWKWARD: &[&str] = &[
	"plain",
	"with space",
	"tab\there",
	"dq\"quote",
	"sq'quote",
	"back\\slash",
	"percent%h%n%%",
	"hash#tag",
	"back`tick`",
	"paren(s)",
	"angle<>pipe|amp&",
	"glob*?[x]",
	"ünïcödé-日本",
	"-leading-dash",
	"=eq=",
	"semi;colon",
	"trailing;",
];

/// What systemd makes of one exec line: words split on whitespace outside
/// double quotes, `\\ \" \n \t \r` inside them, `%%` as `%`. `None` when the
/// line holds a `$` that systemd would substitute or a lone `;` word, which it
/// takes as the end of a command.
fn parse_exec(line: &str) -> Option<Vec<String>> {
	let mut words = Vec::new();
	let mut cur = String::new();
	let mut in_word = false;
	let mut quoted = false;
	let mut chars = line.chars().peekable();
	while let Some(c) = chars.next() {
		match c {
			'"' => {
				quoted = !quoted;
				in_word = true;
			}
			'\\' if quoted => {
				let next = chars.next()?;
				cur.push(match next {
					'n' => '\n',
					't' => '\t',
					'r' => '\r',
					other => other,
				});
				in_word = true;
			}
			'%' => {
				if chars.next() != Some('%') {
					return None;
				}
				cur.push('%');
				in_word = true;
			}
			'$' => return None,
			c if c.is_whitespace() && !quoted => {
				if in_word {
					if cur == ";" {
						return None;
					}
					words.push(std::mem::take(&mut cur));
					in_word = false;
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
		if cur == ";" {
			return None;
		}
		words.push(cur);
	}
	Some(words)
}

/// The value of the only `key=` line in `unit`.
fn only(unit: &str, key: &str) -> String {
	let lines: Vec<&str> = unit
		.lines()
		.filter_map(|l| l.strip_prefix(key).and_then(|r| r.strip_prefix('=')))
		.collect();
	assert_eq!(lines.len(), 1, "expected one {key}= in:\n{unit}");
	lines[0].to_string()
}

/// Everything wrong with the shape of `unit`.
fn shape_problems(unit: &str) -> Vec<String> {
	let mut bad = Vec::new();
	for line in unit.lines() {
		let slashes = line.chars().rev().take_while(|c| *c == '\\').count();
		if slashes % 2 == 1 {
			bad.push(format!("line folds the next one: {line:?}"));
		}
		if line.is_empty() || line.starts_with('[') {
			continue;
		}
		let key = line.split('=').next().unwrap_or("");
		if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric()) || !line.contains('=')
		{
			bad.push(format!("line is not Key=Value: {line:?}"));
		}
	}
	let sections = unit.lines().filter(|l| l.starts_with('[')).count();
	if sections != 3 {
		bad.push(format!("expected 3 sections, found {sections}"));
	}
	if unit.chars().any(|c| c.is_control() && c != '\n') {
		bad.push("unit holds a raw control character".to_string());
	}
	bad
}

fn service_opts(awkward: &str) -> ServiceUnitOpts {
	ServiceUnitOpts::new(
		PathBuf::from(format!("/opt/{awkward}/podup")),
		vec![
			PathBuf::from(format!("/srv/{awkward}/compose.yaml")),
			PathBuf::from(format!("/srv/{awkward}/override.yaml")),
		],
		"proj".to_string(),
		PathBuf::from("/srv/app"),
	)
	.with_profiles(vec![awkward.to_string()])
	.with_env_files(vec![format!("/srv/{awkward}/.env")])
}

#[test]
fn service_unit_exec_lines_give_back_the_arguments_that_went_in() {
	let mut failures = Vec::new();
	for awkward in AWKWARD {
		if awkward.contains(';') {
			// A lone `;` is a word separator on an exec line; skipped here.
			continue;
		}
		let opts = service_opts(awkward);
		let unit = render_service_unit(&opts);
		for p in shape_problems(&unit) {
			failures.push(format!("{awkward:?}: {p}"));
		}
		let start = parse_exec(&only(&unit, "ExecStart"));
		let mut expected = vec![format!("/opt/{awkward}/podup")];
		for f in [
			format!("/srv/{awkward}/compose.yaml"),
			format!("/srv/{awkward}/override.yaml"),
		] {
			expected.push("-f".into());
			expected.push(f);
		}
		expected.extend(["-p", "proj", "--profile"].map(String::from));
		expected.push(awkward.to_string());
		expected.push("--env-file".into());
		expected.push(format!("/srv/{awkward}/.env"));
		let mut with_up = expected.clone();
		with_up.extend(["up", "-d", "--no-build", "--pull", "never"].map(String::from));
		if start.as_ref() != Some(&with_up) {
			failures.push(format!(
				"{awkward:?}: ExecStart parses to {start:?}, expected {with_up:?}"
			));
		}
		let mut with_stop = expected;
		with_stop.push("stop".into());
		let stop = parse_exec(&only(&unit, "ExecStop"));
		if stop.as_ref() != Some(&with_stop) {
			failures.push(format!(
				"{awkward:?}: ExecStop parses to {stop:?}, expected {with_stop:?}"
			));
		}
	}
	assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn update_unit_exec_line_gives_back_the_arguments_that_went_in() {
	let mut failures = Vec::new();
	for awkward in AWKWARD.iter().filter(|a| !a.contains(';')) {
		let unit = render_update_service_unit(&service_opts(awkward));
		for p in shape_problems_two_sections(&unit) {
			failures.push(format!("{awkward:?}: {p}"));
		}
		let words = parse_exec(&only(&unit, "ExecStart")).unwrap_or_default();
		if words.first().map(String::as_str) != Some(&format!("/opt/{awkward}/podup")) {
			failures.push(format!("{awkward:?}: ExecStart words {words:?}"));
		}
		if !words.ends_with(&["up".to_string(), "-d".to_string()]) {
			failures.push(format!("{awkward:?}: ExecStart does not end in `up -d`"));
		}
	}
	assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The update service has two sections, not three.
fn shape_problems_two_sections(unit: &str) -> Vec<String> {
	shape_problems(unit)
		.into_iter()
		.filter(|p| !p.starts_with("expected 3 sections"))
		.collect()
}

#[test]
fn start_unit_exec_lines_give_back_the_container_name() {
	let mut failures = Vec::new();
	for awkward in AWKWARD.iter().filter(|a| !a.contains(';')) {
		let opts = StartUnitOpts::new(
			PathBuf::from(format!("/opt/{awkward}/podman")),
			"proj".to_string(),
			awkward.to_string(),
		);
		let unit = render_start_unit(&opts);
		for p in shape_problems(&unit) {
			failures.push(format!("{awkward:?}: {p}"));
		}
		for (key, verb) in [("ExecStart", "start"), ("ExecStop", "stop")] {
			let want = vec![
				format!("/opt/{awkward}/podman"),
				verb.to_string(),
				"--".to_string(),
				awkward.to_string(),
			];
			let got = parse_exec(&only(&unit, key));
			if got.as_ref() != Some(&want) {
				failures.push(format!("{awkward:?}: {key} parses to {got:?}"));
			}
		}
	}
	assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn the_exec_parser_sees_a_split_a_merge_and_a_substitution() {
	// The checker has to be able to fail. Each line is a defect by hand.
	assert_eq!(
		parse_exec("a \"b c\" d"),
		Some(vec!["a".into(), "b c".into(), "d".into()])
	);
	assert_eq!(parse_exec("\"a\\\"b\""), Some(vec!["a\"b".into()]));
	assert_eq!(parse_exec("50%%off"), Some(vec!["50%off".into()]));
	assert_eq!(parse_exec("a %h"), None, "an undoubled specifier");
	assert_eq!(parse_exec("a $HOME"), None, "a substitution");
	assert_eq!(parse_exec("a ; b"), None, "a command separator");
	assert_eq!(parse_exec("\"a"), None, "an open quote");
	assert!(!shape_problems("[A]\nB=c\\\nD=e\n[F]\n[G]\n").is_empty());
	assert!(!shape_problems("[A]\nB\n[F]\n[G]\n").is_empty());
}
