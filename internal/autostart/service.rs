//! Pure rendering of the `podup-<project>.service` systemd user unit.
//!
//! The unit is a `Type=oneshot` `RemainAfterExit=yes` service that runs `podup
//! ... up -d --build` at boot and `podup ... down` on stop. systemd has no cwd
//! and no relative-path context, so every path the unit embeds is absolute and
//! every argument is escaped per the systemd exec-line syntax.

use std::path::PathBuf;

/// Inputs to render a service-mode autostart unit. Every path must be absolute:
/// systemd resolves the `ExecStart`/`ExecStop` lines with no working directory of
/// its own, so a relative path would be interpreted against `/`.
pub struct ServiceUnitOpts {
	/// Absolute path to the `podup` executable.
	pub exe: PathBuf,
	/// Absolute compose-file paths, in `-f` order (later overrides earlier).
	pub compose_files: Vec<PathBuf>,
	/// Project name (already validated as a safe path component).
	pub project: String,
	/// Absolute working directory (the project base directory).
	pub working_dir: PathBuf,
	/// Active profiles, passed through as `--profile` flags.
	pub profiles: Vec<String>,
	/// Extra env files, passed through as `--env-file` flags.
	pub env_files: Vec<String>,
}

/// Whether a token is safe to place on a systemd exec line without quoting:
/// only an unambiguous, shell-neutral subset of ASCII. Anything else (a space, a
/// quote, a control byte, a glob/redirect metacharacter) forces double-quoting.
fn is_bare_safe(token: &str) -> bool {
	!token.is_empty()
		&& token.bytes().all(|b| {
			b.is_ascii_alphanumeric()
				|| matches!(
					b,
					b'-' | b'_' | b'.' | b'/' | b':' | b'=' | b'@' | b'+' | b','
				)
		})
}

/// Quote a single argument for a systemd `ExecStart=`/`ExecStop=` line. Tokens
/// made only of the safe subset are emitted verbatim; everything else is wrapped
/// in double quotes with `\` and `"` (and the C-style control escapes systemd
/// understands) backslash-escaped, so a path with spaces survives as one argument.
fn quote_arg(token: &str) -> String {
	if is_bare_safe(token) {
		return token.to_string();
	}
	let mut out = String::with_capacity(token.len() + 2);
	out.push('"');
	for ch in token.chars() {
		match ch {
			'"' => out.push_str("\\\""),
			'\\' => out.push_str("\\\\"),
			'\n' => out.push_str("\\n"),
			'\t' => out.push_str("\\t"),
			'\r' => out.push_str("\\r"),
			c => out.push(c),
		}
	}
	out.push('"');
	out
}

/// Reject any unit-embedded value containing ASCII control characters.
///
/// `WorkingDirectory=` (unlike exec-line tokens) takes the rest of its line
/// literally and honours no C-escapes, so a path with an embedded newline
/// would terminate the directive and inject arbitrary unit lines (e.g. an
/// `ExecStartPre=`). No legitimate path or flag value contains control bytes;
/// fail closed instead of trying to escape the unescapable.
pub fn validate_unit_opts(opts: &ServiceUnitOpts) -> Result<(), String> {
	fn check(field: &str, value: &str) -> Result<(), String> {
		if value.chars().any(|c| c.is_ascii_control()) {
			return Err(format!(
				"{field} contains a control character and cannot be embedded in a systemd unit: {value:?}"
			));
		}
		Ok(())
	}
	check("executable path", &opts.exe.to_string_lossy())?;
	check("working directory", &opts.working_dir.to_string_lossy())?;
	check("project name", &opts.project)?;
	for f in &opts.compose_files {
		check("compose file path", &f.to_string_lossy())?;
	}
	for p in &opts.profiles {
		check("profile", p)?;
	}
	for e in &opts.env_files {
		check("env file path", e)?;
	}
	Ok(())
}

/// The leading `podup` arguments shared by both the start and stop commands:
/// `-f <file>...  -p <project>  [--profile P]...  [--env-file E]...`. These must
/// precede the subcommand (`-f`/`-p` are not global flags).
fn leading_args(opts: &ServiceUnitOpts) -> Vec<String> {
	let mut args = Vec::new();
	for f in &opts.compose_files {
		args.push("-f".to_string());
		args.push(f.to_string_lossy().into_owned());
	}
	args.push("-p".to_string());
	args.push(opts.project.clone());
	for p in &opts.profiles {
		args.push("--profile".to_string());
		args.push(p.clone());
	}
	for e in &opts.env_files {
		args.push("--env-file".to_string());
		args.push(e.clone());
	}
	args
}

/// Render a full exec line: the absolute exe, the shared leading args, then the
/// command-specific trailing args, every token escaped and space-joined.
fn exec_line(opts: &ServiceUnitOpts, trailing: &[&str]) -> String {
	let mut tokens = Vec::new();
	tokens.push(opts.exe.to_string_lossy().into_owned());
	tokens.extend(leading_args(opts));
	tokens.extend(trailing.iter().map(|s| s.to_string()));
	tokens
		.iter()
		.map(|t| quote_arg(t))
		.collect::<Vec<_>>()
		.join(" ")
}

/// Render the full `.service` unit file content for service-mode autostart.
pub fn render_service_unit(opts: &ServiceUnitOpts) -> String {
	let start = exec_line(opts, &["up", "-d", "--build"]);
	let stop = exec_line(opts, &["down"]);
	// No `network-online.target` ordering: that target is a system-manager
	// concept and stays inert in the `--user` instance, so depending on it would
	// imply a network-readiness gate that never fires. Under linger the user
	// manager starts after the system network is already up, and podup reaches
	// Podman over a socket on demand, so no explicit network ordering is needed.
	format!(
		"[Unit]\n\
		 Description=podup {project}\n\
		 \n\
		 [Service]\n\
		 Type=oneshot\n\
		 RemainAfterExit=yes\n\
		 WorkingDirectory={workdir}\n\
		 ExecStart={start}\n\
		 ExecStop={stop}\n\
		 \n\
		 [Install]\n\
		 WantedBy=default.target\n",
		project = opts.project,
		workdir = opts.working_dir.display(),
		start = start,
		stop = stop,
	)
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::path::PathBuf;

	fn opts_single() -> ServiceUnitOpts {
		ServiceUnitOpts {
			exe: PathBuf::from("/usr/local/bin/podup"),
			compose_files: vec![PathBuf::from("/srv/app/docker-compose.yml")],
			project: "app".to_string(),
			working_dir: PathBuf::from("/srv/app"),
			profiles: Vec::new(),
			env_files: Vec::new(),
		}
	}

	#[test]
	fn renders_single_file_unit() {
		let s = render_service_unit(&opts_single());
		assert!(s.contains("Description=podup app"));
		// A `--user` unit must NOT order against the system `network-online.target`
		// (it is inert in the user manager); assert it is absent.
		assert!(!s.contains("network-online.target"));
		assert!(s.contains("Type=oneshot"));
		assert!(s.contains("RemainAfterExit=yes"));
		assert!(s.contains("WorkingDirectory=/srv/app"));
		assert!(s.contains("WantedBy=default.target"));
		assert!(s.contains(
			"ExecStart=/usr/local/bin/podup -f /srv/app/docker-compose.yml -p app up -d --build"
		));
		assert!(
			s.contains("ExecStop=/usr/local/bin/podup -f /srv/app/docker-compose.yml -p app down")
		);
	}

	#[test]
	fn renders_multiple_files_in_order() {
		let mut o = opts_single();
		o.compose_files = vec![
			PathBuf::from("/srv/app/base.yml"),
			PathBuf::from("/srv/app/override.yml"),
		];
		let s = render_service_unit(&o);
		assert!(s.contains(
			"ExecStart=/usr/local/bin/podup -f /srv/app/base.yml -f /srv/app/override.yml -p app up -d --build"
		));
		assert!(s.contains(
			"ExecStop=/usr/local/bin/podup -f /srv/app/base.yml -f /srv/app/override.yml -p app down"
		));
	}

	#[test]
	fn includes_profiles_and_env_files() {
		let mut o = opts_single();
		o.profiles = vec!["prod".to_string(), "web".to_string()];
		o.env_files = vec!["/srv/app/.env.prod".to_string()];
		let s = render_service_unit(&o);
		assert!(s.contains(
			"-p app --profile prod --profile web --env-file /srv/app/.env.prod up -d --build"
		));
		assert!(
			s.contains("-p app --profile prod --profile web --env-file /srv/app/.env.prod down")
		);
	}

	#[test]
	fn quotes_paths_with_spaces() {
		let mut o = opts_single();
		o.exe = PathBuf::from("/opt/my tools/podup");
		o.compose_files = vec![PathBuf::from("/srv/my app/compose.yml")];
		o.working_dir = PathBuf::from("/srv/my app");
		let s = render_service_unit(&o);
		// The exe and the compose path are double-quoted as single arguments.
		assert!(s.contains(
			"ExecStart=\"/opt/my tools/podup\" -f \"/srv/my app/compose.yml\" -p app up -d --build"
		));
		// WorkingDirectory takes the rest of the line literally, so it is not quoted.
		assert!(s.contains("WorkingDirectory=/srv/my app"));
	}

	#[test]
	fn ends_with_newline() {
		assert!(render_service_unit(&opts_single()).ends_with("WantedBy=default.target\n"));
	}

	#[test]
	fn validate_rejects_control_chars_in_workdir() {
		let mut o = opts_single();
		o.working_dir = PathBuf::from("/srv/app\nExecStartPre=/bin/evil");
		let err = validate_unit_opts(&o).unwrap_err();
		assert!(err.contains("working directory"), "{err}");
	}

	#[test]
	fn validate_rejects_control_chars_in_exe_and_files() {
		let mut o = opts_single();
		o.exe = PathBuf::from("/usr/local/bin/pod\x07up");
		assert!(validate_unit_opts(&o).is_err());

		let mut o = opts_single();
		o.compose_files = vec![PathBuf::from("/srv/app/com\npose.yml")];
		assert!(validate_unit_opts(&o).is_err());

		let mut o = opts_single();
		o.env_files = vec!["/srv/app/.env\r".to_string()];
		assert!(validate_unit_opts(&o).is_err());
	}

	#[test]
	fn validate_accepts_normal_paths() {
		assert!(validate_unit_opts(&opts_single()).is_ok());
		let mut o = opts_single();
		o.working_dir = PathBuf::from("/srv/my app (prod)");
		assert!(validate_unit_opts(&o).is_ok());
	}

	#[test]
	fn bare_safe_accepts_paths_rejects_spaces() {
		assert!(is_bare_safe("/srv/app/compose.yml"));
		assert!(is_bare_safe("app-1_v2.0"));
		assert!(!is_bare_safe("has space"));
		assert!(!is_bare_safe(""));
		assert!(!is_bare_safe("a\"b"));
	}

	#[test]
	fn quote_arg_escapes_quotes_and_backslashes() {
		assert_eq!(quote_arg("a b"), "\"a b\"");
		assert_eq!(quote_arg("a\"b"), "\"a\\\"b\"");
		assert_eq!(quote_arg("a\\b"), "\"a\\\\b\"");
	}
}
