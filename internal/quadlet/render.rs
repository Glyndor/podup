//! Render compose values into Quadlet directive strings, and the `[Section]`
//! accumulator plus the sanitizing helpers shared across the generator.

use std::collections::BTreeMap;

use crate::compose::types::{Command, RestartPolicy, VolumeMount, VolumeType};
use crate::ports::ParsedPort;

/// Render a parsed port into a Quadlet `PublishPort=` value
/// (`[host_ip:][host_port:]container[/proto]`). A missing or `0` host port is
/// omitted so Podman picks one; the protocol suffix is only added when it is not
/// the default `tcp`.
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

/// Render a volume mount into a Quadlet `Volume=` value. A source naming a
/// declared volume gets a project-prefixed `.volume` suffix (so Quadlet wires it
/// to the generated unit, whose file name is also project-prefixed); an
/// undeclared source passes through verbatim. An empty source renders as just the
/// target. Long-form `read_only`, SELinux relabel (`z`/`Z`), bind propagation,
/// and `nocopy` opts are folded into the trailing `:opt,opt` field.
pub(super) fn render_volume(vol: &VolumeMount, project: &str, declared_volumes: &[&str]) -> String {
	match vol {
		VolumeMount::Short(s) => {
			let parts: Vec<&str> = s.splitn(3, ':').collect();
			if parts.len() >= 2 && declared_volumes.contains(&parts[0]) {
				let mut out = format!("{}.volume:{}", unit_stem(project, parts[0]), parts[1]);
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
				format!("{}.volume", unit_stem(project, &src))
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

/// If `vol` is a long-form `type: tmpfs` mount, render it as a Quadlet `Tmpfs=`
/// value (`target[:size=…,mode=…]`); otherwise return `None` so the caller emits
/// a normal `Volume=`. A tmpfs mount written as `Volume=` would create a
/// persistent named/anonymous volume instead of an in-memory filesystem — a
/// silent semantic inversion. Size/mode mirror the runtime mount options.
pub(super) fn render_tmpfs_mount(vol: &VolumeMount) -> Option<String> {
	let VolumeMount::Long {
		volume_type: VolumeType::Tmpfs,
		target,
		tmpfs,
		..
	} = vol
	else {
		return None;
	};
	let mut opts: Vec<String> = Vec::new();
	if let Some(t) = tmpfs {
		if let Some(size) = t.size {
			opts.push(format!("size={size}"));
		}
		if let Some(mode) = t.mode {
			opts.push(format!("mode={mode:o}"));
		}
	}
	if opts.is_empty() {
		Some(target.clone())
	} else {
		Some(format!("{target}:{}", opts.join(",")))
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

/// Map a compose `restart:` policy onto a systemd `Restart=` value.
/// `unless-stopped` has no systemd equivalent and is treated as `always`.
pub(super) fn render_restart(restart: &RestartPolicy) -> String {
	match restart {
		RestartPolicy::No => "no".to_string(),
		RestartPolicy::Always => "always".to_string(),
		RestartPolicy::UnlessStopped => "always".to_string(),
		RestartPolicy::OnFailure { .. } => "on-failure".to_string(),
	}
}

/// Sort a map of optional-valued entries by key into a stable `Vec`, so the
/// generated unit is byte-deterministic regardless of `HashMap` iteration order.
pub(super) fn sorted_pairs(
	map: std::collections::HashMap<String, Option<String>>,
) -> Vec<(String, Option<String>)> {
	let sorted: BTreeMap<_, _> = map.into_iter().collect();
	sorted.into_iter().collect()
}

/// Sort a map of string-valued entries (labels, sysctls, …) by key into a stable
/// `Vec`, so the generated unit is byte-deterministic regardless of `HashMap`
/// iteration order.
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

/// Keys whose value Quadlet/systemd splits on whitespace (a `KEY=VALUE` option
/// list): an unquoted space would split a single value into a truncated value
/// plus bogus extra entries, so such values are quoted when they contain
/// whitespace.
fn key_is_word_split(key: &str) -> bool {
	matches!(key, "Environment" | "Label" | "Annotation" | "Sysctl")
}

/// Keys whose value is an argument line we render (and quote) ourselves
/// (`PodmanArgs`, `Exec`, `Entrypoint`); systemd must keep splitting these on
/// whitespace, so they are passed through with only control characters stripped.
fn key_is_arg_line(key: &str) -> bool {
	matches!(key, "PodmanArgs" | "Exec" | "Entrypoint")
}

/// Escape a value for a single-line systemd `Key=Value` entry.
///
/// On top of [`sanitize_value`] (control characters stripped so a value can
/// never inject a second directive) this makes the value byte-faithful to the
/// compose source the way docker-compose treats it:
///
/// * systemd specifiers (`%h`, `%U`, …) are passed through literally by doubling
///   `%` to `%%`, instead of being expanded at unit-activation time;
/// * a value ending in a backslash — which would otherwise fold the next
///   physical line (and the directive on it) into this value — is quoted and the
///   backslash escaped, closing the line-continuation hole;
/// * for word-split keys (`Environment`, `Label`, …) a value containing
///   whitespace is double-quoted so systemd keeps it as one value rather than
///   splitting it into bogus extra entries.
///
/// Argument-line keys keep their own rendering (they encode whitespace splitting
/// deliberately) and only have control characters stripped.
pub(super) fn escape_unit_value(key: &str, value: &str) -> String {
	let stripped = sanitize_value(value);
	if key_is_arg_line(key) {
		return stripped;
	}
	let escaped = stripped.replace('%', "%%");
	let needs_quote = (key_is_word_split(key) && escaped.contains(char::is_whitespace))
		|| escaped.ends_with('\\')
		|| escaped.starts_with('"');
	if needs_quote {
		format!("\"{}\"", escaped.replace('\\', "\\\\").replace('"', "\\\""))
	} else {
		escaped
	}
}

/// Build the project-prefixed unit-file stem for a resource, e.g. service `web`
/// in project `proj` → `proj-web`. Generated unit files share a single systemd
/// directory across projects, so the stem (and every in-unit cross-reference to
/// it) carries the project name — matching the project-scoped resource names
/// inside the units — so two projects' `web` services do not clobber each other.
pub(super) fn unit_stem(project: &str, name: &str) -> String {
	safe_unit_stem(&format!("{project}-{name}"))
}

/// Reduce a compose key to a safe single path-component stem for a unit file
/// name. Keeps ASCII alphanumerics and `-`/`_`/`.`; replaces anything else
/// (path separators, control characters) with `_`, and guarantees the result
/// is non-empty and starts with neither a dot nor a dash, so it can never escape
/// the output directory, resolve to `.`/`..`, or be mistaken for a command-line
/// flag by downstream tooling that globs the generated file names.
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
	if stem.is_empty() || stem.starts_with('.') || stem.starts_with('-') {
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
	/// Start an empty section with the given `[name]` header.
	pub(super) fn new(name: &'static str) -> Self {
		Section {
			name,
			lines: Vec::new(),
		}
	}

	/// Append a `Key=Value` line, escaping the value for systemd unit syntax
	/// (control characters stripped, specifiers passed through literally,
	/// whitespace/line-continuation hazards quoted) so a hostile or merely
	/// awkward compose field cannot inject extra directives, split a value, or
	/// swallow the following line. See [`escape_unit_value`].
	pub(super) fn add(&mut self, key: &str, value: String) {
		self.lines
			.push(format!("{key}={}", escape_unit_value(key, &value)));
	}

	/// True when no lines have been added; lets the caller skip emitting an empty
	/// section.
	pub(super) fn is_empty(&self) -> bool {
		self.lines.is_empty()
	}

	/// Render the section as `[name]` followed by its lines, each newline-terminated.
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

	// --- render_publish_port ---

	#[test]
	fn render_publish_port_full_and_partial_forms() {
		let port = |host_ip: &str, host_port: Option<u16>, cp: u16, proto: &str| ParsedPort {
			container_port: cp,
			protocol: proto.to_string(),
			host_ip: host_ip.to_string(),
			host_port,
		};
		// ip + host port + container port, default tcp omits the protocol suffix.
		assert_eq!(
			render_publish_port(&port("127.0.0.1", Some(8080), 80, "tcp")),
			"127.0.0.1:8080:80"
		);
		// No ip, no host port (runtime-assigned) → bare container port.
		assert_eq!(render_publish_port(&port("", None, 80, "tcp")), "80");
		// A non-tcp protocol is appended; a 0 host port is treated as "let Podman pick".
		assert_eq!(render_publish_port(&port("", Some(0), 53, "udp")), "53/udp");
	}

	// --- render_volume ---

	#[test]
	fn render_volume_short_declared_uses_dot_volume_with_options() {
		let out = render_volume(
			&VolumeMount::Short("data:/app:ro".into()),
			"proj",
			&["data"],
		);
		// A declared named volume becomes `<project>-<name>.volume:<target>:<opts>`.
		assert_eq!(out, "proj-data.volume:/app:ro");
		// An undeclared source is passed through verbatim.
		assert_eq!(
			render_volume(&VolumeMount::Short("./host:/app".into()), "proj", &["data"]),
			"./host:/app"
		);
	}

	#[test]
	fn render_volume_long_collects_options_and_handles_empty_source() {
		use crate::compose::types::VolumeOptions;
		let nocopy = VolumeMount::Long {
			volume_type: VolumeType::Volume,
			source: Some("vol".into()),
			target: "/data".into(),
			read_only: Some(true),
			bind: None,
			volume: Some(VolumeOptions {
				nocopy: Some(true),
				..Default::default()
			}),
			tmpfs: None,
			consistency: None,
		};
		// Declared → project-prefixed `.volume` suffix; ro + nocopy folded into
		// the options field.
		assert_eq!(
			render_volume(&nocopy, "proj", &["vol"]),
			"proj-vol.volume:/data:ro,nocopy"
		);

		// An empty source renders as just the target (anonymous mount).
		let anon = VolumeMount::Long {
			volume_type: VolumeType::Volume,
			source: None,
			target: "/scratch".into(),
			read_only: None,
			bind: None,
			volume: None,
			tmpfs: None,
			consistency: None,
		};
		assert_eq!(render_volume(&anon, "proj", &[]), "/scratch");
	}

	// --- render_tmpfs_mount ---

	#[test]
	fn render_tmpfs_mount_with_and_without_options() {
		use crate::compose::types::TmpfsOptions;
		let with_opts = VolumeMount::Long {
			volume_type: VolumeType::Tmpfs,
			source: None,
			target: "/cache".into(),
			read_only: None,
			bind: None,
			volume: None,
			tmpfs: Some(TmpfsOptions {
				size: Some(4096),
				mode: Some(0o700),
			}),
			consistency: None,
		};
		assert_eq!(
			render_tmpfs_mount(&with_opts).as_deref(),
			Some("/cache:size=4096,mode=700")
		);

		// A tmpfs mount with no size/mode renders just the target.
		let bare = VolumeMount::Long {
			volume_type: VolumeType::Tmpfs,
			source: None,
			target: "/run".into(),
			read_only: None,
			bind: None,
			volume: None,
			tmpfs: None,
			consistency: None,
		};
		assert_eq!(render_tmpfs_mount(&bare).as_deref(), Some("/run"));

		// A non-tmpfs mount returns None so the caller emits a normal Volume=.
		assert!(render_tmpfs_mount(&VolumeMount::Short("a:/b".into())).is_none());
	}

	#[test]
	fn render_tmpfs_mount_bare_decimal_mode_is_not_octal_re_encoded() {
		// Regression for #917: a long-form tmpfs with a bare `mode: 700` must
		// render `mode=700`, not `mode=1274` (700 octal-encoded a second time).
		let yaml = "type: tmpfs\ntarget: /run\ntmpfs:\n  mode: 700\n";
		let mount: VolumeMount = serde_yaml::from_str(yaml).expect("parse tmpfs mount");
		assert_eq!(render_tmpfs_mount(&mount).as_deref(), Some("/run:mode=700"));
	}

	// --- escape_unit_value (bug: incomplete systemd value escaping) ---

	#[test]
	fn escape_word_split_value_with_whitespace_is_quoted() {
		// An Environment value with whitespace must be quoted so systemd keeps it
		// as one value instead of splitting it into bogus extra entries.
		assert_eq!(
			escape_unit_value("Environment", "JAVA_OPTS=-Xmx512m -Xms256m"),
			"\"JAVA_OPTS=-Xmx512m -Xms256m\""
		);
		// A Label is word-split too.
		assert_eq!(
			escape_unit_value("Label", "note=hello world"),
			"\"note=hello world\""
		);
		// A scalar key that is not word-split keeps whitespace unquoted.
		assert_eq!(
			escape_unit_value("Description", "web (podup)"),
			"web (podup)"
		);
	}

	#[test]
	fn escape_trailing_backslash_is_quoted_and_escaped() {
		// A value ending in a backslash would otherwise continue onto — and
		// swallow — the next directive line; it must be quoted and escaped.
		let out = escape_unit_value("Environment", "WINPATH=C:\\tmp\\");
		assert!(!out.ends_with('\\') || out.ends_with("\\\""));
		assert_eq!(out, "\"WINPATH=C:\\\\tmp\\\\\"");
	}

	#[test]
	fn escape_percent_is_doubled_for_literal() {
		// systemd specifiers like %h must be passed through literally, not
		// expanded at unit-activation time, matching docker-compose semantics.
		assert_eq!(escape_unit_value("Environment", "HOME=%h"), "HOME=%%h");
		assert_eq!(escape_unit_value("Image", "img%U"), "img%%U");
	}

	#[test]
	fn escape_arg_line_keys_are_left_intact() {
		// PodmanArgs/Exec/Entrypoint encode their own whitespace splitting and
		// must not be quoted or have `%` doubled.
		assert_eq!(
			escape_unit_value("PodmanArgs", "--security-opt apparmor=foo"),
			"--security-opt apparmor=foo"
		);
		assert_eq!(
			escape_unit_value("Exec", "sh -c \"echo %s\""),
			"sh -c \"echo %s\""
		);
	}

	#[test]
	fn safe_unit_stem_strips_leading_dash() {
		// A name starting with `-`/`--` must not yield a file name beginning with
		// a dash (a globbing/flag-injection hazard for downstream tooling).
		assert_eq!(safe_unit_stem("--foo"), "_--foo");
		assert_eq!(safe_unit_stem("-x"), "_-x");
		assert!(!safe_unit_stem("--foo").starts_with('-'));
	}

	#[test]
	fn unit_stem_is_project_prefixed() {
		assert_eq!(unit_stem("proj", "web"), "proj-web");
		// A leading dash in the project still cannot produce a dash-leading stem.
		assert!(!unit_stem("-p", "web").starts_with('-'));
	}
}
