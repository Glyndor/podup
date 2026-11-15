//! A bind source created outside the project directory is reported.

use std::sync::{Arc, Mutex};

use super::*;

struct Sink(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Sink {
	fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
		self.0.lock().unwrap().extend_from_slice(bytes);
		Ok(bytes.len())
	}
	fn flush(&mut self) -> std::io::Result<()> {
		Ok(())
	}
}

/// The warnings `build_mounts_all` logs for one short-form bind of `src`.
fn warnings_for(src: &str, project: &Path) -> String {
	let bytes = Arc::new(Mutex::new(Vec::new()));
	let writer = bytes.clone();
	let subscriber = tracing_subscriber::fmt()
		.without_time()
		.with_ansi(false)
		.with_writer(move || Sink(writer.clone()))
		.finish();
	tracing::subscriber::with_default(subscriber, || {
		build_mounts_all(
			&svc_with_volumes(vec![VolumeMount::Short(format!("{src}:/data"))]),
			project,
		);
	});
	let text = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
	text
}

#[test]
fn a_bind_that_climbs_out_of_the_project_is_reported() {
	let root = tempfile::tempdir().unwrap();
	let project = root.path().join("project");
	std::fs::create_dir(&project).unwrap();
	let text = warnings_for("../outside", &project);
	assert!(
		root.path().join("outside").is_dir(),
		"the directory is still created"
	);
	assert!(text.contains("outside the project"), "{text}");
}

#[cfg(unix)]
#[test]
fn a_bind_through_a_symlink_out_of_the_project_is_reported() {
	let root = tempfile::tempdir().unwrap();
	let project = root.path().join("project");
	let elsewhere = root.path().join("elsewhere");
	std::fs::create_dir(&project).unwrap();
	std::fs::create_dir(&elsewhere).unwrap();
	std::os::unix::fs::symlink(&elsewhere, project.join("link")).unwrap();
	let text = warnings_for("./link/new", &project);
	assert!(elsewhere.join("new").is_dir());
	assert!(text.contains("outside the project"), "{text}");
}

#[test]
fn a_bind_inside_the_project_or_already_there_says_nothing() {
	let root = tempfile::tempdir().unwrap();
	let project = root.path().join("project");
	std::fs::create_dir(&project).unwrap();
	assert!(!warnings_for("./data", &project).contains("outside the project"));
	// Already present: nothing is created, so nothing is reported.
	std::fs::create_dir(root.path().join("there")).unwrap();
	assert!(!warnings_for("../there", &project).contains("outside the project"));
}
