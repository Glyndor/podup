use super::*;
use std::process::Command;

#[test]
fn release_rejects_published_tags_before_building() {
	let doc = workflow("release");
	let steps = doc["jobs"]["verify"]["steps"].as_sequence().unwrap();
	let index = steps
		.iter()
		.position(|s| s["name"].as_str() == Some("Refuse an already published release"))
		.expect("release checks publication before building");
	let step = &steps[index];
	assert_eq!(
		step["env"]["TAG"],
		"${{ github.event_name == 'workflow_dispatch' && inputs.tag || github.ref_name }}"
	);
	assert_eq!(step["env"]["GH_REPO"], "${{ github.repository }}");
	let audit = steps
		.iter()
		.position(|s| s["name"].as_str() == Some("Audit dependencies for known advisories"))
		.unwrap();
	assert!(
		index < audit,
		"publication lookup precedes expensive checks"
	);
	let script = step["run"].as_str().unwrap();
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
		assert_eq!(fs::read_to_string(args).unwrap(), "api\n--paginate\nrepos/example/repo/releases\n--jq\n.[] | select(.draft == false) | .tag_name\n");
	}
}
