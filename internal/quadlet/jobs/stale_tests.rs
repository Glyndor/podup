use super::*;

fn unit(name: &str) -> QuadletUnit {
	QuadletUnit {
		filename: name.to_string(),
		contents: String::new(),
	}
}

fn write(dir: &Path, name: &str, owner: &str) {
	std::fs::write(dir.join(name), format!("# podup-owner: {owner}\n[Timer]\n")).unwrap();
}

#[test]
fn names_owned_timers_that_this_run_does_not_write() {
	let dir = tempfile::tempdir().unwrap();
	write(dir.path(), "proj-job-1.timer", "proj");
	write(dir.path(), "proj-job-2.timer", "proj");
	write(dir.path(), "other-job-1.timer", "other");
	std::fs::write(dir.path().join("proj-job-3.timer"), "[Timer]\n").unwrap();
	std::fs::write(
		dir.path().join("proj-web.container"),
		"# podup-owner: proj\n",
	)
	.unwrap();
	let written = [unit("proj-job-1.timer")];
	assert_eq!(
		stale_timers(dir.path(), "proj", &written),
		["proj-job-2.timer"]
	);
}

#[test]
fn a_missing_directory_has_no_stale_timers() {
	let dir = tempfile::tempdir().unwrap();
	assert!(stale_timers(&dir.path().join("absent"), "proj", &[]).is_empty());
}

#[test]
fn an_oversized_file_is_not_read() {
	let dir = tempfile::tempdir().unwrap();
	let mut text = String::from("# podup-owner: proj\n");
	text.push_str(&"x".repeat(MAX_TIMER_BYTES as usize + 1));
	std::fs::write(dir.path().join("proj-job-1.timer"), text).unwrap();
	assert!(stale_timers(dir.path(), "proj", &[]).is_empty());
}
