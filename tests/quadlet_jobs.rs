//! `generate quadlet -o` for the `jobs:` section: a schedule becomes a systemd
//! timer next to a oneshot container unit.
//!
//! Every command names an unreachable socket, so none of them can touch a real
//! Podman, and every output directory is a temporary one.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

/// A socket that cannot be connected to.
const NO_ENGINE: &str = "unix:///nonexistent/x.sock";

fn compose(contents: &str) -> (TempDir, PathBuf) {
	let dir = tempfile::tempdir().expect("create temp dir");
	let path = dir.path().join("compose.yaml");
	fs::write(&path, contents).expect("write compose file");
	(dir, path)
}

fn generate(file: &Path, out_dir: &Path) -> Output {
	Command::new(env!("CARGO_BIN_EXE_podup"))
		.args(["--socket", NO_ENGINE])
		.args(["-p", "proj", "-f", file.to_str().unwrap()])
		.args(["generate", "quadlet", "-o", out_dir.to_str().unwrap()])
		.env("NO_COLOR", "1")
		.env_remove("RUST_LOG")
		.output()
		.expect("run podup")
}

fn names(dir: &Path) -> Vec<String> {
	let mut found: Vec<String> = fs::read_dir(dir)
		.expect("read output dir")
		.map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
		.collect();
	found.sort();
	found
}

const BACKUP: &str = "\
services:
  db:
    image: postgres
jobs:
  backup:
    image: alpine
    command: ['sh', '-c', 'pg_dump > /backup/dump.sql']
    environment:
      MODE: nightly
    depends_on: [db]
    triggers:
      schedule:
        - '30 2 * * *'
        - cron: '0 12 * * mon-fri'
          timezone: Europe/Madrid
          missed_fires: skip
        - cron: '*/10 * * * *'
          concurrency: queue
        - '0 0 1 * 1'
";

#[test]
fn writes_the_container_the_exportable_timers_and_reports_the_rest() {
	let (_dir, file) = compose(BACKUP);
	let out = tempfile::tempdir().unwrap();
	let run = generate(&file, out.path());
	assert!(
		run.status.success(),
		"{}",
		String::from_utf8_lossy(&run.stderr)
	);
	assert_eq!(
		names(out.path()),
		[
			"proj-backup-1.timer",
			"proj-backup-2.timer",
			"proj-backup.container",
			"proj-db.container",
			"proj-default.network",
		]
	);

	let container = fs::read_to_string(out.path().join("proj-backup.container")).unwrap();
	assert!(
		container.contains("[Service]\nType=oneshot\n"),
		"{container}"
	);
	assert!(!container.contains("[Install]"), "{container}");
	assert!(container.contains("After=proj-db.service\n"), "{container}");
	assert!(
		container.contains("Environment=MODE=nightly\n"),
		"{container}"
	);

	let first = fs::read_to_string(out.path().join("proj-backup-1.timer")).unwrap();
	assert!(first.contains("OnCalendar=*-*-* 02:30:00\n"), "{first}");
	assert!(first.contains("Persistent=true\n"), "{first}");
	let second = fs::read_to_string(out.path().join("proj-backup-2.timer")).unwrap();
	assert!(
		second.contains("OnCalendar=Mon..Fri *-*-* 12:00:00 Europe/Madrid\n"),
		"{second}"
	);
	assert!(second.contains("Persistent=false\n"), "{second}");
	for timer in [&first, &second] {
		assert!(timer.contains("Unit=proj-backup.service\n"), "{timer}");
		assert!(timer.contains("WantedBy=timers.target\n"), "{timer}");
	}

	let err = String::from_utf8_lossy(&run.stderr);
	assert!(
		err.contains("job 'backup': schedule entry 3 was not exported: `concurrency: queue`"),
		"{err}"
	);
	assert!(
		err.contains("job 'backup': schedule entry 4 was not exported: it restricts both"),
		"{err}"
	);
	assert!(err.contains("2 .timer unit(s) were written"), "{err}");
}

#[test]
fn writing_twice_gives_the_same_files() {
	let (_dir, file) = compose(BACKUP);
	let out = tempfile::tempdir().unwrap();
	assert!(generate(&file, out.path()).status.success());
	let before: Vec<_> = names(out.path())
		.iter()
		.map(|n| fs::read_to_string(out.path().join(n)).unwrap())
		.collect();
	assert!(generate(&file, out.path()).status.success());
	let after: Vec<_> = names(out.path())
		.iter()
		.map(|n| fs::read_to_string(out.path().join(n)).unwrap())
		.collect();
	assert_eq!(before, after);
}

#[test]
fn a_manual_only_project_writes_no_job_unit() {
	let (_dir, file) = compose(
		"services:\n  web:\n    image: nginx\njobs:\n  once:\n    image: alpine\n    triggers: {manual: true}\n",
	);
	let out = tempfile::tempdir().unwrap();
	let run = generate(&file, out.path());
	assert!(run.status.success());
	assert_eq!(
		names(out.path()),
		["proj-default.network", "proj-web.container"]
	);
}

#[test]
fn sixty_four_schedules_write_sixty_four_timers() {
	let mut yaml =
		String::from("jobs:\n  many:\n    image: alpine\n    triggers:\n      schedule:\n");
	for minute in 0..64 {
		yaml.push_str(&format!(
			"        - '{} {} * * *'\n",
			minute % 60,
			minute % 24
		));
	}
	let (_dir, file) = compose(&yaml);
	let out = tempfile::tempdir().unwrap();
	assert!(generate(&file, out.path()).status.success());
	let found = names(out.path());
	assert_eq!(found.iter().filter(|n| n.ends_with(".timer")).count(), 64);
	assert!(found.contains(&"proj-many-64.timer".to_string()));
}

#[test]
fn a_service_and_a_job_that_sanitize_to_one_file_name_fail_instead_of_overwriting() {
	let (_dir, file) = compose(
		"services:\n  a_b:\n    image: nginx\njobs:\n  'a:b':\n    image: alpine\n    triggers:\n      schedule: ['0 3 * * *']\n",
	);
	let out = tempfile::tempdir().unwrap();
	let run = generate(&file, out.path());
	assert!(!run.status.success());
	assert!(String::from_utf8_lossy(&run.stderr).contains("proj-a_b.container"));
	assert!(
		names(out.path()).is_empty(),
		"nothing is written on a collision"
	);
}

#[test]
fn a_hostile_job_name_stays_inside_the_output_directory() {
	let (_dir, file) = compose(
		"jobs:\n  '../../escape':\n    image: alpine\n    triggers:\n      schedule: ['0 3 * * *']\n",
	);
	let root = tempfile::tempdir().unwrap();
	let out = root.path().join("out");
	let run = generate(&file, &out);
	assert!(
		run.status.success(),
		"{}",
		String::from_utf8_lossy(&run.stderr)
	);
	for name in names(&out) {
		assert!(!name.contains(".."), "{name}");
	}
	assert_eq!(
		names(root.path()),
		["out"],
		"nothing written beside the output dir"
	);
}

#[test]
fn the_stdout_form_lists_the_timers_too() {
	let (_dir, file) = compose(BACKUP);
	let run = Command::new(env!("CARGO_BIN_EXE_podup"))
		.args([
			"--socket",
			NO_ENGINE,
			"-p",
			"proj",
			"-f",
			file.to_str().unwrap(),
		])
		.args(["generate", "quadlet"])
		.env("NO_COLOR", "1")
		.output()
		.unwrap();
	let text = String::from_utf8_lossy(&run.stdout);
	assert!(text.contains("# proj-backup-1.timer\n"), "{text}");
	assert!(text.contains("# proj-backup.container\n"), "{text}");
}
