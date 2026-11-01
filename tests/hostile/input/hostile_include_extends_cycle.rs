//! An external include whose service extends back through an external file.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use tempfile::TempDir;

const BUDGET: Duration = Duration::from_secs(30);
const MAX_DIAGNOSTIC_BYTES: usize = 16 * 1024;
const DEAD_SOCKET: &str = "unix:///nonexistent/podup-hostile-compose-test.sock";

struct Outcome {
	code: Option<i32>,
	text: String,
	took: Duration,
}

fn write(dir: &Path, name: &str, contents: &str) -> PathBuf {
	let path = dir.join(name);
	fs::write(&path, contents).expect("write compose file");
	path
}

fn config(file: &Path) -> Outcome {
	let start = Instant::now();
	let mut child = Command::new(env!("CARGO_BIN_EXE_podup"))
		.args(["-f", file.to_str().unwrap(), "config", "--format", "json"])
		.env("DOCKER_HOST", DEAD_SOCKET)
		.env("CONTAINER_HOST", DEAD_SOCKET)
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.expect("spawn podup");
	let (mut out, mut err) = (child.stdout.take().unwrap(), child.stderr.take().unwrap());
	let drain = std::thread::spawn(move || {
		use std::io::Read;
		let (mut stdout, mut stderr) = (String::new(), String::new());
		let _ = out.read_to_string(&mut stdout);
		let _ = err.read_to_string(&mut stderr);
		stdout + &stderr
	});
	let status = loop {
		if let Some(status) = child.try_wait().expect("poll podup") {
			break Some(status);
		}
		if start.elapsed() > BUDGET {
			let _ = child.kill();
			let _ = child.wait();
			break None;
		}
		std::thread::sleep(Duration::from_millis(20));
	};
	Outcome {
		code: status.and_then(|status| status.code()),
		text: drain.join().expect("join output reader"),
		took: start.elapsed(),
	}
}

fn project_tree(dir: &TempDir, base_extends: &str) -> PathBuf {
	let root = dir.path();
	let project = root.join("project");
	let shared = root.join("shared");
	let base = root.join("base");
	fs::create_dir_all(&project).unwrap();
	fs::create_dir_all(&shared).unwrap();
	fs::create_dir_all(&base).unwrap();
	write(
		&project,
		"compose.yaml",
		"include:\n  - ../shared/child.yaml\nservices:\n  project:\n    image: project-image\n",
	);
	write(
		&shared,
		"child.yaml",
		"services:\n  child:\n    extends:\n      file: ../base/base.yaml\n      service: base\n",
	);
	write(
		&base,
		"base.yaml",
		&format!(
			"services:\n  base:\n    image: alpine\n    environment:\n      FROM_BASE: inherited-marker\n{base_extends}"
		),
	);
	project.join("compose.yaml")
}

#[test]
fn an_external_include_and_extends_cycle_is_reported() {
	let valid = TempDir::new().expect("create valid tree");
	let compose = project_tree(&valid, "");
	let outcome = config(&compose);
	assert_eq!(
		outcome.code,
		Some(0),
		"valid external chain failed:\n{}",
		outcome.text
	);
	assert!(
		outcome.took < BUDGET,
		"valid external chain took {:?}",
		outcome.took
	);
	assert!(
		outcome.text.contains("inherited-marker"),
		"the included service did not inherit its base value:\n{}",
		outcome.text
	);

	let cyclic = TempDir::new().expect("create cyclic tree");
	let compose = project_tree(
		&cyclic,
		"    extends:\n      file: ../shared/child.yaml\n      service: child\n",
	);
	let outcome = config(&compose);
	assert!(outcome.code.is_some(), "cycle exceeded {BUDGET:?}");
	assert!(outcome.took < BUDGET, "cycle took {:?}", outcome.took);
	assert!(
		outcome
			.text
			.to_lowercase()
			.contains("circular extends.file"),
		"expected the circular extends.file diagnostic, got:\n{}",
		outcome.text
	);
	assert_ne!(outcome.code, Some(0), "the cycle was accepted");
	assert!(
		outcome.text.len() <= MAX_DIAGNOSTIC_BYTES,
		"cycle diagnostic exceeded {MAX_DIAGNOSTIC_BYTES} bytes ({} bytes)",
		outcome.text.len()
	);
}
