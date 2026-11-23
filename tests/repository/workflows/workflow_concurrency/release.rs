use super::*;

// The step is a bash script that the test runs against a stub of `gh`; the
// Windows runners have no stub-friendly shell, and the step only ever runs on
// Linux, so the behaviour is checked on Unix.
#[cfg(unix)]
#[test]
fn release_rejects_published_tags_before_building() {
	let doc = workflow("reusable-release-verify");
	let steps = doc["jobs"]["verify"]["steps"].as_sequence().unwrap();
	let index = steps
		.iter()
		.position(|s| s["name"].as_str() == Some("Refuse an already published release"))
		.expect("release checks publication before building");
	let step = &steps[index];
	assert_eq!(step["env"]["TAG"], "${{ inputs.tag }}");
	assert!(
		step.get("if").is_none(),
		"a release that is already published is refused in a rehearsal as well"
	);
	assert_eq!(step["env"]["GH_REPO"], "${{ github.repository }}");
	// The audit installs a tool of other people's, so it is a job of its own that waits for this one:
	// the publication lookup precedes it, and nothing in this job installs anything.
	assert_eq!(doc["jobs"]["audit"]["needs"], "verify");
	assert!(
		!steps.iter().any(|s| s["run"]
			.as_str()
			.is_some_and(|r| r.contains("cargo install"))),
		"the job that names the commit installs a tool"
	);
	let script = step["run"].as_str().unwrap();
	use std::process::Command;
	for (tags, api_rc, expected_rc, diagnostic) in [
		("v1.2.3\nv1.2.4", "0", 1, "already published"),
		("", "0", 0, ""),
		("v1.2.30\nv1.2.4", "0", 0, ""),
		("", "7", 7, "lookup failed"),
	] {
		let dir = tempfile::tempdir().unwrap();
		let args = dir.path().join("args");
		let stub = r#"
gh() {
  printf '%s\n' "$@" > "$GH_ARGS"
  if [ "$API_RC" -ne 0 ]; then echo 'lookup failed' >&2; return "$API_RC"; fi
  printf '%s\n' "$PUBLISHED_TAGS"
}
"#;
		let output = Command::new("bash")
			.arg("-c")
			.arg(format!("{stub}\n{script}"))
			.env("GH_ARGS", &args)
			.env("GH_REPO", "example/repo")
			.env("TAG", "v1.2.3")
			.env("API_RC", api_rc)
			.env("PUBLISHED_TAGS", tags)
			.output()
			.unwrap();
		assert_eq!(output.status.code(), Some(expected_rc), "{output:?}");
		let text = format!(
			"{}{}",
			String::from_utf8_lossy(&output.stdout),
			String::from_utf8_lossy(&output.stderr)
		);
		assert!(text.contains(diagnostic), "{text}");
		assert_eq!(
			fs::read_to_string(args).unwrap(),
			"api\n--paginate\nrepos/example/repo/releases\n--jq\n.[] | select(.draft == false) | .tag_name\n"
		);
	}
}
