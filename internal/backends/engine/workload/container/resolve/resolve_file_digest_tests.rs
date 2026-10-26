//! `config_hash` with a recorded `file_digests` map: the entries override
//! the file-read branch, so the per-container label describes the bytes
//! `create_project_secrets` actually uploaded rather than whatever is on
//! disk at label time. Split from `resolve_tests.rs` so the existing
//! `config_hash` tests stay together.

use super::config_hash;
use crate::parse_str;

/// Project name the file-digest tests use. Mirrors the engine's
/// `Engine::with_base_dir(..., "proj", ...)` so the scoped-name lookup in
/// `config_hash` agrees with the test fixtures.
const PROJECT: &str = "proj";

/// Empty digest map for the no-entry branch of `config_hash`: nothing was
/// uploaded, so every `file:` ref falls through to the file read it
/// always did.
fn empty_digests() -> std::collections::HashMap<String, [u8; 32]> {
	std::collections::HashMap::new()
}

/// Temp directory used as the project base for `config_hash`: relative
/// `file:` paths resolve against it, so each test owns its own fixture
/// tree and cannot race another test's reads.
fn temp_base() -> tempfile::TempDir {
	tempfile::tempdir().expect("tempdir")
}

/// Write `contents` to `name` in `dir` so the secret/config source in
/// the compose fixture reads the same bytes.
fn write_secret_file(dir: &std::path::Path, name: &str, contents: &[u8]) -> std::path::PathBuf {
	let path = dir.join(name);
	let mut f = std::fs::File::create(&path).expect("create secret fixture");
	std::io::Write::write_all(&mut f, contents).expect("write secret fixture");
	f.sync_all().ok();
	path
}

/// `config_hash` prefers the recorded digest in `file_digests` over a fresh
/// read of the host file: a `file:` source's bytes can change between the
/// upload and the label build, and the hash must describe what was
/// uploaded, not what is on disk at label time. Proves it by recording a
/// digest, then changing the file on disk to a different payload, and
/// asserting the hash is pinned to the recorded digest. With no entry the
/// hash follows the disk (the pre-fix behaviour), so the two cases are
/// each independently checked.
#[test]
fn config_hash_prefers_recorded_file_digest_over_disk_for_secret() {
	use sha2::{Digest, Sha256};
	let base = temp_base();
	let file_path = write_secret_file(base.path(), "s.txt", b"v1");
	let yaml = format!(
		"services:\n  web:\n    image: x\n    secrets: [s]\nsecrets:\n  s:\n    file: {}\n",
		file_path.display()
	);
	let file = parse_str(&yaml).unwrap();

	// Record the digest of the bytes currently on disk, then mutate the
	// file to different bytes. With the entry, the hash must not move;
	// without it, the hash must.
	let mut digests = std::collections::HashMap::new();
	digests.insert(
		format!("{PROJECT}_secret_s"),
		<[u8; 32]>::from(Sha256::digest(b"v1")),
	);
	let h_with_recorded =
		config_hash(&file.services["web"], &file, PROJECT, base.path(), &digests).unwrap();
	std::fs::write(&file_path, b"v2").unwrap();
	let h_after_disk_change =
		config_hash(&file.services["web"], &file, PROJECT, base.path(), &digests).unwrap();
	assert_eq!(
		h_with_recorded, h_after_disk_change,
		"a recorded digest must keep the hash stable even when the on-disk file changes"
	);

	// Sanity: without the entry the same on-disk change does move the hash,
	// which is the pre-fix behaviour this branch replaces.
	std::fs::write(&file_path, b"v1").unwrap();
	let h_disk_v1 = config_hash(
		&file.services["web"],
		&file,
		PROJECT,
		base.path(),
		&empty_digests(),
	)
	.unwrap();
	std::fs::write(&file_path, b"v2").unwrap();
	let h_disk_v2 = config_hash(
		&file.services["web"],
		&file,
		PROJECT,
		base.path(),
		&empty_digests(),
	)
	.unwrap();
	assert_ne!(
		h_disk_v1, h_disk_v2,
		"without a recorded entry, the hash follows the on-disk file"
	);

	// The recorded digest (v1) and the on-disk file (v2) must produce
	// distinct hashes, which is the property that fixes the bug: the
	// container carries the label of v1 because that is what was uploaded,
	// while the file on disk is now v2.
	assert_ne!(
		h_with_recorded, h_disk_v2,
		"recorded digest and on-disk bytes must hash differently when the bytes differ"
	);
}

/// Same contract for `configs:` references: the recorded digest wins over
/// a re-read of the host file.
#[test]
fn config_hash_prefers_recorded_file_digest_over_disk_for_config() {
	use sha2::{Digest, Sha256};
	let base = temp_base();
	let file_path = write_secret_file(base.path(), "c.txt", b"v1");
	let yaml = format!(
		"services:\n  web:\n    image: x\n    configs: [c]\nconfigs:\n  c:\n    file: {}\n",
		file_path.display()
	);
	let file = parse_str(&yaml).unwrap();

	let mut digests = std::collections::HashMap::new();
	digests.insert(
		format!("{PROJECT}_config_c"),
		<[u8; 32]>::from(Sha256::digest(b"v1")),
	);
	let h_with_recorded =
		config_hash(&file.services["web"], &file, PROJECT, base.path(), &digests).unwrap();
	std::fs::write(&file_path, b"v2").unwrap();
	let h_after_disk_change =
		config_hash(&file.services["web"], &file, PROJECT, base.path(), &digests).unwrap();
	assert_eq!(
		h_with_recorded, h_after_disk_change,
		"a recorded digest must keep the config hash stable even when the on-disk file changes"
	);

	std::fs::write(&file_path, b"v1").unwrap();
	let h_disk_v1 = config_hash(
		&file.services["web"],
		&file,
		PROJECT,
		base.path(),
		&empty_digests(),
	)
	.unwrap();
	std::fs::write(&file_path, b"v2").unwrap();
	let h_disk_v2 = config_hash(
		&file.services["web"],
		&file,
		PROJECT,
		base.path(),
		&empty_digests(),
	)
	.unwrap();
	assert_ne!(
		h_disk_v1, h_disk_v2,
		"without a recorded entry, the config hash follows the on-disk file"
	);
	assert_ne!(
		h_with_recorded, h_disk_v2,
		"recorded digest and on-disk bytes must hash differently for configs when the bytes differ"
	);
}

/// Two services that share the same `file:` secret get the same hash from
/// one snapshot of the digest map. `create_project_secrets` reads the file
/// once (per the union) and records one digest; `config_hash` is then
/// called once per service and must produce the same hash for both, which
/// is what makes a single recreate decision correct.
#[test]
fn two_services_sharing_a_file_secret_get_the_same_hash_from_one_snapshot() {
	use sha2::{Digest, Sha256};
	let base = temp_base();
	let file_path = write_secret_file(base.path(), "shared.txt", b"v1");
	let yaml = format!(
		"services:\n  app:\n    image: x\n    secrets: [s]\n  worker:\n    image: x\n    secrets: [s]\nsecrets:\n  s:\n    file: {}\n",
		file_path.display()
	);
	let file = parse_str(&yaml).unwrap();

	let mut snap = std::collections::HashMap::new();
	snap.insert(
		format!("{PROJECT}_secret_s"),
		<[u8; 32]>::from(Sha256::digest(b"v1")),
	);

	let h_app = config_hash(&file.services["app"], &file, PROJECT, base.path(), &snap).unwrap();
	let h_worker =
		config_hash(&file.services["worker"], &file, PROJECT, base.path(), &snap).unwrap();
	assert_eq!(
		h_app, h_worker,
		"two services referencing the same file: secret must hash equal from one snapshot"
	);

	// And changing the file on disk after the snapshot does not move
	// either hash, since both services consult the snapshot, not the disk.
	std::fs::write(&file_path, b"v2").unwrap();
	let h_app_after =
		config_hash(&file.services["app"], &file, PROJECT, base.path(), &snap).unwrap();
	let h_worker_after =
		config_hash(&file.services["worker"], &file, PROJECT, base.path(), &snap).unwrap();
	assert_eq!(h_app, h_app_after);
	assert_eq!(h_worker, h_worker_after);
}
