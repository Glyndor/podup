//! Quadlet units generated from compose values written to inject into them.
//!
//! A compose file is untrusted input and `generate quadlet` turns it into
//! systemd units that run as the user. A value that smuggles a line break, a
//! second section or a specifier into a unit would run something the compose
//! file never declared. Each case puts a hostile payload into one compose field
//! and checks the structure of every generated unit, not its exact text:
//!
//! * no character that is a control character other than the line feed that
//!   ends a line (so no `\r`, NUL or ESC reaches a unit);
//! * every line is blank, a comment, a known `[Section]` header or `Key=...`;
//! * no directive the payload tried to add shows up as a line of its own;
//! * no unit has two headers of the same section;
//! * no line ends in an odd run of backslashes, which would fold the next line
//!   into it;
//! * every `%` outside a command line is part of a `%%` pair, so no specifier
//!   is expanded at runtime;
//! * the unit file name is one safe path component.
//!
//! A field the parser or the exporter refuses outright counts as handled.

#[path = "../../support/quadlet_cases.rs"]
mod quadlet_cases;

#[cfg(unix)]
#[path = "../../support/quadlet_env_files.rs"]
mod quadlet_env_files;

use podup::parse_str;
use podup::quadlet::{generate_at, QuadletOutput};
use quadlet_cases::cases;

/// Section headers a generated unit may carry.
const SECTIONS: &[&str] = &[
	"Unit",
	"Container",
	"Service",
	"Install",
	"Network",
	"Volume",
	"Build",
	"Pod",
];

/// Payloads. Each one tries a different way out of a `Key=Value` line.
/// `{C}` is replaced by a canary token no honest unit contains.
const PAYLOADS: &[&str] = &[
	// New lines, a new section and a `\r` line ending, then control bytes.
	"a\nExecStartPre=/bin/{C}\n[Service]\nExecStart=/bin/{C}\r\n[Install]\nWantedBy={C}\n\0\u{1b}]0;t\u{7}",
	// Specifiers, quotes, shell syntax and a trailing backslash.
	"a\"; ExecStartPre=/bin/{C} ;\" %h %n %U %t $HOME ${HOME} `id` $(id) \\",
	// Line continuation: the value ends in a backslash.
	"/bin/{C}\\",
];

/// Quote `s` as a YAML double-quoted scalar.
fn q(s: &str) -> String {
	let mut out = String::from("\"");
	for c in s.chars() {
		match c {
			'"' => out.push_str("\\\""),
			'\\' => out.push_str("\\\\"),
			'\n' => out.push_str("\\n"),
			'\r' => out.push_str("\\r"),
			'\0' => out.push_str("\\0"),
			c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
			c => out.push(c),
		}
	}
	out.push('"');
	out
}

/// Everything wrong with the structure of `out`.
fn problems(units: &[(&str, &str)], canary: &str) -> Vec<String> {
	let mut bad = Vec::new();
	for &(name, contents) in units {
		if name.is_empty()
			|| name.starts_with(['.', '-'])
			|| !name
				.chars()
				.all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
		{
			bad.push(format!(
				"unit file name is not one safe component: {name:?}"
			));
		}
		if let Some(c) = contents.chars().find(|c| c.is_control() && *c != '\n') {
			bad.push(format!("{name}: raw control character {c:?}"));
		}
		let mut seen: Vec<&str> = Vec::new();
		for line in contents.lines() {
			let trailing = line.chars().rev().take_while(|c| *c == '\\').count();
			if trailing % 2 == 1 {
				bad.push(format!("{name}: line folds the next one: {line:?}"));
			}
			if line.is_empty() || line.starts_with('#') {
				continue;
			}
			// Command lines (`Exec=`, `Entrypoint=`) are outside this rule.
			let command_line = line.starts_with("Exec=") || line.starts_with("Entrypoint=");
			if !command_line && line.replace("%%", "").contains('%') {
				bad.push(format!("{name}: a `%` that is not doubled: {line:?}"));
			}
			if let Some(section) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
				if !SECTIONS.contains(&section) {
					bad.push(format!("{name}: unknown section {line:?}"));
				}
				if seen.contains(&section) {
					bad.push(format!("{name}: section [{section}] appears twice"));
				}
				seen.push(section);
				continue;
			}
			let key = line.split('=').next().unwrap_or("");
			if key.is_empty()
				|| !key.chars().all(|c| c.is_ascii_alphanumeric())
				|| !line.contains('=')
			{
				bad.push(format!("{name}: line is not Key=Value: {line:?}"));
			}
			// The secrets unit holds file paths in the arguments of its `ExecStart=`
			// lines, each as one double-quoted word: the text of a payload inside the
			// quotes is an argument, not a directive.
			let in_quotes =
				key == "ExecStart" && only_inside_quotes(line, &format!("/bin/{canary}"));
			if line.contains(&format!("/bin/{canary}"))
				&& key.starts_with("Exec")
				&& key != "Exec"
				&& !in_quotes
			{
				bad.push(format!("{name}: injected directive: {line:?}"));
			}
			if line.starts_with("WantedBy=") && line.contains(canary) {
				bad.push(format!("{name}: injected install target: {line:?}"));
			}
		}
	}
	bad
}

/// Whether every occurrence of `needle` in `line` is inside a double-quoted word,
/// where systemd reads it as part of one argument. A `\"` does not close a quote.
fn only_inside_quotes(line: &str, needle: &str) -> bool {
	let (mut inside, mut escaped) = (false, false);
	let mut quoted = vec![false; line.len() + 1];
	for (at, c) in line.char_indices() {
		if escaped {
			escaped = false;
		} else if c == '\\' {
			escaped = true;
		} else if c == '"' {
			inside = !inside;
		}
		quoted[at..at + c.len_utf8()].fill(inside);
	}
	line.match_indices(needle).all(|(at, _)| quoted[at])
}

/// The (file name, contents) of every unit in `out`.
fn pairs(out: &QuadletOutput) -> Vec<(&str, &str)> {
	out.units
		.iter()
		.map(|u| (u.filename.as_str(), u.contents.as_str()))
		.collect()
}

/// Generate units for `yaml`, or `None` when parsing or exporting refuses it.
fn export(yaml: &str) -> Option<QuadletOutput> {
	let file = parse_str(yaml).ok()?;
	podup::quadlet::validate_for_quadlet(&file).ok()?;
	Some(generate_at(&file, "proj", std::path::Path::new("/srv/app")))
}

#[test]
fn no_compose_field_injects_into_a_unit() {
	let mut failures = Vec::new();
	let mut generated = 0;
	for (i, payload) in PAYLOADS.iter().enumerate() {
		let canary = format!("canary{i}x");
		let payload = payload.replace("{C}", &canary);
		for case in cases() {
			let yaml = (case.yaml)(&q(&payload));
			let Some(out) = export(&yaml) else { continue };
			generated += 1;
			for p in problems(&pairs(&out), &canary) {
				failures.push(format!("[{} / payload {i}] {p}", case.name));
			}
		}
	}
	assert!(
		generated > 20,
		"only {generated} cases reached the exporter; the table is not exercising it"
	);
	assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The checker has to be able to see an injection, or the test above proves
/// nothing. These are units written by hand with each defect.
#[test]
fn the_structure_check_sees_each_kind_of_injection() {
	let unit = |contents: &'static str| vec![("proj-web.container", contents)];
	let cases = [
		("extra directive", "[Service]\nExecStartPre=/bin/c\n"),
		(
			"second section",
			"[Service]\nRestart=no\n[Service]\nRestart=no\n",
		),
		("raw cr", "[Container]\nImage=a\rb\n"),
		("unknown section", "[Evil]\nA=b\n"),
		("single percent", "[Container]\nEnvironment=A=%h\n"),
		("folding backslash", "[Container]\nImage=a\\\nExec=b\n"),
		("not key value", "[Container]\nnonsense\n"),
		(
			"a quote that closed early",
			"[Service]\nExecStart=/bin/sh -c \"x\" \"a\" ; ExecStartPre=/bin/c\n",
		),
	];
	for (what, contents) in cases {
		assert!(
			!problems(&unit(contents), "c").is_empty(),
			"the check missed: {what}"
		);
	}
	assert!(problems(&unit("[Container]\nImage=a\nEnvironment=A=%%h\n"), "c").is_empty());
	// A payload that stays inside a quoted argument is an argument.
	let quoted = unit("[Service]\nExecStart=/bin/sh -c \"x\" \"a\\\"; ExecStartPre=/bin/c\"\n");
	assert!(problems(&quoted, "c").is_empty());
}

/// `generate quadlet` stops, before it writes a unit, for a secret whose file is a
/// link (`up` refuses it too), and says which file and why.
#[cfg(unix)]
#[test]
fn a_link_where_a_secret_file_should_be_stops_generate_quadlet_before_any_unit() {
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(dir.path().join("real"), "x").unwrap();
	std::os::unix::fs::symlink(dir.path().join("real"), dir.path().join("link")).unwrap();
	let compose = dir.path().join("compose.yaml");
	std::fs::write(
		&compose,
		"services:\n  web:\n    image: img\n    secrets: [token]\nsecrets:\n  token:\n    file: ./link\n",
	)
	.unwrap();
	let out = dir.path().join("units");
	let run = std::process::Command::new(env!("CARGO_BIN_EXE_podup"))
		.args(["-f", compose.to_str().unwrap(), "generate", "quadlet", "-o"])
		.arg(&out)
		.output()
		.unwrap();
	assert!(!run.status.success());
	assert!(String::from_utf8_lossy(&run.stderr).contains("it is a link"));
	assert!(!out.exists());
	// A file that is not there yet is written.
	std::fs::write(
		&compose,
		std::fs::read_to_string(&compose)
			.unwrap()
			.replace("./link", "./later"),
	)
	.unwrap();
	let run = std::process::Command::new(env!("CARGO_BIN_EXE_podup"))
		.args(["-f", compose.to_str().unwrap(), "generate", "quadlet", "-o"])
		.arg(&out)
		.output()
		.unwrap();
	assert!(run.status.success());
	assert!(
		out.join("proj-podup-secrets.service").exists()
			|| std::fs::read_dir(&out).unwrap().count() > 1
	);
}
