use super::resolve_bind_source;
use super::{
	config_hash, resolve_links, resolve_stop_signal, resolve_volume_name, resolve_volumes_from,
};
use crate::parse_str;

mod bind_source;
mod config_hash;
mod config_hash_file_refs;
mod links;
mod stop_signal;
mod volume_name;
mod volumes_from;

/// Project name used by the file-hash tests below. Mirrors the engine's
/// `Engine::with_base_dir(..., "proj", ...)` so the scoped-name lookup in
/// `config_hash` agrees with the test fixtures.
const PROJECT: &str = "proj";

/// Empty digest map for tests that exercise `config_hash`'s file-read
/// branch: nothing was uploaded, so every `file:` ref falls through to the
/// path it always took. The new tests in this file build their own maps.
fn empty_digests() -> std::collections::HashMap<String, [u8; 32]> {
	std::collections::HashMap::new()
}

/// A temp directory used as the project base directory for `config_hash`
/// calls: relative `file:` paths resolve against it, so each test owns its
/// own fixture tree and cannot race another test's reads.
fn temp_base() -> tempfile::TempDir {
	tempfile::tempdir().expect("tempdir")
}
