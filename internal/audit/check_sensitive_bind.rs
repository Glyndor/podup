//! The `sensitive_bind_mount` check, in its own file because it carries
//! two path tables, a matcher for the rootless runtime directory and a
//! short-form volume parser, which together would push `check_fns.rs` past
//! the line limit.

use podup::compose::types::{ComposeFile, Service, VolumeMount, VolumeType};

use super::super::Finding;
use super::check_fns::finding;

/// `volumes:` entry whose bind-mount source is a sensitive host path. One
/// finding per matching mount; the message names the path and the access
/// mode so the operator can act. A non-bind mount (named volume, tmpfs,
/// npipe, cluster, anonymous volume) is out of scope: the host filesystem
/// is not in play.
///
/// ## What is on the list, and why each entry earns it
///
/// Every entry grants the container a path to host state the operator
/// almost never wants to expose. Each entry below is defended with the
/// specific capability an attacker gains, not a generic "looks
/// sensitive". The list is intentionally short: a check that fires on
/// paths operators legitimately mount teaches them to ignore `audit`,
/// which costs more than the check is worth.
///
/// ### Exact paths: the container runtime socket itself
///
/// These paths are the only entries where the bind target is the
/// attack vector. The Unix socket grants the container the ability to
/// spawn sibling containers as the user that owns the daemon (the host
/// user under rootless Podman, root under rootful); reading the file
/// contents is not even part of the attack. This is qualitatively worse
/// than every other entry on the list, so the reason message uses
/// stronger wording and the exact match is checked before the prefix
/// matches below.
///
/// - `/var/run/docker.sock`, `/run/docker.sock`: the Docker daemon
///   socket. Either spelling reaches the same file on every distro
///   podup supports; both are listed because the audit cannot tell
///   whether the operator's `/var/run` is a symlink to `/run`.
/// - `/var/run/podman/podman.sock`, `/run/podman/podman.sock`: the
///   system (rootful) Podman socket, same dual-spelling reason.
/// - `/run/user/<uid>/podman/podman.sock`, and the same under
///   `/var/run/user/`: the rootless Podman socket. This is the one a
///   rootless operator actually has, and the one podup connects to by
///   default. The uid varies, so it cannot sit in the exact list; it is
///   matched by shape, with the uid segment required to be all digits.
///   The whole of `/run/user/` is deliberately not matched: it also holds
///   the session bus, the audio server and the display sockets, which
///   operators mount on purpose for desktop containers, and flagging
///   those would teach them to ignore this check.
///
/// ### Path prefixes: the directory and every subdirectory
///
/// Anything under one of these directories exposes the same capability,
/// so a subpath is the same finding as the directory itself. `/etc/ssh`
/// is exactly what the operator is exposing; matching it differently
/// from `/etc` would let `image: ... -v /etc/ssh:/ssh:ro` slip through
/// the gate while `-v /etc:/etc:ro` does not.
///
/// - `/`: the host root filesystem. Matched as an exact entry rather
///   than a prefix because every absolute path starts with `/`, and the
///   prefix-list match would therefore fire on every bind mount in the
///   compose file. As an exact entry it fires on the root itself
///   (`-v /:/host:ro` exposes the whole host filesystem) and stays
///   silent on every path under it (`/etc`, `/var`, `/home`, ...),
///   which the prefix list already covers when they apply and which
///   stay silent on purpose when they do not. The wording uses the
///   prefix-list shape ("a sensitive host path (...)") rather than the
///   stronger socket wording because the bind target here is a
///   directory the container can walk, not a single file that grants
///   a daemon verb; the capability, not the file, is the attack vector.
///   The normaliser folds `/`, `//`, and `/.` to a single `/` before
///   the comparison so every POSIX-equivalent spelling of the root
///   reaches this entry as one finding.
/// - `/proc`: kernel and process state. The notable escape is
///   `/proc/1/root`, a symlink that resolves to the host's root
///   filesystem once the container has the right mount-namespace
///   permissions; reading `/proc/<pid>/environ` leaks another
///   container's environment. `/proc/sys` carries kernel tunables.
/// - `/sys`: hardware inventory and kernel tunables. Writing to
///   `/sys/bus/.../drivers/.../bind` or `/sys/.../unbind` can detach or
///   rebind host drivers; reading `/sys/firmware` leaks platform state.
/// - `/dev`: raw device nodes. `/dev/sda` is a passthrough to the host
///   disk; the mount is not a sandbox boundary on this path.
/// - `/etc`: system config. Hostname leaks via `/etc/hostname`, sshd
///   config and host keys via `/etc/ssh`, account list via
///   `/etc/passwd`. A writable bind is also a foothold for persistence
///   (modify `crontab`, drop an init script, etc.).
/// - `/boot`: bootloader config and the initramfs. A writable bind is
///   a foothold for boot-chain compromise; a read-only bind still
///   discloses kernel version and the initrd layout.
/// - `/root`: root's home directory. SSH keys under `/root/.ssh` and
///   root config under `/root/.config` are the secrets the operator is
///   exposing. `/root` is named rather than `.ssh` alone so the rule
///   matches across distros that place the home elsewhere
///   (`/var/root` is out of scope by design; podup targets rootless
///   Podman where the running user's `$HOME` is the relevant value).
/// - `/run/podman`, `/var/run/podman`, `/run/docker`, `/var/run/docker`,
///   and `/run/user/<uid>/podman` (plus its `/var/run` spelling):
///   the directories that hold the daemon socket. The exact socket
///   matches above catch the file itself; these prefixes catch a
///   directory bind, which still exposes the socket to the container
///   and is the more common mistake when an operator copies a Docker
///   recipe that mounts the whole run dir.
///
/// ## What the rule deliberately does not do
///
/// - **`:ro` still fires.** A read-only `/etc` discloses hostname,
///   sshd config, and the user list; that is a real leak. The check
///   does not silently pass on `ro`, the message just names the access
///   mode so the operator can tell which kind of exposure they have.
/// - **Relative paths (`./`, `../`, `~/`) do not fire.** The audit
///   cannot tell whether a relative path resolves under the project
///   directory or under a sensitive location; the false-positive cost
///   of flagging every `./data:/data` is higher than the false-negative
///   cost of missing the rare `~/.ssh`. The check stays Linux-specific:
///   Windows paths (`C:\...`) do not start with `/`, so they do not
///   match either. Operators on Windows have a different surface and a
///   different audit story; this rule does not pretend to cover them.
/// - **A path under the project directory does not fire.** Operators
///   legitimately mount the project tree into a build container; the
///   project directory is conventionally `/srv/app`, `/home/<user>/<repo>`,
///   or wherever the operator cloned it. None of `/srv`, `/home`, or
///   the home subdirectories are in the list above, so a `./data:/data`
///   or a `/srv/app/data:/data` stays silent on purpose. The host root
///   is matched as an exact entry for the same reason: `/`, `/home`,
///   and `/srv` are different things to expose, and `/` only fires
///   when the operator really did mean the whole host filesystem.
pub fn check_sensitive_bind_mount(
	name: &str,
	service: &Service,
	_file: &ComposeFile,
) -> Vec<Finding> {
	let mut out = Vec::new();
	for mount in &service.volumes {
		let Some((source, read_only)) = bind_source(mount) else {
			continue;
		};
		// Normalise: trailing slashes are dropped (`/etc/` and `/etc`
		// are the same directory) and the host root folds to a single
		// `/` regardless of how many leading slashes the operator
		// wrote or whether they spelled the current directory (`/.`).
		// After this fold the exact-match entry for `/` catches `/`,
		// `//`, `/.`, and `/./` as the same finding; every other path
		// passes through unchanged so the prefix loop keeps its
		// existing behaviour.
		let normalized = normalize_host_path(source);
		let mode = if read_only { "read-only " } else { "" };
		if let Some(reason) = sensitive_bind_reason(&normalized, mode) {
			out.push(finding(name, "sensitive_bind_mount", &reason));
		}
	}
	out
}

/// Fold the spellings of the host root that POSIX treats as equivalent
/// to a single `/` and drop any trailing slashes from the rest. The
/// sensitive list's exact-match entry for `/` then catches every
/// spelling without the prefix loop firing on every absolute path.
///
/// POSIX collapses duplicate leading slashes (every distro podup
/// supports treats `//` and `///` as `/`), and `/.` is the current
/// directory of the root, which is the root itself. After folding
/// these the only way an absolute bind path can reach the empty
/// stripped case is by being the root, so the caller does not need
/// a separate "skip when empty" branch.
fn normalize_host_path(source: &str) -> String {
	// Strip trailing slashes (`/etc/` -> `/etc`).
	let trimmed = source.trim_end_matches('/');
	// Collapse duplicate leading slashes (`//foo` -> `/foo`).
	let stripped = trimmed.trim_start_matches('/');
	// After both strips the only spellings of the root are:
	// - empty (`/`, `//`, `///`, `/./`)
	// - `.` (`/.`, `//.`)
	if stripped.is_empty() || stripped == "." {
		return "/".to_string();
	}
	format!("/{}", stripped)
}

/// Container runtime socket paths: the bind target IS the attack vector.
/// Returned with stronger wording than the prefix matches below. Order
/// matters only in that any exact match takes priority over the
/// directory-prefix matches; all four spellings point at the same file
/// on every supported distro.
const SENSITIVE_BIND_EXACT: &[&str] = &[
	"/var/run/docker.sock",
	"/run/docker.sock",
	"/var/run/podman/podman.sock",
	"/run/podman/podman.sock",
];

/// Sensitive path prefixes paired with the one-line capability each
/// grants. The reason text is what the operator sees in the report;
/// keeping it adjacent to the path makes the per-entry rationale
/// impossible to drift between the list and the message.
const SENSITIVE_BIND_PREFIXES: &[(&str, &str)] = &[
	(
		"/proc",
		"kernel and process state (/proc/1/root reads the host root filesystem)",
	),
	(
		"/sys",
		"kernel tunables and hardware inventory (can unbind host drivers)",
	),
	(
		"/dev",
		"raw device nodes (/dev/sda is a passthrough to the host disk)",
	),
	("/etc", "system config (hostname, sshd_config, /etc/passwd)"),
	("/boot", "bootloader config and initramfs"),
	("/root", "root's home directory (SSH keys, root config)"),
	(
		"/run/podman",
		"system Podman runtime directory (contains podman.sock)",
	),
	(
		"/var/run/podman",
		"system Podman runtime directory (contains podman.sock)",
	),
	(
		"/run/docker",
		"Docker runtime directory (contains docker.sock)",
	),
	(
		"/var/run/docker",
		"Docker runtime directory (contains docker.sock)",
	),
];

/// What a path under a user's runtime directory is, for the rootless
/// Podman shape only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RootlessPodman {
	/// `<runtime>/podman/podman.sock` itself.
	Socket,
	/// `<runtime>/podman` or anything under it other than the socket.
	Directory,
}

/// Match `/run/user/<uid>/podman[/...]` and its `/var/run/user/` spelling,
/// with `<uid>` all digits. Anything else under `/run/user/` answers
/// `None`: see the doc comment on the check for why the whole runtime
/// directory is not flagged.
fn rootless_podman_path(path: &str) -> Option<RootlessPodman> {
	let rest = path
		.strip_prefix("/run/user/")
		.or_else(|| path.strip_prefix("/var/run/user/"))?;
	let (uid, rest) = rest.split_once('/')?;
	if uid.is_empty() || !uid.bytes().all(|b| b.is_ascii_digit()) {
		return None;
	}
	match rest {
		"podman/podman.sock" => Some(RootlessPodman::Socket),
		"podman" => Some(RootlessPodman::Directory),
		_ if rest.starts_with("podman/") => Some(RootlessPodman::Directory),
		_ => None,
	}
}

/// Build the reason string for a sensitive bind mount. The exact socket
/// matches fire first because the message is stronger; the host root
/// fires next with the prefix-list wording because the bind target is
/// a directory the container can walk, not a single file that grants
/// a daemon verb; prefix matches follow with a capability hint that
/// names what the attacker gains.
fn sensitive_bind_reason(normalized_path: &str, mode: &str) -> Option<String> {
	let rootless = rootless_podman_path(normalized_path);
	if normalized_path == "/" {
		// The whole host filesystem is a superset of every path on the
		// list below. The entry sits beside the prefix list because the
		// wording matches, and uses an exact comparison rather than a
		// prefix comparison because every absolute path starts with `/`
		// and a prefix match would fire on every bind mount in the
		// file. The normaliser folds `/`, `//`, `/.`, and `/./` to the
		// canonical `/` before this branch is reached.
		return Some(format!(
			"volumes: {normalized_path} is {mode}a sensitive host path \
			 (the whole host filesystem)"
		));
	}
	if SENSITIVE_BIND_EXACT.contains(&normalized_path) || rootless == Some(RootlessPodman::Socket) {
		// Exact match: the bind target is the attack vector itself, not
		// a directory of files. Phrasing distinguishes this entry from
		// the directory-prefix matches below so a CI consumer can grep
		// for the socket variant without parsing the path.
		return Some(format!(
			"volumes: {normalized_path} is the container runtime socket; \
			 the {mode}bind gives the container the ability to spawn sibling containers"
		));
	}
	if rootless == Some(RootlessPodman::Directory) {
		return Some(format!(
			"volumes: {normalized_path} is {mode}a sensitive host path \
			 (rootless Podman runtime directory, contains podman.sock)"
		));
	}
	for (prefix, capability) in SENSITIVE_BIND_PREFIXES {
		if normalized_path == *prefix || normalized_path.starts_with(&format!("{prefix}/")) {
			return Some(format!(
				"volumes: {normalized_path} is {mode}a sensitive host path ({capability})"
			));
		}
	}
	None
}

/// Extract the bind source path and read-only flag from one volume
/// mount, or `None` when the entry is not a bind (named volume, tmpfs,
/// npipe, cluster, anonymous volume) or the source path is unusable.
/// Only absolute Linux paths are returned; relative paths are operator-
/// chosen and are deliberately out of scope for this check.
fn bind_source(mount: &VolumeMount) -> Option<(&str, bool)> {
	match mount {
		VolumeMount::Short(s) => {
			// A single-path short form (`/data`, `cache`) is an anonymous
			// volume, not a bind. The first colon separates src:dst;
			// anything after that is mount options. Both the colon-less
			// form and the colon-bearing form have to be guarded.
			let (src, _dst, opts) = split_short_volume_spec(s);
			if !is_short_bind_source(src) {
				return None;
			}
			// The check stays Linux-specific: relative paths and Windows
			// paths do not match the sensitive prefixes. Operators who
			// mount `./data:/data` or `C:/...:/data` get no finding here
			// on purpose; see the check-level doc comment for why.
			if !src.starts_with('/') {
				return None;
			}
			let read_only = opts.split(',').any(|opt| opt.trim() == "ro");
			Some((src, read_only))
		}
		VolumeMount::Long {
			volume_type: VolumeType::Bind,
			source,
			read_only,
			..
		} => {
			let src = source.as_deref()?;
			if !src.starts_with('/') {
				return None;
			}
			Some((src, read_only.unwrap_or(false)))
		}
		_ => None,
	}
}

/// Split a short-form volume spec `src:dst[:opts]` into its three
/// fields, with the same Windows-drive-letter carve-out as the engine
/// (`internal/engine/volume_mounts/spec.rs::split_volume_spec`) so the
/// two agree on which colon is the separator. Duplicated here to keep
/// the audit free of a dependency on the engine module.
fn split_short_volume_spec(s: &str) -> (&str, &str, &str) {
	let scan_from = if has_windows_drive_prefix(s) { 2 } else { 0 };
	let seps: Vec<usize> = s
		.as_bytes()
		.iter()
		.enumerate()
		.skip(scan_from)
		.filter(|&(_, b)| *b == b':')
		.map(|(i, _)| i)
		.take(2)
		.collect();
	match seps.as_slice() {
		[] => (s, s, ""),
		[a] => (&s[..*a], &s[a + 1..], ""),
		[a, b] => (&s[..*a], &s[a + 1..*b], &s[b + 1..]),
		_ => unreachable!("take(2) yields at most two separators"),
	}
}

/// Whether a short-form spec's source is a host bind (vs a named volume).
/// A leading `/`, `.`, `~`, or Windows drive prefix marks a bind; a
/// bare identifier is a named volume. Mirrors
/// `internal/engine/volume_mounts/spec.rs::is_bind_source`.
fn is_short_bind_source(src: &str) -> bool {
	src.starts_with('/')
		|| src.starts_with('.')
		|| src.starts_with('~')
		|| has_windows_drive_prefix(src)
}

/// Whether `s` begins with a Windows drive-letter prefix (`C:`), so the
/// colon that follows is part of the path rather than a `src:dst`
/// separator. Mirrors
/// `internal/engine/volume_mounts/spec.rs::has_windows_drive_prefix`.
fn has_windows_drive_prefix(s: &str) -> bool {
	let b = s.as_bytes();
	b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}
