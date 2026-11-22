//! Bind paths and command arguments must survive the real generator unchanged.

use std::{fs, process::Command};

use super::quadlet_generator::generator;
use super::{exec_start_words, literal_path};

/// The value of `flag`, read from either spelling the generator writes: as
/// two words (`-v`, `src:dst`) or as one word joined with `=`
/// (`--entrypoint=[...]`, which is how Podman 6.2.0 writes it).
fn option(argv: &[String], flag: &str) -> String {
	let joined = format!("{flag}=");
	let values: Vec<String> = argv
		.iter()
		.enumerate()
		.filter_map(|(i, arg)| {
			if arg == flag {
				Some(argv[i + 1].clone())
			} else {
				arg.strip_prefix(&joined).map(str::to_string)
			}
		})
		.collect();
	assert_eq!(values.len(), 1, "{flag} must appear once: {argv:?}");
	values.into_iter().next().unwrap()
}

#[test]
fn bind_and_command_bytes_survive_the_real_quadlet_generator_or_are_refused() {
	let Some(quadlet) = generator() else {
		return;
	};
	// Exercise quoting even if podup refuses the colon-bearing bind afterwards.
	for payload in [
		r#"%h $HOME "quote" 'single' \ space"#,
		r#"%h $HOME "quote" 'single' \ : space"#,
	] {
		let dir = tempfile::tempdir().unwrap();
		let source = dir.path().join(format!("source {payload}"));
		fs::create_dir(&source).unwrap();
		let source = source.to_str().unwrap();
		let target = format!("/target {payload}");
		let command = [format!("command {payload}"), format!("last {payload}\\")];
		let entrypoint = [
			format!("/entrypoint {payload}"),
			format!("entry-arg {payload}"),
		];
		// JSON is valid YAML; $$ protects literal dollars from Compose interpolation.
		let escape = |s: &str| s.replace('$', "$$");
		let yaml = serde_json::json!({"services": {"web": {
			"image": "docker.io/library/alpine:latest", "network_mode": "host",
			"volumes": [{"type": "bind", "source": escape(source), "target": escape(&target)}],
			"command": command.iter().map(|s| escape(s)).collect::<Vec<_>>(),
			"entrypoint": entrypoint.iter().map(|s| escape(s)).collect::<Vec<_>>()
		}}});
		let compose = dir.path().join("compose.yaml");
		fs::write(&compose, yaml.to_string()).unwrap();
		let units = dir.path().join("units");
		let generated = Command::new(env!("CARGO_BIN_EXE_podup"))
			.args(["-p", "argv", "-f"])
			.arg(&compose)
			.args(["generate", "quadlet", "-o"])
			.arg(&units)
			.output()
			.unwrap();
		if !generated.status.success() {
			assert_eq!(
				generated.status.code(),
				Some(1),
				"podup crashed: {generated:?}"
			);
			assert!(!generated.stderr.is_empty(), "refusal needs a diagnostic");
			assert!(!units.exists() || fs::read_dir(&units).unwrap().next().is_none());
			eprintln!("podup refused bind/argv fixture {payload:?}");
			continue;
		}
		let output = Command::new(&quadlet)
			.args(["-user", "-dryrun", "-no-kmsg-log"])
			.env("QUADLET_UNIT_DIRS", &units)
			.output()
			.expect("run the installed Quadlet generator");
		assert!(
			output.status.success(),
			"Quadlet rejected exported input: {output:?}"
		);
		let stdout = String::from_utf8(output.stdout).unwrap();
		let commands: Vec<_> = stdout
			.lines()
			.filter_map(|s| s.strip_prefix("ExecStart="))
			.collect();
		assert_eq!(commands.len(), 1, "expected one container: {stdout}");
		let argv = exec_start_words(commands[0]);
		let image = argv
			.iter()
			.position(|s| s == "docker.io/library/alpine:latest")
			.unwrap();
		let recovered: Vec<_> = argv[image + 1..].iter().map(|s| literal_path(s)).collect();
		assert_eq!(recovered, command, "command bytes changed: {stdout}");
		let recovered: Vec<String> =
			serde_json::from_str(&literal_path(&option(&argv, "--entrypoint"))).unwrap();
		assert_eq!(recovered, entrypoint, "entrypoint bytes changed: {stdout}");
		let mount = literal_path(&option(&argv, "-v"));
		// Decode Podman's colon-delimited mount, rather than comparing its text.
		// A colon inside either path must be refused or represented losslessly.
		let fields: Vec<_> = mount.split(':').collect();
		assert!(matches!(fields.len(), 2 | 3), "ambiguous mount: {mount:?}");
		assert_eq!(fields[0], source, "bind source changed: {stdout}");
		assert_eq!(fields[1], target, "bind target changed: {stdout}");
		eprintln!("Quadlet bind/argv round-trip executed: {payload:?}");
	}
}
