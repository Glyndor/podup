use super::*;

// --- an excluded pipe ---------------------------------------------------

/// A pipe the filter excludes is never opened or classified for sending, so it
/// does not stop the sync of the rest. The filter used to run only after the
/// walk had refused the pipe.
#[cfg(unix)]
#[test]
fn an_excluded_pipe_does_not_stop_the_sync() {
	let dir = tempdir().unwrap();
	let tree = dir.path().canonicalize().unwrap().join("tree");
	fs::create_dir(&tree).unwrap();
	fs::write(tree.join("ok.txt"), b"ok").unwrap();
	let status = std::process::Command::new("mkfifo")
		.arg(tree.join("ignored.pipe"))
		.status()
		.expect("mkfifo");
	assert!(status.success());
	let skip = |name: &Path| name.ends_with("ignored.pipe");
	let (gz, _) = sync_to_gz_with_sent(&tree, Path::new("tree"), &skip)
		.expect("an excluded pipe must not fail the sync");
	let names: Vec<String> = tar_entries(&gz).into_iter().map(|(n, _)| n).collect();
	assert!(names.contains(&"tree/ok.txt".to_string()), "{names:?}");
	assert!(
		!names.iter().any(|n| n.contains("ignored.pipe")),
		"{names:?}"
	);
}
