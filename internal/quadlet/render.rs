//! Render compose values into Quadlet directive strings, and the `[Section]`
//! accumulator plus the sanitizing helpers shared across the generator.

use std::collections::BTreeMap;

use crate::compose::types::{Command, RestartPolicy, VolumeMount};
use crate::ports::ParsedPort;

pub(super) fn render_publish_port(p: &ParsedPort) -> String {
	let mut s = String::new();
	if !p.host_ip.is_empty() {
		s.push_str(&p.host_ip);
		s.push(':');
	}
	// A host port of None/0 means "let Podman pick"; omit it so the published
	// side is empty (PublishPort=<container>) and Podman assigns a port.
	if let Some(host) = p.host_port.filter(|n| *n != 0) {
		s.push_str(&host.to_string());
		s.push(':');
	}
	s.push_str(&p.container_port.to_string());
	if p.protocol != "tcp" {
		s.push('/');
		s.push_str(&p.protocol);
	}
	s
}

pub(super) fn render_volume(vol: &VolumeMount, declared_volumes: &[&str]) -> String {
	match vol {
		VolumeMount::Short(s) => {
			let parts: Vec<&str> = s.splitn(3, ':').collect();
			if parts.len() >= 2 && declared_volumes.contains(&parts[0]) {
				let mut out = format!("{}.volume:{}", parts[0], parts[1]);
				if let Some(opts) = parts.get(2) {
					out.push(':');
					out.push_str(opts);
				}
				out
			} else {
				s.clone()
			}
		}
		VolumeMount::Long {
			source,
			target,
			read_only,
			bind,
			volume,
			..
		} => {
			let src = source.clone().unwrap_or_default();
			let src = if declared_volumes.contains(&src.as_str()) {
				format!("{src}.volume")
			} else {
				src
			};
			// Collect mount options into the trailing `:opt,opt` field so nothing
			// is dropped: SELinux relabel (`z`/`Z`) and bind propagation are
			// security- and correctness-relevant on hardened hosts.
			let mut opts: Vec<String> = Vec::new();
			if *read_only == Some(true) {
				opts.push("ro".to_string());
			}
			if let Some(b) = bind {
				if let Some(selinux) = &b.selinux {
					opts.push(selinux.clone());
				}
				if let Some(propagation) = &b.propagation {
					opts.push(propagation.clone());
				}
			}
			if let Some(v) = volume {
				if v.nocopy == Some(true) {
					opts.push("nocopy".to_string());
				}
			}
			let mut out = if src.is_empty() {
				target.clone()
			} else {
				format!("{src}:{target}")
			};
			if !opts.is_empty() {
				out.push(':');
				out.push_str(&opts.join(","));
			}
			out
		}
	}
}

/// Render a compose `command:`/`entrypoint:` into a single-line systemd `Exec=`
/// value. systemd splits the value on whitespace honouring double-quoted groups
/// and C-style escapes, so each list element is quoted when it contains
/// whitespace or quoting characters, and control characters are always escaped.
/// Without this a multi-line argument (e.g. an `sh -c` block scalar) would emit
/// raw newlines that spill onto extra physical lines and corrupt the unit file.
pub(super) fn render_command(command: &Command) -> String {
	match command {
		// A shell-form command is already a single command line; keep its word
		// splitting but neutralise control characters so it stays on one line.
		Command::Shell(s) => escape_exec_control(s),
		Command::Exec(parts) => parts
			.iter()
			.map(|p| quote_exec_arg(p))
			.collect::<Vec<_>>()
			.join(" "),
	}
}

/// C-escape the characters that must never appear raw in a systemd command line.
fn escape_exec_control(s: &str) -> String {
	let mut out = String::with_capacity(s.len());
	for c in s.chars() {
		match c {
			'\\' => out.push_str("\\\\"),
			'\n' => out.push_str("\\n"),
			'\r' => out.push_str("\\r"),
			'\t' => out.push_str("\\t"),
			_ => out.push(c),
		}
	}
	out
}

/// Quote a single `Exec=` argument: wrap it in double quotes (escaping `"` and
/// control characters) when it is empty or contains whitespace or quoting
/// characters, so systemd keeps it as one argument. Plain arguments pass through
/// unchanged.
fn quote_exec_arg(arg: &str) -> String {
	let needs_quoting = arg.is_empty()
		|| arg
			.chars()
			.any(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '\\' | '$' | '%' | ';'));
	if needs_quoting {
		format!("\"{}\"", escape_exec_control(arg).replace('"', "\\\""))
	} else {
		arg.to_string()
	}
}

pub(super) fn render_restart(restart: &RestartPolicy) -> String {
	match restart {
		RestartPolicy::No => "no".to_string(),
		RestartPolicy::Always => "always".to_string(),
		RestartPolicy::UnlessStopped => "always".to_string(),
		RestartPolicy::OnFailure { .. } => "on-failure".to_string(),
	}
}

pub(super) fn sorted_pairs(
	map: std::collections::HashMap<String, Option<String>>,
) -> Vec<(String, Option<String>)> {
	let sorted: BTreeMap<_, _> = map.into_iter().collect();
	sorted.into_iter().collect()
}

pub(super) fn sorted_label_pairs(
	map: std::collections::HashMap<String, String>,
) -> Vec<(String, String)> {
	let sorted: BTreeMap<_, _> = map.into_iter().collect();
	sorted.into_iter().collect()
}

/// Strip ASCII control characters from a value before it is written into a
/// unit file. systemd unit entries are single-line `Key=Value` pairs; an
/// embedded newline from a hostile compose field would otherwise inject
/// arbitrary unit directives (e.g. a `[Service]` `ExecStartPre=`). Compose
/// input is untrusted, so every dynamic value is sanitized at the boundary.
pub(super) fn sanitize_value(value: &str) -> String {
	value.chars().filter(|c| !c.is_control()).collect()
}

/// Reduce a compose key to a safe single path-component stem for a unit file
/// name. Keeps ASCII alphanumerics and `-`/`_`/`.`; replaces anything else
/// (path separators, control characters) with `_`, and guarantees the result
/// is non-empty and does not start with a dot, so it can never escape the
/// output directory or resolve to `.`/`..`.
pub(super) fn safe_unit_stem(name: &str) -> String {
	let mut stem: String = name
		.chars()
		.map(|c| {
			if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
				c
			} else {
				'_'
			}
		})
		.collect();
	if stem.is_empty() || stem.starts_with('.') {
		stem.insert(0, '_');
	}
	stem
}

/// A single `[Section]` accumulating `Key=Value` lines in insertion order.
pub(super) struct Section {
	name: &'static str,
	lines: Vec<String>,
}

impl Section {
	pub(super) fn new(name: &'static str) -> Self {
		Section {
			name,
			lines: Vec::new(),
		}
	}

	pub(super) fn add(&mut self, key: &str, value: String) {
		self.lines.push(format!("{key}={}", sanitize_value(&value)));
	}

	pub(super) fn is_empty(&self) -> bool {
		self.lines.is_empty()
	}

	pub(super) fn render(&self) -> String {
		let mut s = format!("[{}]\n", self.name);
		for line in &self.lines {
			s.push_str(line);
			s.push('\n');
		}
		s
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn safe_unit_stem_neutralizes_traversal_and_control_chars() {
		assert_eq!(safe_unit_stem("web"), "web");
		assert_eq!(safe_unit_stem("db-data_1.x"), "db-data_1.x");
		assert_eq!(safe_unit_stem("../../etc/passwd"), "_.._.._etc_passwd");
		assert_eq!(safe_unit_stem("/abs"), "_abs");
		assert_eq!(safe_unit_stem(".hidden"), "_.hidden");
		assert_eq!(safe_unit_stem(""), "_");
		assert!(!safe_unit_stem("a\nb").contains('\n'));
	}

	#[test]
	fn sanitize_value_strips_control_characters() {
		assert_eq!(sanitize_value("plain"), "plain");
		assert_eq!(sanitize_value("a\nb\tc\r"), "abc");
	}

	#[test]
	fn render_command_exec_quotes_args_with_whitespace() {
		let cmd = Command::Exec(vec![
			"server".to_string(),
			"--port".to_string(),
			"9000".to_string(),
		]);
		// Plain arguments pass through unquoted.
		assert_eq!(render_command(&cmd), "server --port 9000");

		let cmd = Command::Exec(vec!["echo".to_string(), "hello world".to_string()]);
		assert_eq!(render_command(&cmd), "echo \"hello world\"");
	}

	#[test]
	fn render_command_exec_multiline_arg_stays_one_line() {
		// A multi-line block-scalar argument (e.g. an `sh -c` script) must be
		// quoted with its newlines C-escaped so the rendered Exec= never spills
		// onto a second physical line or mashes adjacent tokens together.
		let cmd = Command::Exec(vec![
			"sh".to_string(),
			"-c".to_string(),
			"mkdir -p /www\necho hi > /www/index.html\nexec httpd".to_string(),
		]);
		let out = render_command(&cmd);
		assert!(
			!out.contains('\n'),
			"rendered Exec must be a single line: {out}"
		);
		assert_eq!(
			out,
			"sh -c \"mkdir -p /www\\necho hi > /www/index.html\\nexec httpd\""
		);
	}

	#[test]
	fn render_command_exec_escapes_quotes_and_backslashes() {
		let cmd = Command::Exec(vec![
			"sh".to_string(),
			"-c".to_string(),
			"printf '%s' \"a\\b\"".to_string(),
		]);
		let out = render_command(&cmd);
		assert!(!out.contains('\n'));
		// The embedded double quotes and backslash are escaped inside the quoted
		// argument.
		assert!(out.starts_with("sh -c \""));
		assert!(out.contains("\\\""));
		assert!(out.contains("\\\\"));
	}

	#[test]
	fn render_command_shell_neutralizes_newlines() {
		let cmd = Command::Shell("echo a\necho b".to_string());
		let out = render_command(&cmd);
		assert!(!out.contains('\n'));
		assert_eq!(out, "echo a\\necho b");

		// A plain single-line shell command is unchanged.
		assert_eq!(
			render_command(&Command::Shell("echo hi".to_string())),
			"echo hi"
		);
	}
}
