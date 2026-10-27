use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use crate::compose::types::{ComposeFile, Service};
use crate::engine::secrets::scoped_name;
use crate::error::{ComposeError, Result};

use super::volumes::default_volume_name;
use super::volumes::resolve_bind_source;
use super::volumes::resolve_volume_name;

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
	// The effective identity of every network and named volume the service
	// references: the container is attached to the *resolved* network name and
	// mounts the *resolved* volume name, and those names are decided by the
	// top-level `networks:`/`volumes:` blocks, which are not part of the
	// serialized service. Renaming one of them changes the container's spec,
	// so the hash has to change with it: otherwise `up` creates the new
	// network and still finds the container "unchanged", leaving it attached
	// to the old one.
	for (kind, key, resolved) in overridden_resource_identities(service, file, project) {
		hash_resource_identity(&mut hasher, kind, &key, &resolved);
	}
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

/// The effective identities of the resources a service references, as
/// `(kind, key, resolved name)` records for the config hash to fold in.
///
/// A service names a network or a volume by its *compose key*; what the
/// container is really attached to (or mounts) is the *resolved* name, and
/// that is what the top-level `networks:`/`volumes:` blocks decide. Those
/// blocks are not part of the serialized service, so a rename of the real name
/// would otherwise leave the hash — and therefore the recreate-on-change
/// decision — untouched.
///
/// Only an *overridden* name is reported. The default (`{project}_{key}`) is
/// derived from the key the service already carries, so it is already in the
/// hash; folding it in as well would change the hash of every existing
/// container with a network or a named volume on the first `up` after the
/// upgrade, recreating containers whose writable layer would be lost (the
/// same reason `compose::dependencies::effective_depends_on` computes the
/// effective dependency list instead of rewriting the parsed one).
///
/// The resolution is the very one the create path uses
/// (`network::resolve_network_name` and `resolve_volume_name`), so the hash
/// and the container spec cannot describe two different resources.
pub(super) fn overridden_resource_identities(
	service: &Service,
	file: &ComposeFile,
	project: &str,
) -> Vec<(&'static [u8], String, String)> {
	let mut out: Vec<(&'static [u8], String, String)> = Vec::new();
	for key in service.networks.names() {
		let resolved = crate::engine::network::resolve_network_name(&key, file, project);
		if resolved != crate::engine::network::default_network_name(&key, project) {
			out.push((b"network", key, resolved));
		}
	}
	for reference in crate::engine::volume_mounts::named_volume_refs(service) {
		// A reference the file does not declare has no resolved name: the
		// create path rejects it on its own, and hashing must not turn a
		// `config --hash` of such a file into an error.
		if !file.volumes.contains_key(&reference) {
			continue;
		}
		let Ok(resolved) = resolve_volume_name(&reference, project, file) else {
			continue;
		};
		if resolved != default_volume_name(&reference, project) {
			out.push((b"volume", reference, resolved));
		}
	}
	out
}

/// Fold the effective identity of a referenced resource (the name the
/// container is really attached to or mounts) into the config hasher.
///
/// The framing is the same shape the secret/config payloads use — kind, key,
/// length-prefixed value — with its own kind tags, so a network named like a
/// secret cannot produce the same bytes as one.
fn hash_resource_identity(hasher: &mut sha2::Sha256, kind: &[u8], key: &str, resolved: &str) {
	use sha2::Digest;
	hasher.update(kind);
	hasher.update(key.as_bytes());
	hasher.update((resolved.len() as u64).to_le_bytes());
	hasher.update(resolved.as_bytes());
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
	// (`internal/backends/engine/host_files/watch/mod.rs`); aligned to a page and small enough
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
