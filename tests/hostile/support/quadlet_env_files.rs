//! Round-trip env_file paths through the installed Quadlet generator.

use std::{fs, io::Write, path::Path, process::Command};

use super::quadlet_cases::ENV_FILE_PATHS;

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
	let quadlet = Path::new("/usr/libexec/podman/quadlet");
	match fs::metadata(quadlet) {
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
			writeln!(
				std::io::stderr(),
				"SKIP: /usr/libexec/podman/quadlet is absent; env_file round-trip was not executed"
			)
			.unwrap();
			return;
		}
		result => assert!(result.expect("inspect Quadlet binary").is_file()),
	}
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
		let output = Command::new(quadlet)
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
