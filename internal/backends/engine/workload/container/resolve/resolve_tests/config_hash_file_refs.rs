use super::*;

use std::io::Write;
use std::path::Path;

/// Helper for the file-hash tests: write `contents` to `name` in `dir` and
/// return the path. The secret/config source in the compose fixture points at
/// the same name so the resolver reads what we just wrote.
fn write_secret_file(dir: &Path, name: &str, contents: &[u8]) -> std::path::PathBuf {
	let path = dir.join(name);
	let mut f = std::fs::File::create(&path).expect("create secret fixture");
	f.write_all(contents).expect("write secret fixture");
	f.sync_all().ok();
	path
}

#[test]
fn config_hash_tracks_file_secret_bytes() {
	// Rotating a `file:` secret must change the hash so the container is
	// recreated to pick up the new (point-in-time) native secret. The
	// fixture is the same shape as the issue's reproducer.
	let base = temp_base();
	let file_path = write_secret_file(base.path(), "s.txt", b"v1");
	let yaml = format!(
		"services:\n  web:\n    image: x\n    secrets: [s]\nsecrets:\n  s:\n    file: {}\n",
		file_path.display()
	);
	let file = parse_str(&yaml).unwrap();
	let h1 = config_hash(
		&file.services["web"],
		&file,
		PROJECT,
		base.path(),
		&empty_digests(),
		&empty_env(),
	)
	.unwrap();
	std::fs::write(&file_path, b"v2").unwrap();
	let h2 = config_hash(
		&file.services["web"],
		&file,
		PROJECT,
		base.path(),
		&empty_digests(),
		&empty_env(),
	)
	.unwrap();
	assert_ne!(
		h1, h2,
		"changing only the file behind a file: secret must change the hash"
	);
}

#[test]
fn config_hash_stable_when_file_bytes_unchanged() {
	// Rewriting the same bytes (e.g. touch / chmod / atomic rename) must not
	// flap the hash and trigger a spurious recreate.
	let base = temp_base();
	let file_path = write_secret_file(base.path(), "s.txt", b"v1");
	let yaml = format!(
		"services:\n  web:\n    image: x\n    secrets: [s]\nsecrets:\n  s:\n    file: {}\n",
		file_path.display()
	);
	let file = parse_str(&yaml).unwrap();
	let h1 = config_hash(
		&file.services["web"],
		&file,
		PROJECT,
		base.path(),
		&empty_digests(),
		&empty_env(),
	)
	.unwrap();
	std::fs::write(&file_path, b"v1").unwrap();
	let h2 = config_hash(
		&file.services["web"],
		&file,
		PROJECT,
		base.path(),
		&empty_digests(),
		&empty_env(),
	)
	.unwrap();
	assert_eq!(
		h1, h2,
		"rewriting the same file bytes must leave the hash unchanged"
	);
}

#[test]
fn config_hash_tracks_file_config_bytes() {
	// The same recreate-on-rotation contract as file secrets applies to
	// file configs.
	let base = temp_base();
	let file_path = write_secret_file(base.path(), "c.txt", b"a");
	let yaml = format!(
		"services:\n  web:\n    image: x\n    configs: [c]\nconfigs:\n  c:\n    file: {}\n",
		file_path.display()
	);
	let file = parse_str(&yaml).unwrap();
	let h1 = config_hash(
		&file.services["web"],
		&file,
		PROJECT,
		base.path(),
		&empty_digests(),
		&empty_env(),
	)
	.unwrap();
	std::fs::write(&file_path, b"b").unwrap();
	let h2 = config_hash(
		&file.services["web"],
		&file,
		PROJECT,
		base.path(),
		&empty_digests(),
		&empty_env(),
	)
	.unwrap();
	assert_ne!(
		h1, h2,
		"changing only the file behind a file: config must change the hash"
	);
}

#[test]
fn config_hash_missing_file_errors_with_path() {
	// A missing or unreadable file must surface as an error carrying the path.
	// An empty contribution would make a vanished file look unchanged, the same
	// failure the issue measured whenever the host edit was preceded by a delete.
	let base = temp_base();
	let missing = base.path().join("does-not-exist.txt");
	let yaml = format!(
		"services:\n  web:\n    image: x\n    secrets: [s]\nsecrets:\n  s:\n    file: {}\n",
		missing.display()
	);
	let file = parse_str(&yaml).unwrap();
	let err = config_hash(
		&file.services["web"],
		&file,
		PROJECT,
		base.path(),
		&empty_digests(),
		&empty_env(),
	)
	.unwrap_err();
	let msg = err.to_string();
	assert!(
		msg.contains(&*missing.to_string_lossy()),
		"error must name the missing file path {missing:?}: {msg}",
	);
}

#[test]
fn config_hash_distinguishes_file_secret_from_inline_with_same_bytes() {
	// Same `name`, same bytes, different source: an inline `content:` and a
	// `file:` that resolve to the same string must still hash differently,
	// because the two sources reach the container through different secrets
	// management and a future `up` must not silently flatten one into the
	// other.
	let base = temp_base();
	let file_path = write_secret_file(base.path(), "s.txt", b"shared");
	let file_yaml = format!(
		"services:\n  web:\n    image: x\n    secrets: [s]\nsecrets:\n  s:\n    file: {}\n",
		file_path.display()
	);
	let inline_yaml =
		"services:\n  web:\n    image: x\n    secrets: [s]\nsecrets:\n  s:\n    content: shared\n";
	let file = parse_str(&file_yaml).unwrap();
	let inline = parse_str(inline_yaml).unwrap();
	let h_file = config_hash(
		&file.services["web"],
		&file,
		PROJECT,
		base.path(),
		&empty_digests(),
		&empty_env(),
	)
	.unwrap();
	let h_inline = config_hash(
		&inline.services["web"],
		&inline,
		PROJECT,
		base.path(),
		&empty_digests(),
		&empty_env(),
	)
	.unwrap();
	assert_ne!(
		h_file, h_inline,
		"a file: secret and an inline secret with the same bytes must hash differently"
	);
}
