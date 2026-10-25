#![cfg(unix)]

//! Paths named by a compose file, and paths podup writes, seen from the
//! command line with `HOME` and every `XDG_*` variable pointed at a temp dir.
//!
//! The compose file is the untrusted party here. It can name a FIFO, a device
//! or a directory where a file is expected, ask for a host path that resolves
//! to somewhere sensitive, or carry service names meant to become file names.
//! Each case checks that the process ends inside a budget, refuses cleanly
//! where it should, and writes nothing outside the directory it was given.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use tempfile::TempDir;

const BUDGET: Duration = Duration::from_secs(30);
const DEAD_SOCKET: &str = "unix:///nonexistent/podup-hostile-fs-test.sock";

struct Outcome {
	code: Option<i32>,
	text: String,
}

/// A temp dir that stands in for the whole user environment.
struct Sandbox {
	root: TempDir,
}

impl Sandbox {
	fn new() -> Self {
		let root = tempfile::tempdir().expect("tempdir");
		for dir in ["home", "config", "data", "project"] {
			fs::create_dir(root.path().join(dir)).expect("mkdir");
		}
		Self { root }
	}

	fn project(&self) -> PathBuf {
		self.root.path().join("project")
	}

	fn write(&self, name: &str, contents: &str) -> PathBuf {
		let path = self.project().join(name);
		fs::write(&path, contents).expect("write");
		path
	}

	/// Run `podup -p proj -f <file> <args>` with the environment confined.
	fn run(&self, file: &Path, args: &[&str]) -> Outcome {
		let start = Instant::now();
		let mut child = Command::new(env!("CARGO_BIN_EXE_podup"))
			.args(["-p", "proj", "-f", file.to_str().unwrap()])
			.args(args)
			.current_dir(self.project())
			.env("HOME", self.root.path().join("home"))
			.env("XDG_CONFIG_HOME", self.root.path().join("config"))
			.env("XDG_DATA_HOME", self.root.path().join("data"))
			.env("XDG_RUNTIME_DIR", self.root.path().join("home"))
			.env("DOCKER_HOST", DEAD_SOCKET)
			.env("CONTAINER_HOST", DEAD_SOCKET)
			.env_remove("PODMAN_SOCKET")
			.stdout(Stdio::piped())
			.stderr(Stdio::piped())
			.spawn()
			.expect("spawn podup");
		let (mut out, mut err) = (child.stdout.take().unwrap(), child.stderr.take().unwrap());
		let drain = std::thread::spawn(move || {
			use std::io::Read;
			let (mut a, mut b) = (String::new(), String::new());
			let _ = out.read_to_string(&mut a);
			let _ = err.read_to_string(&mut b);
			a + &b
		});
		let status = loop {
			if let Some(status) = child.try_wait().expect("poll") {
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
			code: status.and_then(|s| s.code()),
			text: drain.join().expect("join"),
		}
	}
}

fn mkfifo(path: &Path) {
	assert!(Command::new("mkfifo").arg(path).status().unwrap().success());
}

#[test]
fn special_files_named_as_env_file_or_include_are_refused_without_blocking() {
	let sb = Sandbox::new();
	mkfifo(&sb.project().join("pipe"));
	fs::create_dir(sb.project().join("adir")).unwrap();
	std::os::unix::fs::symlink("/dev/zero", sb.project().join("zero")).unwrap();
	for target in ["pipe", "adir", "zero", "/dev/zero", "/dev/stdin"] {
		let env_file = sb.write(
			"env.yaml",
			&format!("services:\n  web:\n    image: alpine\n    env_file: [{target}]\n"),
		);
		let include = sb.write(
			"inc.yaml",
			&format!("include:\n  - {target}\nservices:\n  web:\n    image: alpine\n"),
		);
		for (what, file) in [("env_file", env_file), ("include", include)] {
			let outcome = sb.run(&file, &["config"]);
			assert!(outcome.code.is_some(), "{what} {target}: blocked or killed");
			assert_ne!(outcome.code, Some(0), "{what} {target}: accepted");
			assert!(
				outcome.text.contains("refusing to read"),
				"{what} {target}: {}",
				outcome.text
			);
			assert!(!outcome.text.contains("panicked"), "{}", outcome.text);
		}
	}
}

#[test]
fn a_fifo_named_dot_env_in_the_project_directory_does_not_block_the_loader() {
	let sb = Sandbox::new();
	mkfifo(&sb.project().join(".env"));
	let file = sb.write("c.yaml", "services:\n  web:\n    image: alpine\n");
	let outcome = sb.run(&file, &["config"]);
	assert_eq!(outcome.code, Some(0), "{}", outcome.text);
}

#[test]
fn audit_flags_a_sensitive_bind_in_every_spelling_that_stays_absolute() {
	for source in [
		"/etc",
		"/etc/",
		"//etc",
		"/etc/ssh",
		"/proc",
		"/var/run/docker.sock",
	] {
		let sb = Sandbox::new();
		let file = sb.write(
			"c.yaml",
			&format!(
				"services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - \"{source}:/m\"\n"
			),
		);
		let outcome = sb.run(&file, &["audit", "--format", "json"]);
		assert!(outcome.code.is_some());
		assert!(
			outcome.text.contains("sensitive_bind_mount"),
			"{source} was not flagged:\n{}",
			outcome.text
		);
	}
}

#[test]
fn generated_unit_names_stay_inside_the_output_directory() {
	let sb = Sandbox::new();
	let victim = sb.project().join("victim");
	fs::write(&victim, "precious").unwrap();
	let file = sb.write(
		"c.yaml",
		"services:\n  \"../../victim\":\n    image: alpine\n    environment:\n      PW: hunter2\n\
		 \x20 \"a/b\":\n    image: alpine\n  \"..\":\n    image: alpine\n\
		 volumes:\n  \"../v\":\nnetworks:\n  \"../n\":\n",
	);
	let out = sb.root.path().join("out");
	let outcome = sb.run(&file, &["generate", "quadlet", "-o", out.to_str().unwrap()]);
	assert_eq!(outcome.code, Some(0), "{}", outcome.text);
	assert_eq!(fs::read(&victim).unwrap(), b"precious");
	let mut seen = 0;
	for entry in fs::read_dir(&out).unwrap() {
		let path = entry.unwrap().path();
		assert!(path.is_file() && path.parent() == Some(out.as_path()));
		seen += 1;
	}
	assert!(seen >= 5, "expected a unit per name, found {seen}");
	for dir in ["home", "config", "data"] {
		assert_eq!(
			fs::read_dir(sb.root.path().join(dir)).unwrap().count(),
			0,
			"{dir} was written to"
		);
	}
	assert!(!sb.root.path().join("v").exists() && !sb.root.path().join("n").exists());
}

#[test]
fn units_that_carry_environment_values_are_private_even_under_a_loose_umask() {
	use std::os::unix::fs::PermissionsExt;
	let sb = Sandbox::new();
	let file = sb.write(
		"c.yaml",
		"services:\n  web:\n    image: alpine\n    environment:\n      PW: hunter2\n",
	);
	let out = sb.root.path().join("out");
	let status = Command::new("sh")
		.arg("-c")
		.arg("umask 000; exec \"$0\" \"$@\"")
		.arg(env!("CARGO_BIN_EXE_podup"))
		.args(["-p", "proj", "-f", file.to_str().unwrap()])
		.args(["generate", "quadlet", "-o", out.to_str().unwrap()])
		.env("DOCKER_HOST", DEAD_SOCKET)
		.stdout(Stdio::null())
		.stderr(Stdio::null())
		.status()
		.unwrap();
	assert!(status.success());
	for entry in fs::read_dir(&out).unwrap() {
		let mode = entry.unwrap().metadata().unwrap().permissions().mode() & 0o777;
		assert_eq!(mode & 0o077, 0, "a unit is readable by others: {mode:o}");
	}
}

#[test]
fn autostart_dry_run_writes_nothing_under_the_confined_home_and_xdg_dirs() {
	let sb = Sandbox::new();
	let file = sb.write("c.yaml", "services:\n  web:\n    image: alpine\n");
	let outcome = sb.run(&file, &["autostart", "install", "--dry-run", "--no-start"]);
	assert!(outcome.code.is_some(), "{}", outcome.text);
	for dir in ["home", "config", "data"] {
		assert_eq!(
			fs::read_dir(sb.root.path().join(dir)).unwrap().count(),
			0,
			"{dir}: a dry run wrote something"
		);
	}
}
