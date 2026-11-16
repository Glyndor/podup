//! Every Linux job takes its runner from the one repository variable `CI_RUNNER`.
//!
//! A job pinned to `ubuntu-latest` or `ubuntu-24.04` ignores the variable, so a
//! self-hosted runner set through it would quietly leave that job on a GitHub
//! one. This fails when a Linux `runs-on` is written as the fixed literal again,
//! unless the file and literal are in the list below with the reason.

use std::fs;
use std::path::Path;

/// `(file, literal, why it stays on a GitHub-hosted runner)`.
const EXCEPTIONS: [(&str, &str, &str); 3] = [
	(
		"release.yml",
		"ubuntu-latest",
		"it builds, signs and publishes the release: never on a runner the owner may remove",
	),
	(
		"podman-lane.yml",
		"ubuntu-24.04",
		"the VM legs boot QEMU with /dev/kvm and sudo, and are a required check",
	),
	(
		"podman-lane-develop-nightly.yml",
		"ubuntu-24.04",
		"the same VM legs, run nightly against develop",
	),
];

const FIXED: [&str; 2] = ["ubuntu-latest", "ubuntu-24.04"];

fn workflows() -> Vec<(String, String)> {
	let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows");
	let mut out = Vec::new();
	for entry in fs::read_dir(&dir).expect("the workflows directory is readable") {
		let path = entry.expect("a directory entry").path();
		if path.extension().is_some_and(|e| e == "yml") {
			let name = path.file_name().unwrap().to_string_lossy().into_owned();
			out.push((
				name,
				fs::read_to_string(&path).expect("a workflow is readable"),
			));
		}
	}
	out.sort();
	out
}

/// One line per Linux `runs-on` written as a fixed literal outside the list.
fn problems(files: &[(String, String)]) -> Vec<String> {
	let mut out = Vec::new();
	for (name, text) in files {
		for (number, line) in text.lines().enumerate() {
			let code = line.split('#').next().unwrap_or("").trim();
			let Some(value) = code.strip_prefix("runs-on:") else {
				continue;
			};
			let value = value.trim();
			if FIXED.contains(&value)
				&& !EXCEPTIONS.iter().any(|(f, l, _)| f == name && *l == value)
			{
				out.push(format!(
					"{name}:{}: runs-on: {value} ignores CI_RUNNER; write \
					 runs-on: ${{{{ fromJSON(vars.CI_RUNNER || '\"ubuntu-latest\"') }}}}",
					number + 1
				));
			}
		}
	}
	out
}

#[test]
fn no_linux_job_is_pinned_to_a_github_runner() {
	let found = problems(&workflows());
	assert!(found.is_empty(), "\n{}\n", found.join("\n"));
}

#[test]
fn the_exceptions_still_name_a_job_that_needs_them() {
	// A stale entry would let a fixed runner back in unnoticed.
	let files = workflows();
	for (file, literal, why) in EXCEPTIONS {
		let text = &files
			.iter()
			.find(|(name, _)| name == file)
			.unwrap_or_else(|| panic!("{file} is listed for {why} but does not exist"))
			.1;
		assert!(
			text.lines()
				.any(|l| l.split('#').next().unwrap_or("").trim() == format!("runs-on: {literal}")),
			"{file} no longer pins {literal}; drop the exception ({why})"
		);
	}
}

#[test]
fn a_pinned_runner_is_reported() {
	let files = vec![(
		"example.yml".to_string(),
		"jobs:\n  a:\n    runs-on: ubuntu-latest\n".to_string(),
	)];
	assert_eq!(problems(&files).len(), 1);
	let ok = vec![(
		"example.yml".to_string(),
		"jobs:\n  a:\n    runs-on: ${{ fromJSON(vars.CI_RUNNER || '\"ubuntu-latest\"') }}\n"
			.to_string(),
	)];
	assert!(problems(&ok).is_empty());
}
