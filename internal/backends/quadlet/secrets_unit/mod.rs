//! The unit that creates the Podman secrets a project mounts.
//!
//! Quadlet has no secret unit (a `.secret` file gives `No files parsed`), and
//! `Secret=` in a `.container` only names a secret that already exists. `up`
//! creates them; the export has to as well, or the stack cannot start from the
//! units alone.
//!
//! One plain systemd unit per project, `<project>-podup-secrets.service`, a
//! `Type=oneshot` with `RemainAfterExit=yes`, holds one `podman secret create`
//! per secret or config whose source is a `file:`. Every container that mounts
//! one has `Requires=` and `After=` on it, so a file that cannot be read makes
//! the service fail and the containers not start. The unit holds the absolute
//! path of the file and never its bytes: Podman reads the file when the unit
//! starts, so a rotated file is picked up on the next start.
//!
//! What cannot be exported safely is left to the operator and reported:
//! `environment:` and `content:` sources would put the value in a unit file,
//! and an `external:` secret is not ours to create. The container unit keeps its
//! `Secret=`, so without the secret it does not start.
//!
//! One unit per project, one `podman secret create` per name: two containers
//! that share a secret would otherwise run `--replace` at the same time, and a
//! delete-then-create of one name by two processes deletes what the other made.

mod validate;

pub(super) use validate::validate;

use crate::compose::types::{ComposeFile, ConfigConfig, SecretConfig};
use crate::error::sanitize_name;

use super::unit::owner_marker;
use super::QuadletUnit;

/// What kind of top-level definition a name belongs to. It decides the Podman
/// name: `up` namespaces a secret and a config of the same compose name apart.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
	Secret,
	Config,
}

impl Kind {
	fn word(self) -> &'static str {
		match self {
			Self::Secret => "secret",
			Self::Config => "config",
		}
	}
}

/// What one mounted name needs from the export.
pub(crate) enum Source {
	/// A file Podman reads when the unit starts.
	File {
		/// The Podman secret name `up` would create.
		scoped: String,
		/// Absolute path of the file, resolved when the units are generated.
		path: String,
		/// The mode the container sees the file with when the service gives none:
		/// the permission bits of the host file, as `up` mounts it.
		mode: u32,
		/// The file could not be read at generation time, so `mode` is the
		/// narrowest one rather than the file's own.
		mode_guessed: bool,
	},
	/// The operator creates it: a source whose value cannot sit in a unit.
	Operator {
		/// What to tell the operator.
		advice: String,
	},
	/// A source `up` would refuse (a link, a mode with an execute bit, a name
	/// Podman cannot carry, two sources). It is not exported, and the unit holds a
	/// command for its name that fails, so a container that mounts it does not
	/// start with whatever secret of that name happens to exist.
	Refused {
		/// The name in the compose file.
		name: String,
		/// What is wrong and what to do.
		advice: String,
	},
	/// No definition, or one with no source: nothing to export and nothing to say.
	None,
}

/// The pieces of a top-level definition the export reads, the same for a secret
/// and for a config.
pub(crate) struct Def<'a> {
	pub file: Option<&'a str>,
	pub content: Option<&'a str>,
	pub environment: Option<&'a str>,
	pub external: bool,
}

impl Def<'_> {
	/// How many of `file`, `environment` and `content` the definition gives.
	pub(crate) fn sources(&self) -> usize {
		usize::from(self.file.is_some())
			+ usize::from(self.content.is_some())
			+ usize::from(self.environment.is_some())
	}
}

impl<'a> From<&'a SecretConfig> for Def<'a> {
	fn from(d: &'a SecretConfig) -> Self {
		Self {
			file: d.file.as_deref(),
			content: d.content.as_deref(),
			environment: d.environment.as_deref(),
			external: d.external == Some(true),
		}
	}
}

impl<'a> From<&'a ConfigConfig> for Def<'a> {
	fn from(d: &'a ConfigConfig) -> Self {
		Self {
			file: d.file.as_deref(),
			content: d.content.as_deref(),
			environment: d.environment.as_deref(),
			external: d.external == Some(true),
		}
	}
}

/// The name of the project's secrets unit, with its suffix.
pub(crate) fn service_name(project: &str) -> String {
	format!(
		"{}.service",
		super::render::unit_stem(project, "podup-secrets")
	)
}

/// Whether `var` can be named in a command the operator will copy: a POSIX
/// variable name, nothing a shell would read as more.
fn is_plain_variable(var: &str) -> bool {
	let mut chars = var.chars();
	chars
		.next()
		.is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
		&& chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// What stands at a `file:` path, looked at without following a link, as `up`
/// looks at it: it reads a regular file and refuses a link, a directory, a pipe,
/// a device and a socket.
enum Host {
	/// A regular file, with its permission bits.
	Regular(u32),
	/// Nothing could be learned (the file is not there yet, or cannot be reached).
	Unknown,
	/// Not a regular file; the word for what it is.
	Other(&'static str),
}

#[cfg(unix)]
fn host_file(path: &str) -> Host {
	use std::os::unix::fs::{FileTypeExt, PermissionsExt};
	let Ok(meta) = std::fs::symlink_metadata(path) else {
		return Host::Unknown;
	};
	let ty = meta.file_type();
	if ty.is_file() {
		return Host::Regular(meta.permissions().mode() & 0o7777);
	}
	Host::Other(if ty.is_symlink() {
		"link"
	} else if ty.is_dir() {
		"directory"
	} else if ty.is_fifo() {
		"FIFO (named pipe)"
	} else if ty.is_char_device() || ty.is_block_device() {
		"device"
	} else if ty.is_socket() {
		"socket"
	} else {
		"file that is not a regular one"
	})
}

#[cfg(not(unix))]
fn host_file(_path: &str) -> Host {
	Host::Unknown
}

/// Whether a path can be written into a command the operator will copy without
/// quoting: no character a shell reads as more.
fn is_plain_path(path: &str) -> bool {
	!path.is_empty()
		&& path
			.chars()
			.all(|c| c.is_ascii_alphanumeric() || "._/@+-".contains(c))
}

/// Decide what the export does with one definition `name` of `kind`.
///
/// What `up` checks when it creates and mounts a secret, this checks too, or the
/// secret is not exported: a name Podman secrets can carry, a mode that is not
/// executable or setuid. Nothing here prints text taken from the compose file
/// that has not passed one of those checks, since the operator may copy it.
pub(crate) fn classify(
	kind: Kind,
	project: &str,
	name: &str,
	def: Option<Def<'_>>,
	base_dir: &std::path::Path,
) -> Source {
	let Some(def) = def else {
		return Source::None;
	};
	if def.external {
		return Source::None;
	}
	if def.sources() == 0 {
		return Source::None;
	}
	let word = kind.word();
	let refused = |advice: String| Source::Refused {
		name: name.to_string(),
		advice,
	};
	if def.sources() > 1 {
		return refused(more_than_one_source(word, name));
	}
	if !crate::engine::is_valid_secret_name(name) {
		return refused(format!(
			"a {word} has a name Podman secrets cannot carry (ASCII letters, digits, `-`, `_` \
			 and `.`, at most 128, no leading dot); rename it"
		));
	}
	let scoped = crate::engine::scoped_name(project, word, name);
	let to_create = format!("podman secret create --label podup.project={project} {scoped}");
	if let Some(file) = def.file {
		// An absolute path is the path on the machine the units run on, a POSIX
		// one, and is taken as written whatever the host that generates them. A
		// relative one, or `~`, is resolved the way `up` resolves it, and where
		// that host joins with `\` (Windows) the join is read as `/`.
		let path = if file.starts_with('/') {
			file.to_string()
		} else {
			let joined = crate::engine::container::resolve_bind_source(file, base_dir);
			#[cfg(windows)]
			let joined = joined.replace('\\', "/");
			joined
		};
		let path = tidy_path(&path);
		if let Err(why) = checked_path(&path) {
			return refused(format!(
				"the file of {word} {name:?} is not exported ({why}); create it with \
				 `{to_create} <file>`"
			));
		}
		let (mode, mode_guessed) = match host_file(&path) {
			Host::Regular(mode) => (mode, false),
			Host::Unknown => (0o400, true),
			Host::Other(what) => {
				return refused(format!(
					"the file {} of {word} {name:?} is not exported: it is a {what}, and `up` \
					 refuses it too. Give the {word} a regular file and generate the units again",
					sanitize_name(&path)
				));
			}
		};
		if crate::engine::reject_dangerous_secret_mode(mode, &format!("{word} {name:?}")).is_err() {
			let fix = if is_plain_path(&path) {
				format!("for example `chmod 0600 {path}`")
			} else {
				"for example to 0600".to_string()
			};
			return refused(format!(
				"the file {} of {word} {name:?} is not exported: its mode is {mode:04o}, \
				 which has an execute, setuid, setgid or sticky bit. `up` mounts it without \
				 those bits; the export refuses it instead of trimming it. Change the mode, \
				 {fix}, and generate the units again",
				sanitize_name(&path)
			));
		}
		return Source::File {
			scoped,
			path,
			mode,
			mode_guessed,
		};
	}
	let advice = if let Some(var) = def.environment {
		if is_plain_variable(var) {
			format!(
				"{word} {name:?} takes its value from the variable {var}, which a unit cannot \
				 carry; create it with `printenv {var} | {to_create} -`"
			)
		} else {
			format!(
				"{word} {name:?} takes its value from a variable whose name is not a plain \
				 identifier ({var:?}), and a unit cannot carry the value; create it with \
				 `{to_create} -` and the value on standard input"
			)
		}
	} else {
		format!(
			"{word} {name:?} has its value in the compose file, which a unit must not carry; \
			 create it with `{to_create} -` and the value on standard input, or give the \
			 {word} a `file:`"
		)
	};
	Source::Operator { advice }
}

/// `./` and `//` in the middle of a path mean nothing and read badly in a unit;
/// they are dropped without touching `..`. A `\` is a character of a POSIX name.
fn tidy_path(path: &str) -> String {
	let joined: Vec<&str> = path
		.split('/')
		.filter(|part| !part.is_empty() && *part != ".")
		.collect();
	let joined = joined.join("/");
	if path.starts_with('/') {
		format!("/{joined}")
	} else {
		joined
	}
}

/// What to tell the operator about a definition that gives more than one source.
/// `up` takes `content`, then `environment`, then `file`; the Compose
/// Specification allows one, so no choice is made here.
fn more_than_one_source(word: &str, name: &str) -> String {
	format!(
		"{word} {name:?} gives more than one of `file`, `environment` and `content`, and the \
		 Compose Specification allows one; keep the one you mean"
	)
}

/// A path the unit can carry: absolute, with no control character. Anything else
/// (a newline would end the `ExecStart=` line) is refused, and the secret is left
/// to the operator, with the container still naming it so it does not start.
fn checked_path(path: &str) -> Result<(), &'static str> {
	if !path.starts_with('/') {
		return Err("its path is not absolute");
	}
	if path.chars().any(char::is_control) {
		return Err("its path holds a control character");
	}
	Ok(())
}

/// One command of the secrets unit.
enum Export {
	/// Create the secret from the file.
	Create {
		scoped: String,
		path: String,
		/// The permission bits the file had when the units were generated, when
		/// they were learned: the command fails if the file has any other now.
		mode: Option<u32>,
	},
	/// Fail, saying which name was not exported.
	Refuse { name: String },
}

/// Every secret and config a service of `file` mounts, by what the export does
/// with it. Referenced names only: a definition no service mounts creates nothing.
pub(crate) fn referenced(
	file: &ComposeFile,
	project: &str,
	base_dir: &std::path::Path,
) -> Vec<Source> {
	let mut seen: Vec<(&str, &str)> = Vec::new();
	let mut out = Vec::new();
	// A job with a schedule is written as a container unit by the same code as a
	// service, so what it mounts is created by the same unit.
	let jobs = file
		.jobs
		.values()
		.filter(|job| !job.triggers.schedule.is_empty())
		.map(|job| &job.service);
	for service in file.services.values().chain(jobs) {
		for secret in &service.secrets {
			let name = secret.source();
			if !seen.contains(&(Kind::Secret.word(), name)) {
				seen.push((Kind::Secret.word(), name));
				out.push(classify(
					Kind::Secret,
					project,
					name,
					file.secrets.get(name).map(Def::from),
					base_dir,
				));
			}
		}
		for config in &service.configs {
			let name = config.source();
			if !seen.contains(&(Kind::Config.word(), name)) {
				seen.push((Kind::Config.word(), name));
				out.push(classify(
					Kind::Config,
					project,
					name,
					file.configs.get(name).map(Def::from),
					base_dir,
				));
			}
		}
	}
	out
}

/// The script every `ExecStart=` creates a secret with, one constant for all
/// secrets. The name, the project, the path and the mode arrive as arguments
/// (`$1` to `$4`), always quoted, so the shell never reads them as text, and `--`
/// ends the options of each `podman` call so a value that starts with a dash is a
/// value. It does what `up` does when it creates a secret: replace one that is
/// there only if it is labelled as the project's, and refuse any other. `podman`
/// and `stat` are found the way the rest of the unit finds its programs, through
/// `/usr/bin/env`.
///
/// The file is opened once, as `up` reads it, and everything after is about that
/// descriptor (`3`), not the path, so the file cannot be changed for another
/// between the check and the use:
///
/// - the path must be a regular file and not a link before it is opened (a link, a
///   directory, a pipe and a device are what `up` refuses; opening a pipe would
///   wait for a writer);
/// - the open file must be a regular file, and the same one the path names now,
///   which a link put in the path while it was opened is not;
/// - its mode must be the one it had when the units were generated (`$4`, empty
///   when that was not known), since the container is given that mode: a file
///   that has become more open, or more closed, makes the command fail, and
///   generating the units again takes the mode it has then;
/// - the secret is created from the descriptor, through standard input.
pub(super) const CREATE_SCRIPT: &str = r#"set -eu; p() { /usr/bin/env podman "$@"; }; s() { /usr/bin/env stat "$@"; }; [ -f "$3" ] && [ ! -L "$3" ] || { echo "podup: $3 is not a regular file (a link, a directory, a pipe or a device is refused); not creating $1" >&2; exit 1; }; exec 3< "$3"; [ -f /dev/fd/3 ] && [ "$(s -L -c %d:%i /dev/fd/3)" = "$(s -c %d:%i -- "$3")" ] || { echo "podup: $3 is not the regular file it was when it was opened; not creating $1" >&2; exit 1; }; if [ -n "$4" ]; then m=$(s -L -c %a /dev/fd/3); [ $((0$m)) -eq $((0$4)) ] || { echo "podup: $3 has mode $m, and had $4 when the units were generated; not creating $1; generate the units again" >&2; exit 1; }; fi; p secret exists -- "$1" && rc=0 || rc=$?; case $rc in 0) [ "$(p secret inspect --format '{{index .Spec.Labels "podup.project"}}' -- "$1")" = "$2" ] || { echo "podup: a secret named $1 exists and is not labelled podup.project=$2; not replacing it" >&2; exit 1; };; 1) ;; *) exit 1;; esac; exec /usr/bin/env podman secret create --replace --label "podup.project=$2" -- "$1" - <&3"#;

/// What the unit runs for a name that was refused: it fails, so a container that
/// mounts the name does not start with whatever secret of that name exists.
pub(super) const REFUSE_SCRIPT: &str = r#"set -eu; echo "podup: $1 was not exported when the units were generated (the warnings of that run say why); not creating it" >&2; exit 1"#;

/// The secrets unit of `file`, when some secret or config has a file source,
/// and the warnings for what is left to the operator.
///
/// With `write_unit` false (the units `autostart --mode quadlet` installs) no
/// unit is written and nothing is said here: the container units keep the
/// warnings they have always had.
pub(crate) fn build(
	file: &ComposeFile,
	project: &str,
	base_dir: &std::path::Path,
	write_unit: bool,
) -> (Option<QuadletUnit>, Vec<String>) {
	if !write_unit {
		return (None, Vec::new());
	}
	let mut exports = Vec::new();
	let mut warnings = Vec::new();
	for source in referenced(file, project, base_dir) {
		match source {
			Source::File {
				scoped,
				path,
				mode,
				mode_guessed,
			} => {
				if mode_guessed {
					warnings.push(format!(
						"the file {} could not be read to learn its mode, so {scoped} is \
						 mounted with the narrowest one (0400) unless the service gives a mode",
						sanitize_name(&path)
					));
				}
				exports.push(Export::Create {
					scoped,
					path,
					mode: (!mode_guessed).then_some(mode),
				});
			}
			Source::Refused { name, advice } => {
				warnings.push(advice);
				exports.push(Export::Refuse { name });
			}
			Source::Operator { advice } => warnings.push(advice),
			Source::None => {}
		}
	}
	if exports.is_empty() {
		return (None, warnings);
	}
	let create = exec_word(CREATE_SCRIPT).expect("the script has no control character");
	let refuse = exec_word(REFUSE_SCRIPT).expect("the script has no control character");
	let project_word = exec_word(project).expect("a project name has no control character");
	let mut text = owner_marker(project);
	// The timeout is for a path that was turned into a pipe after it was checked:
	// opening it would wait for a writer, and a oneshot waits for ever otherwise.
	text.push_str(&format!(
		"[Unit]\nDescription=Podman secrets of {project} (podup)\n\n[Service]\nType=oneshot\nRemainAfterExit=yes\nTimeoutStartSec=120\n"
	));
	for export in &exports {
		match export {
			Export::Create { scoped, path, mode } => {
				let (Some(name), Some(path)) = (exec_word(scoped), exec_word(path)) else {
					continue;
				};
				// An empty word when the mode was not learned: the script then has
				// nothing to compare with.
				let mode = mode.map_or_else(String::new, |m| format!("{m:04o}"));
				let mode = exec_word(&mode).expect("digits have no control character");
				text.push_str(&format!(
					"ExecStart=/bin/sh -c {create} podup-secret {name} {project_word} {path} {mode}\n"
				));
			}
			Export::Refuse { name } => {
				let name = exec_word(name).unwrap_or_else(|| "\"?\"".to_string());
				text.push_str(&format!(
					"ExecStart=/bin/sh -c {refuse} podup-secret {name}\n"
				));
			}
		}
	}
	let unit = QuadletUnit {
		filename: service_name(project),
		contents: text,
	};
	(Some(unit), warnings)
}

/// The first source of `file` that `up` would refuse, as an error, for the
/// command that writes the units to refuse before it writes any.
pub(crate) fn refusals(
	file: &ComposeFile,
	project: &str,
	base_dir: &std::path::Path,
) -> crate::error::Result<()> {
	for source in referenced(file, project, base_dir) {
		if let Source::Refused { advice, .. } = source {
			return Err(crate::error::ComposeError::Unsupported(advice));
		}
	}
	Ok(())
}

/// One word of an `ExecStart=` line. systemd splits the line on blanks, reads
/// quotes and backslash escapes, and substitutes `%` specifiers and `$` variables;
/// a path or a name goes in as one double-quoted word with each made literal.
///
/// The word for `text`, or `None` when it holds a control character (a newline
/// would end the line and start another directive).
fn exec_word(text: &str) -> Option<String> {
	if text.chars().any(char::is_control) {
		return None;
	}
	let mut word = String::with_capacity(text.len() + 2);
	word.push('"');
	for c in text.chars() {
		match c {
			'\\' => word.push_str("\\\\"),
			'"' => word.push_str("\\\""),
			'%' => word.push_str("%%"),
			'$' => word.push_str("$$"),
			other => word.push(other),
		}
	}
	word.push('"');
	Some(word)
}

#[cfg(test)]
#[path = "hardening_tests.rs"]
mod hardening_tests;
#[cfg(all(test, unix))]
#[path = "real_guard_tests.rs"]
mod real_guard_tests;
#[cfg(all(test, unix))]
#[path = "real_tests.rs"]
mod real_tests;
#[cfg(test)]
#[path = "tests.rs"]
mod tests;
