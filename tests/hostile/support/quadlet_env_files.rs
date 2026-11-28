//! Round-trip env_file paths, and the file that carries `environment:` values,
//! through the installed Quadlet generator.

use std::{fs, process::Command};

use super::quadlet_cases::ENV_FILE_PATHS;

#[path = "quadlet_argv.rs"]
mod quadlet_argv;
#[path = "quadlet_generator.rs"]
mod quadlet_generator;
use quadlet_generator::generator;

// Decode the generated ExecStart, independently of podup's input serializer.
// Unknown escapes fail rather than silently changing the expected path.
fn exec_start_words(value: &str) -> Vec<String> {
	let mut words = Vec::new();
	let mut word = String::new();
	let mut quoted = false;
	let mut chars = value.chars();
	while let Some(c) = chars.next() {
		match c {
			'"' => quoted = !quoted,
			'\\' => word.push(match chars.next().expect("trailing ExecStart escape") {
				'\\' => '\\',
				'"' => '"',
				'n' => '\n',
				'r' => '\r',
				't' => '\t',
				's' => ' ',
				'x' => {
					let hex: String = chars.by_ref().take(2).collect();
					assert_eq!(hex.len(), 2, "short hex escape");
					char::from(u8::from_str_radix(&hex, 16).expect("hex escape"))
				}
				other => panic!("unsupported ExecStart escape: {other:?}"),
			}),
			c if c.is_ascii_whitespace() && !quoted => {
				if !word.is_empty() {
					words.push(std::mem::take(&mut word));
				}
			}
			c => word.push(c),
		}
	}
	assert!(!quoted, "unclosed ExecStart quote: {value}");
	if !word.is_empty() {
		words.push(word);
	}
	words
}

// systemd consumes these pairs at activation; an unpaired marker would expand.
fn literal_path(value: &str) -> String {
	let mut out = String::new();
	let mut chars = value.chars();
	while let Some(c) = chars.next() {
		if matches!(c, '%' | '$') {
			assert_eq!(chars.next(), Some(c), "unescaped marker in {value:?}");
		}
		out.push(c);
	}
	out
}

#[test]
fn env_file_paths_survive_the_real_quadlet_generator() {
	let Some(quadlet) = generator() else {
		return;
	};
	for name in ENV_FILE_PATHS {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join(name);
		fs::write(&path, "F2=value\n").unwrap();
		// Compose interpolation consumes $$ before the Quadlet serializer runs.
		let yaml = serde_json::json!({"services": {"web": {
			"image": "docker.io/library/alpine:latest",
			"network_mode": "host",
			"env_file": [name.replace('$', "$$")]
		}}});
		let compose = dir.path().join("compose.yaml");
		fs::write(&compose, yaml.to_string()).unwrap();
		let units = dir.path().join("units");
		let generated = Command::new(env!("CARGO_BIN_EXE_podup"))
			.args(["-p", "f2", "-f"])
			.arg(&compose)
			.args(["generate", "quadlet", "-o"])
			.arg(&units)
			.output()
			.unwrap();
		assert!(
			generated.status.success(),
			"generate {name:?}: {}",
			String::from_utf8_lossy(&generated.stderr)
		);
		let output = Command::new(&quadlet)
			.args(["-user", "-dryrun", "-no-kmsg-log"])
			.env("QUADLET_UNIT_DIRS", &units)
			.output()
			.expect("run the installed Quadlet generator");
		assert!(
			output.status.success(),
			"Quadlet {name:?}: {}",
			String::from_utf8_lossy(&output.stderr)
		);
		let stdout = String::from_utf8(output.stdout).unwrap();
		let commands: Vec<_> = stdout
			.lines()
			.filter_map(|line| line.strip_prefix("ExecStart="))
			.collect();
		assert_eq!(commands.len(), 1, "expected one container: {stdout}");
		let argv = exec_start_words(commands[0]);
		assert_eq!(
			argv.iter().filter(|arg| *arg == "--env-file").count(),
			1,
			"{name:?} must produce exactly one --env-file: {stdout}"
		);
		let arg = argv
			.windows(2)
			.find(|pair| pair[0] == "--env-file")
			.expect("--env-file needs a path");
		assert_eq!(
			literal_path(&arg[1]).as_bytes(),
			path.to_str().unwrap().as_bytes(),
			"path bytes changed for {name:?}: {stdout}"
		);
		eprintln!("Quadlet round-trip executed: {name:?}");
	}
}

/// Values `podman --env-file` has to give back byte for byte, under the names
/// `V0`, `V1` and so on.
const ENV_VALUES: &[&str] = &[
	"plain",
	" blanks around ",
	"\"double\" and 'single'",
	"$HOME ${HOME} %h %% # not a comment \\n",
	"a=b=c",
	"a\tb",
	"",
];

/// The values reach the container through the env file: the generated command
/// names the file after the service's own and holds none of them, and a
/// container started with those files sees each value as written. The values
/// stand for passwords, so no failure message holds one.
#[test]
fn environment_values_reach_the_container_through_a_file() {
	let Some(quadlet) = generator() else {
		return;
	};
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(dir.path().join("base.env"), "V0=from-env-file\nBASE=kept\n").unwrap();
	let mut environment = serde_json::Map::new();
	for (i, value) in ENV_VALUES.iter().enumerate() {
		// Compose interpolation consumes $$ before the exporter runs.
		environment.insert(format!("V{i}"), value.replace('$', "$$").into());
	}
	let yaml = serde_json::json!({"services": {"web": {
		"image": "docker.io/library/busybox:latest",
		"env_file": ["base.env"],
		"environment": environment,
	}}});
	let compose = dir.path().join("compose.yaml");
	fs::write(&compose, yaml.to_string()).unwrap();
	let units = dir.path().join("a dir");
	let generated = Command::new(env!("CARGO_BIN_EXE_podup"))
		.args(["-p", "envv", "-f"])
		.arg(&compose)
		.args(["generate", "quadlet", "-o"])
		.arg(&units)
		.output()
		.unwrap();
	assert!(generated.status.success());
	let output = Command::new(&quadlet)
		.args(["-user", "-dryrun", "-no-kmsg-log"])
		.env("QUADLET_UNIT_DIRS", &units)
		.output()
		.expect("run the installed Quadlet generator");
	assert!(output.status.success());
	let stdout = String::from_utf8(output.stdout).unwrap();
	let command = stdout
		.lines()
		.filter_map(|line| line.strip_prefix("ExecStart="))
		.find(|command| command.starts_with("/usr/bin/podman run "))
		.expect("one container");
	let argv = exec_start_words(command);
	assert!(
		!argv.iter().any(|a| a == "--env" || a.starts_with("V1=")),
		"a value is on the command line"
	);
	let files: Vec<String> = argv
		.windows(2)
		.filter(|pair| pair[0] == "--env-file")
		.map(|pair| literal_path(&pair[1]))
		.collect();
	assert_eq!(
		files,
		[
			dir.path().join("base.env").to_str().unwrap(),
			units.join("envv-web.podup.env").to_str().unwrap(),
		]
	);
	if Command::new("podman").arg("--version").output().is_err() {
		eprintln!("SKIP: podman is not installed");
		return;
	}
	let name = format!("t{}envv", std::process::id());
	let mut run = Command::new("podman");
	run.args(["run", "--rm", "--name", &name]);
	for file in &files {
		run.arg("--env-file").arg(file);
	}
	// NUL ends each value, so a value with blanks at its ends reads whole.
	let names: Vec<String> = (0..ENV_VALUES.len()).map(|i| format!("V{i}")).collect();
	let script = format!(
		r#"for n in {} BASE; do printenv "$n" | head -c -1; printf '\0'; done"#,
		names.join(" ")
	);
	let out = run
		.args(["docker.io/library/busybox:latest", "sh", "-c", &script])
		.output()
		.unwrap();
	assert!(
		out.status.success(),
		"podman run: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	let seen: Vec<&[u8]> = out.stdout.split(|b| *b == 0).collect();
	for (i, value) in ENV_VALUES.iter().enumerate() {
		assert!(
			seen[i] == value.as_bytes(),
			"the container reads V{i} wrong"
		);
	}
	assert!(
		seen[ENV_VALUES.len()] == b"kept",
		"the service's file is lost"
	);
}
