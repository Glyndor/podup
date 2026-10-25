//! Values the user typed, and DEL or C1 controls in rendered config, must not
//! reach the terminal raw. No engine is involved: the socket does not exist.

#![cfg(unix)]

use std::process::Command;

const DEAD_SOCKET: &str = "unix:///nonexistent/podup-hostile-args.sock";
const PAYLOAD: &str = "A\u{1b}[2J\r\u{9b}2J\u{7f}B";

fn run(dir: &std::path::Path, args: &[&str]) -> String {
	let out = Command::new(env!("CARGO_BIN_EXE_podup"))
		.current_dir(dir)
		.args(["--socket", DEAD_SOCKET])
		.args(args)
		.env("NO_COLOR", "1")
		.env_remove("RUST_LOG")
		.stdin(std::process::Stdio::null())
		.output()
		.expect("run podup");
	format!(
		"{}\n{}",
		String::from_utf8_lossy(&out.stdout),
		String::from_utf8_lossy(&out.stderr)
	)
}

/// The first control character left once podup's own `ESC [ ... m` styling is
/// removed, so the label colours are not mistaken for the payload.
fn raw_control(text: &str) -> Option<char> {
	let mut plain = String::new();
	let mut chars = text.chars().peekable();
	while let Some(c) = chars.next() {
		if c == '\u{1b}' && chars.peek() == Some(&'[') {
			let mut seq = String::new();
			for n in chars.by_ref() {
				seq.push(n);
				if n.is_ascii_alphabetic() {
					break;
				}
			}
			let body = seq.get(1..seq.len().saturating_sub(1)).unwrap_or("");
			if seq.ends_with('m') && body.chars().all(|d| d.is_ascii_digit() || d == ';') {
				continue;
			}
			plain.push(c);
			plain.push_str(&seq);
		} else {
			plain.push(c);
		}
	}
	plain
		.chars()
		.find(|c| c.is_control() && *c != '\n' && *c != '\t')
}

fn project() -> tempfile::TempDir {
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(
		dir.path().join("compose.yaml"),
		"services:\n  web:\n    image: nginx\n",
	)
	.unwrap();
	dir
}

#[test]
fn config_json_escapes_del_and_c1_controls() {
	let dir = project();
	std::fs::write(
		dir.path().join("compose.yaml"),
		"services:\n  web:\n    image: nginx\n    container_name: \"a\\x7fb\\x9bc\\x85d\"\n",
	)
	.unwrap();
	let text = run(
		dir.path(),
		&["-f", "compose.yaml", "config", "--format", "json"],
	);
	assert!(text.contains("a\\u007fb\\u009bc\\u0085d"), "{text:?}");
	assert_eq!(raw_control(&text), None, "{text:?}");
}

#[test]
fn a_generate_quadlet_output_path_is_escaped_when_the_units_are_written() {
	// Under the temp dir, so the write succeeds for any user, root included.
	let dir = project();
	let target = format!("{}/{PAYLOAD}/out", dir.path().display());
	let text = run(
		dir.path(),
		&["-f", "compose.yaml", "generate", "quadlet", "-o", &target],
	);
	assert!(text.contains("wrote "), "{text:?}");
	assert!(text.contains("\\r\\u{9b}"), "{text:?}");
	assert_eq!(raw_control(&text), None, "{text:?}");
}

#[test]
fn a_missing_compose_file_path_is_escaped_in_its_error() {
	let dir = project();
	let file = format!("{PAYLOAD}.yaml");
	let text = run(dir.path(), &["-f", &file, "config"]);
	assert_eq!(raw_control(&text), None, "{text:?}");
}

#[test]
fn an_unknown_service_name_is_escaped_in_its_error() {
	let dir = project();
	let text = run(dir.path(), &["-f", "compose.yaml", "config", PAYLOAD]);
	assert_eq!(raw_control(&text), None, "{text:?}");
}
