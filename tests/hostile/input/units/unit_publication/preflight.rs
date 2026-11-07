use std::fs;
use std::path::Path;
use std::process::Command;

use podup::quadlet::{generate_at, write_units, QuadletUnit};

const OLD: &[u8] = b"# podup-owner: f6\n[Network]\nNetworkName=previous\n";

fn units() -> Vec<QuadletUnit> {
	let file = podup::parse_str(
		"services:\n  web:\n    image: alpine\nnetworks:\n  extra: {}\n  default: {}\n",
	)
	.unwrap();
	let units = generate_at(&file, "f6", Path::new("/srv/app")).units;
	assert_eq!(units.len(), 3);
	assert_eq!(units[0].filename, "f6-extra.network");
	assert_eq!(units[1].filename, "f6-default.network");
	assert_eq!(units[2].filename, "f6-web.container");
	units
}

fn assert_unchanged(dir: &Path, prior: &[u8]) {
	let mut names: Vec<_> = fs::read_dir(dir)
		.unwrap()
		.map(|e| e.unwrap().file_name())
		.collect();
	names.sort();
	assert_eq!(
		names,
		["f6-extra.network", "f6-web.container"],
		"a failed batch published a new file"
	);
	assert_eq!(
		fs::read(dir.join("f6-extra.network")).unwrap(),
		prior,
		"a failed batch changed previous bytes"
	);
}

fn positive_control(dir: &Path, units: &[QuadletUnit]) {
	let written = write_units(dir, units).unwrap();
	assert_eq!(written.len(), 3);
	for unit in units {
		assert_eq!(
			fs::read(dir.join(&unit.filename)).unwrap(),
			unit.contents.as_bytes()
		);
	}
}

#[test]
fn final_fifo_rejects_batch_without_new_files_or_changed_bytes() {
	let root = tempfile::tempdir().unwrap();
	let dir = root.path();
	let units = units();
	fs::write(dir.join("f6-extra.network"), OLD).unwrap();
	let fifo = dir.join("f6-web.container");
	assert!(Command::new("mkfifo")
		.arg(&fifo)
		.status()
		.unwrap()
		.success());
	let err = write_units(dir, &units).unwrap_err();
	assert!(err.to_string().contains("not a regular file"), "{err}");
	assert_unchanged(dir, OLD);
	use std::os::unix::fs::FileTypeExt;
	assert!(fs::symlink_metadata(&fifo).unwrap().file_type().is_fifo());
	fs::remove_file(fifo).unwrap();
	positive_control(dir, &units);
}

#[test]
fn final_foreign_project_rejects_batch_without_new_files_or_changed_bytes() {
	let root = tempfile::tempdir().unwrap();
	let dir = root.path();
	let units = units();
	fs::write(dir.join("f6-extra.network"), OLD).unwrap();
	let foreign = dir.join("f6-web.container");
	let foreign_bytes = b"# podup-owner: other\n[Container]\nImage=precious\n";
	fs::write(&foreign, foreign_bytes).unwrap();
	let err = write_units(dir, &units).unwrap_err();
	assert!(
		err.to_string().contains("belongs to project 'other'"),
		"{err}"
	);
	assert_unchanged(dir, OLD);
	assert_eq!(fs::read(&foreign).unwrap(), foreign_bytes);
	fs::write(&foreign, "# podup-owner: f6\n[Container]\nImage=old\n").unwrap();
	positive_control(dir, &units);
}
