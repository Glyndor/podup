//! Name, path, link, and config-hash resolution for container creation.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use crate::compose::types::{ComposeFile, Service};
use crate::env_file;
use crate::error::{ComposeError, Result};

use super::super::secrets::scoped_name;

/// Resolve a named-volume reference to the volume name `create_volumes`
/// produced: a custom `name:`, the raw name for an external volume, or the
/// `{project}_{name}` form.
///
/// An empty reference is an anonymous volume (an image `VOLUME` directive or a
/// colon-less short-form mount); it carries no name, so Podman assigns one and
/// it is returned unchanged. A non-empty reference that is *not* declared under
/// the top-level `volumes:` map is rejected, matching the compose-spec: leaving
/// it unprefixed would make Podman auto-create a bare, unlabelled global volume
/// that escapes project namespacing and is never reclaimed by `down -v`.
pub(super) fn resolve_volume_name(
	reference: &str,
	project: &str,
	file: &ComposeFile,
) -> Result<String> {
	if reference.is_empty() {
		return Ok(String::new());
	}
	match file.volumes.get(reference) {
		Some(cfg) => Ok(
			if let Some(name) = cfg.as_ref().and_then(|c| c.name.as_deref()) {
				name.to_string()
			} else if cfg.as_ref().and_then(|c| c.external).unwrap_or(false) {
				reference.to_string()
			} else {
				format!("{project}_{reference}")
			},
		),
		None => Err(ComposeError::Unsupported(format!(
			"service refers to undefined volume \"{reference}\"; declare it under the \
			 top-level `volumes:` key, or use a bind mount or anonymous volume instead"
		))),
	}
}

/// Resolve a bind-mount source path: expand a leading `~`, then make a relative
/// path absolute against the project base directory. Absolute paths (including
/// staged secret/config files) are returned unchanged.
pub(crate) fn resolve_bind_source(src: &str, base_dir: &Path) -> String {
	if src.is_empty() {
		return src.to_string();
	}
	let expanded = if let Some(rest) = src.strip_prefix("~/") {
		// Join with the platform separator rather than hardcoding `/`, and look
		// up the home directory in a platform-correct way (USERPROFILE on
		// native Windows, where HOME is usually unset).
		match home_dir() {
			Some(home) => home.join(rest).to_string_lossy().into_owned(),
			None => src.to_string(),
		}
	} else if src == "~" {
		home_dir()
			.map(|h| h.to_string_lossy().into_owned())
			.unwrap_or_else(|| src.to_string())
	} else {
		src.to_string()
	};
	if Path::new(&expanded).is_absolute() {
		expanded
	} else {
		base_dir.join(&expanded).to_string_lossy().into_owned()
	}
}

/// The current user's home directory. Prefers `HOME` (set on Unix and most
/// shells), falling back to `USERPROFILE` for native Windows where `HOME` is
/// usually absent. Empty values are treated as unset.
fn home_dir() -> Option<std::path::PathBuf> {
	std::env::var_os("HOME")
		.or_else(|| std::env::var_os("USERPROFILE"))
		.filter(|v| !v.is_empty())
		.map(std::path::PathBuf::from)
}

/// Resolve a service's `links` to concrete container references.
///
/// A compose `links:` entry names a sibling service; it is rewritten to that
/// service's container name with the service name kept as the network alias
/// (`{container}:{alias}`), so the linked container is reachable by the compose
/// service name. `external_links` reference containers outside the project and
/// are passed through verbatim.
pub(super) fn resolve_links(service: &Service, file: &ComposeFile, project: &str) -> Vec<String> {
	let mut links: Vec<String> = service
		.links
		.iter()
		.map(|link| {
			let (target, alias) = link.split_once(':').unwrap_or((link, link));
			let container = file
				.services
				.get(target)
				.map(|svc| {
					svc.container_name
						.clone()
						// Auto-generated container names are always index-suffixed;
						// a link/volumes_from references the first replica.
						.unwrap_or_else(|| format!("{project}-{target}-1"))
				})
				.unwrap_or_else(|| target.to_string());
			format!("{container}:{alias}")
		})
		.collect();
	links.extend(service.external_links.iter().cloned());
	links
}

/// Resolve a service's `volumes_from` entries to concrete container references.
///
/// libpod's `SpecGenerator.volumes_from` expects container names, but compose
/// `volumes_from:` names sibling services. Each entry may be `<service>`,
/// `service:<name>`, or `container:<name>`, any of which may carry a trailing
/// `:ro`/`:rw` access-mode suffix. The bare-service and `service:` forms are
/// rewritten to the referenced service's container name (an explicit
/// `container_name:` is honoured, otherwise the first replica
/// `{project}-{service}-1`), preserving
/// the access mode. The `container:` form already names a container outside the
/// project and is passed through verbatim, as is any service name that is not
/// declared in this compose file.
pub(super) fn resolve_volumes_from(
	service: &Service,
	file: &ComposeFile,
	project: &str,
) -> Vec<String> {
	service
		.volumes_from
		.iter()
		.map(|entry| {
			// Split off a trailing access mode so it survives the rewrite.
			let (reference, mode) = match entry.rsplit_once(':') {
				Some((head, tail @ ("ro" | "rw"))) => (head, Some(tail)),
				_ => (entry.as_str(), None),
			};
			let resolved = if let Some(name) = reference.strip_prefix("container:") {
				// Already a concrete container outside the project: pass through.
				name.to_string()
			} else {
				let target = reference.strip_prefix("service:").unwrap_or(reference);
				file.services
					.get(target)
					.map(|svc| {
						svc.container_name
							.clone()
							// Auto-generated container names are always index-suffixed;
							// a link/volumes_from references the first replica.
							.unwrap_or_else(|| format!("{project}-{target}-1"))
					})
					// Unknown service: leave the reference untouched.
					.unwrap_or_else(|| target.to_string())
			};
			match mode {
				Some(mode) => format!("{resolved}:{mode}"),
				None => resolved,
			}
		})
		.collect()
}

/// Stable content hash of a service definition, stored as the
/// `podup.config-hash` label. On `up`, comparing this against the label on an
/// existing container tells podup whether the service configuration changed
/// and the container must be recreated, or is unchanged and can be left as is.
///
/// The resolved bytes of any inline `content:`/`environment:` secret or config
/// the service references are folded in, so rotating an inline value recreates
/// the container to pick it up. Previously these were live host bind-mounts, so
/// a re-`up` reflected the change without recreation; now they are point-in-time
/// Podman-native secrets, so the recreate must be driven by the hash. A `file:`
/// source is also copied into a Podman-native secret at `up` time (see
/// `engine::secrets::plan`), so the same recreate rule applies: the file's
/// contents are digested into this hash using the same kind-and-name labels as
/// an inline payload, and a `file:` edit therefore forces a recreate the way an
/// inline rotation does. `external:` sources are by-reference and contribute
/// nothing.
///
/// `file_digests` is the SHA-256 of every `file:` payload `create_project_secrets`
/// uploaded during this invocation, keyed by the project-scoped Podman secret
/// name. When a `file:` ref has an entry there, the recorded digest is folded
/// in directly instead of re-reading the host file: a re-read could land on
/// bytes that changed between the upload and the label build (image
/// acquisition, a `depends_on` wait), which would describe the new bytes in
/// the label while the container still mounts the old ones. A ref with no
/// entry falls through to the file read, which is exactly what every call
/// site that does not go through `create_project_secrets` (`config --hash`,
/// autostart's start mode) keeps seeing.
///
/// `base_dir` is the project directory `file:` paths are resolved against:
/// the same anchor `bind` mounts and the secret creator use, so a path that
/// the engine can read at `up` is the same path the engine hashes now.
pub(crate) fn config_hash(
	service: &Service,
	file: &ComposeFile,
	project: &str,
	base_dir: &Path,
	file_digests: &HashMap<String, [u8; 32]>,
) -> Result<String> {
	use sha2::{Digest, Sha256};
	let mut hasher = Sha256::new();
	// Canonicalise through `serde_json::Value` first: `Value::Object` is
	// backed by a `BTreeMap` (see `serde_json::Map`), so a `to_value` then
	// `to_vec` round-trip emits map keys in lexicographic order regardless of
	// how the service's `HashMap`-typed fields happen to iterate. A direct
	// `to_vec(&service)` walks the field's `HashMap` directly and produces
	// different bytes for different iteration orders, which would flap the
	// hash on every parse and trigger spurious recreates. The double
	// serialisation is therefore load-bearing for the stable-hash invariant
	// (#1364 deferred the proposed optimisation; see
	// `config_hash_stable_despite_map_field_order`). Fail closed if either
	// step fails (e.g. a non-scalar mapping key in an `x-` extension):
	// returning an empty/default hash would make distinct services hash
	// identically and silently suppress recreation and inline-secret rotation.
	let serialized = serde_json::to_value(service)
		.and_then(|v| serde_json::to_vec(&v))
		.map_err(|e| ComposeError::Unsupported(format!("cannot hash service config: {e}")))?;
	hasher.update(&serialized);
	for secret_ref in &service.secrets {
		if let Some(def) = file.secrets.get(secret_ref.source()) {
			hash_inline_payload(
				&mut hasher,
				b"secret",
				secret_ref.source(),
				def.content.as_deref(),
				def.environment.as_deref(),
			);
			if let Some(host_path) = def.file.as_deref() {
				let resolved = resolve_bind_source(host_path, base_dir);
				let resolved = Path::new(&resolved);
				let key = scoped_name(project, "secret", secret_ref.source());
				match file_digests.get(&key) {
					Some(digest) => hash_recorded_file_payload(
						&mut hasher,
						b"secret",
						secret_ref.source(),
						digest,
					),
					None => {
						hash_file_payload(&mut hasher, b"secret", secret_ref.source(), resolved)?
					}
				}
			}
		}
	}
	for config_ref in &service.configs {
		if let Some(def) = file.configs.get(config_ref.source()) {
			hash_inline_payload(
				&mut hasher,
				b"config",
				config_ref.source(),
				def.content.as_deref(),
				def.environment.as_deref(),
			);
			if let Some(host_path) = def.file.as_deref() {
				let resolved = resolve_bind_source(host_path, base_dir);
				let resolved = Path::new(&resolved);
				let key = scoped_name(project, "config", config_ref.source());
				match file_digests.get(&key) {
					Some(digest) => hash_recorded_file_payload(
						&mut hasher,
						b"config",
						config_ref.source(),
						digest,
					),
					None => {
						hash_file_payload(&mut hasher, b"config", config_ref.source(), resolved)?
					}
				}
			}
		}
	}
	Ok(hasher
		.finalize()
		.iter()
		.map(|b| format!("{b:02x}"))
		.collect())
}

/// Fold an inline secret/config's resolved bytes into the config hasher. Inline
/// `content:` contributes its literal bytes; `environment:` contributes the
/// current value of the named variable (empty if unset; `up` errors on a
/// genuinely missing var later). `file:`/`external:` sources contribute nothing
/// from this branch (`file:` is handled by [`hash_file_payload`]; `external:`
/// contributes nothing at all because it is by-reference).
fn hash_inline_payload(
	hasher: &mut sha2::Sha256,
	kind: &[u8],
	name: &str,
	content: Option<&str>,
	environment: Option<&str>,
) {
	use sha2::Digest;
	// The environment-sourced branch holds the resolved `String` in a local so
	// the `&[u8]` view stays alive across `update()` (#1364, and also E0716
	// otherwise: a temporary `as_bytes()` is freed at end of statement).
	let env_value;
	let payload: Option<&[u8]> = match (content, environment) {
		(Some(c), _) => Some(c.as_bytes()),
		(None, Some(var)) => {
			env_value = std::env::var(var).unwrap_or_default();
			Some(env_value.as_bytes())
		}
		(None, None) => None,
	};
	if let Some(payload) = payload {
		hasher.update(kind);
		hasher.update(name.as_bytes());
		// Length-prefix so (name, payload) pairs cannot be confused across refs.
		hasher.update((payload.len() as u64).to_le_bytes());
		// `update` takes `impl AsRef<[u8]>`; `&[u8]` skips the `.to_vec()`
		// round-trip the previous code paid (#1364).
		hasher.update(payload);
	}
}

/// Fold a `file:` secret/config's resolved bytes into the config hasher.
///
/// `path` is read here, not at `up` time, so the hash reflects the bytes
/// the next `up` would copy into the Podman-native secret. A missing or
/// unreadable file is an error carrying the path; an empty contribution
/// would make a vanished file look unchanged, so the very thing the issue
/// measured (a file edit leaving the hash equal) would stay broken whenever
/// the host edit was preceded by a delete.
///
/// The file content is read through `sha2::Sha256` itself rather than
/// loaded into memory: a multi-megabyte secret was the source of one of
/// the early hash designs, and `update` streams whatever we feed it, but
/// feeding it whole-byte slices of a `Vec<u8>` defeats the point. The
/// inner `Sha256` produces a fixed 32-byte digest regardless of file size,
/// so the outer config hasher never carries more than that one block for
/// the file payload; the rest of the framing (`kind`, `name`, and the
/// digest-vs-inline-bytes asymmetry) is what keeps a `file:` secret
/// from hashing equal to an inline secret that happens to carry the same
/// bytes.
fn hash_file_payload(
	hasher: &mut sha2::Sha256,
	kind: &[u8],
	name: &str,
	path: &Path,
) -> Result<()> {
	use sha2::{Digest, Sha256};
	// The error text routes through `kind_str` so it reads "secret"/"config"
	// rather than the `[115, 101, 99, 114, 101, 116]` byte sequence `Debug`
	// emits for `&[u8]`; the diagnostic is the only thing a user sees when an
	// `up` fails on this branch, so the difference is worth the line.
	let kind_str = std::str::from_utf8(kind).unwrap_or("<binary>");
	let mut file = std::fs::File::open(path).map_err(|e| {
		ComposeError::Unsupported(format!(
			"{kind_str} {name:?} from \"{}\": {e}",
			path.display()
		))
	})?;
	let mut digest_hasher = Sha256::new();
	// 8 KiB mirrors what the rest of the project uses for chunked copies
	// (`internal/engine/watch/mod.rs`); aligned to a page and small enough
	// to keep stack pressure zero on the unwinding paths.
	let mut buf = [0u8; 8192];
	loop {
		let n = file.read(&mut buf).map_err(|e| {
			ComposeError::Unsupported(format!(
				"{kind_str} {name:?} from \"{}\": {e}",
				path.display()
			))
		})?;
		if n == 0 {
			break;
		}
		digest_hasher.update(&buf[..n]);
	}
	let digest = digest_hasher.finalize();
	hash_recorded_file_payload(hasher, kind, name, digest.as_slice());
	Ok(())
}

/// Fold a precomputed SHA-256 of a `file:` secret/config into the config
/// hasher using the same framing [`hash_file_payload`] applies to a digest
/// freshly read off disk. The framing is load-bearing: it is what keeps a
/// `file:` secret from hashing equal to an inline secret that happens to
/// carry the same bytes, and is what kept the digest-vs-inline-bytes
/// asymmetry stable for callers that record and replay it. Called by the
/// `file_digests` branch of [`config_hash`].
fn hash_recorded_file_payload(hasher: &mut sha2::Sha256, kind: &[u8], name: &str, digest: &[u8]) {
	use sha2::Digest;
	hasher.update(kind);
	hasher.update(name.as_bytes());
	// Length-prefixed like the inline payloads, so the two kinds of record
	// frame the same way.
	hasher.update((digest.len() as u64).to_le_bytes());
	hasher.update(digest);
}

pub(super) fn build_env(service: &Service, base_dir: &Path) -> Result<Vec<String>> {
	let entries = service.env_file.to_entries();
	let env_file_vars = if !entries.is_empty() {
		env_file::load_env_file_entries(&entries, base_dir)?
	} else {
		HashMap::new()
	};
	Ok(env_file::merge_env(
		service.environment.to_map(),
		env_file_vars,
	))
}

/// Resolve a compose `stop_signal:` value to its numeric `syscall.Signal`.
///
/// libpod's `SpecGenerator.stop_signal` is an integer; sending the signal name
/// as a string returns HTTP 500. Accepts a bare number (`"15"`), a signal name
/// with or without the `SIG` prefix (`"SIGTERM"`, `"term"`), case-insensitively.
/// An unrecognized name returns a clear [`ComposeError::Unsupported`] rather than
/// silently dropping the value.
pub(super) fn resolve_stop_signal(signal: &str) -> Result<i64> {
	let trimmed = signal.trim();
	if let Ok(num) = trimmed.parse::<i64>() {
		return Ok(num);
	}
	let upper = trimmed.to_ascii_uppercase();
	let name = upper.strip_prefix("SIG").unwrap_or(&upper);
	signal_number(name)
		.ok_or_else(|| ComposeError::Unsupported(format!("unknown stop_signal '{signal}'")))
}

/// First realtime signal number on Linux/glibc (`SIGRTMIN`).
const SIGRTMIN: i64 = 34;
/// Last realtime signal number on Linux/glibc (`SIGRTMAX`).
const SIGRTMAX: i64 = 64;

/// Map a bare (no `SIG` prefix) upper-case signal name to its Linux number.
///
/// In addition to the named POSIX signals, the realtime signals are resolved:
/// `RTMIN`/`RTMAX` and the offset forms `RTMIN+N`/`RTMAX-N`, matching how
/// docker-compose and Podman interpret them. The computed number must stay within
/// the realtime range [`SIGRTMIN`]..=[`SIGRTMAX`]; an out-of-range offset (e.g.
/// `RTMIN+40`) resolves to `None` so the caller reports it as unknown.
fn signal_number(name: &str) -> Option<i64> {
	if let Some(n) = realtime_signal_number(name) {
		return Some(n);
	}
	let n = match name {
		"HUP" => 1,
		"INT" => 2,
		"QUIT" => 3,
		"ILL" => 4,
		"TRAP" => 5,
		"ABRT" | "IOT" => 6,
		"BUS" => 7,
		"FPE" => 8,
		"KILL" => 9,
		"USR1" => 10,
		"SEGV" => 11,
		"USR2" => 12,
		"PIPE" => 13,
		"ALRM" => 14,
		"TERM" => 15,
		"STKFLT" => 16,
		"CHLD" | "CLD" => 17,
		"CONT" => 18,
		"STOP" => 19,
		"TSTP" => 20,
		"TTIN" => 21,
		"TTOU" => 22,
		"URG" => 23,
		"XCPU" => 24,
		"XFSZ" => 25,
		"VTALRM" => 26,
		"PROF" => 27,
		"WINCH" => 28,
		"IO" | "POLL" => 29,
		"PWR" => 30,
		"SYS" => 31,
		_ => return None,
	};
	Some(n)
}

/// Resolve a realtime signal name (`RTMIN`, `RTMAX`, `RTMIN+N`, `RTMAX-N`) to its
/// number, or `None` if `name` is not a realtime form or the computed number
/// falls outside the realtime range [`SIGRTMIN`]..=[`SIGRTMAX`].
fn realtime_signal_number(name: &str) -> Option<i64> {
	let (base, rest) = if let Some(rest) = name.strip_prefix("RTMIN") {
		(SIGRTMIN, rest)
	} else {
		let rest = name.strip_prefix("RTMAX")?;
		(SIGRTMAX, rest)
	};
	let number = if rest.is_empty() {
		base
	} else {
		// An offset must be `+N` (only valid after RTMIN) or `-N` (only valid
		// after RTMAX), matching POSIX/glibc's `SIGRTMIN+n` / `SIGRTMAX-n`.
		let (sign, digits) = rest.split_at(1);
		let offset: i64 = digits.parse().ok()?;
		match (base == SIGRTMIN, sign) {
			(true, "+") => base + offset,
			(false, "-") => base - offset,
			_ => return None,
		}
	};
	(SIGRTMIN..=SIGRTMAX).contains(&number).then_some(number)
}

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "resolve_file_digest_tests.rs"]
mod file_digest_tests;
