//! The rejections an external `extends` can earn: a base service the file does
//! not define, a cycle spanning two files, and a chain past the depth guard.
//!
//! Split out of `tests/compose/parsing/parse/reuse/extends/mod.rs`; the imports are inherited via
//! `use super::*`.

use std::io::Write;

use super::*;

#[test]
fn extends_external_file_missing_service_errors() {
	// The external file exists but does not define the referenced base service.
	let dir = tempfile::tempdir().unwrap();
	let common_path = dir.path().join("common.yml");
	let mut f = std::fs::File::create(&common_path).unwrap();
	writeln!(f, "services:\n  other:\n    image: alpine\n").unwrap();

	let main_path = dir.path().join("docker-compose.yml");
	let mut m = std::fs::File::create(&main_path).unwrap();
	writeln!(
		m,
		"services:\n  app:\n    extends:\n      service: base\n      file: ./common.yml\n"
	)
	.unwrap();

	let err = parse_files_with_env_files(&[main_path], &[]).unwrap_err();
	assert!(err.to_string().contains("base"));
}

#[test]
fn extends_external_file_circular_across_files_errors() {
	// app -> base(in common.yml) -> app(back in the main file) forms a cycle that
	// spans the on-disk extends resolver; it must be detected, not loop forever.
	let dir = tempfile::tempdir().unwrap();
	let common_path = dir.path().join("common.yml");
	let mut f = std::fs::File::create(&common_path).unwrap();
	writeln!(
		f,
		"services:\n  base:\n    image: alpine\n    extends:\n      service: app\n      file: ./docker-compose.yml\n"
	)
	.unwrap();

	let main_path = dir.path().join("docker-compose.yml");
	let mut m = std::fs::File::create(&main_path).unwrap();
	writeln!(
		m,
		"services:\n  app:\n    extends:\n      service: base\n      file: ./common.yml\n"
	)
	.unwrap();

	assert!(parse_files_with_env_files(&[main_path], &[]).is_err());
}

#[test]
fn extends_external_chain_exceeds_max_depth_errors() {
	// A long cross-file extends chain trips the depth guard rather than recursing
	// without bound. Each file extends the next; 64 hops is well past the limit.
	let dir = tempfile::tempdir().unwrap();
	let levels = 64;
	for i in 0..levels {
		let path = dir.path().join(format!("svc{i}.yml"));
		let mut f = std::fs::File::create(&path).unwrap();
		writeln!(
			f,
			"services:\n  base:\n    image: alpine\n    extends:\n      service: base\n      file: ./svc{}.yml\n",
			i + 1
		)
		.unwrap();
	}

	let main_path = dir.path().join("docker-compose.yml");
	let mut m = std::fs::File::create(&main_path).unwrap();
	writeln!(
		m,
		"services:\n  app:\n    extends:\n      service: base\n      file: ./svc0.yml\n"
	)
	.unwrap();

	assert!(parse_files_with_env_files(&[main_path], &[]).is_err());
}
