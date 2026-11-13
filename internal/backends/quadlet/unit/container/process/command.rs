//! The process the container runs: its entrypoint and its arguments.

use crate::compose::types::{Command, Service};

use super::super::{render_command, Section};

/// Emit the `Entrypoint=` and `Exec=` lines of the service.
pub(in super::super) fn emit_command(service: &Service, container: &mut Section) {
	if let Some(entrypoint) = &service.entrypoint {
		container.add("Entrypoint", render_entrypoint(entrypoint));
	}
	if let Some(command) = &service.command {
		container.add("Exec", render_command(command));
	}
}

/// Quadlet hands the whole `Entrypoint=` value to `podman run --entrypoint` as
/// one argument, so `/bin/sh -c` became a single executable named `/bin/sh -c`.
/// Podman reads a JSON array there as the vector, which is what `up` sends. The
/// value reaches `ExecStart=` as written, so a `%` is doubled for systemd, as
/// `Exec=` does.
fn render_entrypoint(entrypoint: &Command) -> String {
	match entrypoint {
		Command::Exec(parts) if parts.len() > 1 => {
			let words: Vec<String> = parts.iter().map(|p| json_string(p)).collect();
			format!("[{}]", words.join(","))
		}
		other => render_command(other),
	}
}

/// One JSON string, with `%` doubled for systemd. Written by hand because a
/// list of strings has nothing that can fail to encode.
fn json_string(word: &str) -> String {
	let mut out = String::with_capacity(word.len() + 2);
	out.push('"');
	for c in word.chars() {
		match c {
			'"' => out.push_str("\\\""),
			'\\' => out.push_str("\\\\"),
			'%' => out.push_str("%%"),
			c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
			c => out.push(c),
		}
	}
	out.push('"');
	out
}
