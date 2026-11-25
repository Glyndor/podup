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
	// The mode comes from `dry_run` and from nothing else: a published tag stops a release and not a
	// rehearsal. The step has no condition of its own that could turn it off.
	assert_eq!(
		step["env"]["MODE"],
		"${{ inputs.dry_run && 'rehearsal' || 'release' }}"
	);
	assert!(
		step.get("if").is_none(),
		"the check of a published release has no condition of its own"
	);
	assert_eq!(
		step["run"].as_str().unwrap().trim(),
		"bash .github/scripts/release/check-unpublished.sh \"$TAG\" \"$MODE\""
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
	use std::process::Command;
	let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
	let script = root.join(".github/scripts/release/check-unpublished.sh");
	// (published tags, exit status of the lookup, mode, expected status, text the output must hold)
	for (tags, api_rc, mode, expected_rc, diagnostic) in [
		("v1.2.3\nv1.2.4", "0", "release", 1, "already published"),
		("v1.2.3\nv1.2.4", "0", "rehearsal", 0, "publishes nothing"),
		("", "0", "release", 0, ""),
		("", "0", "rehearsal", 0, ""),
		("v1.2.30\nv1.2.4", "0", "release", 0, ""),
		("", "7", "release", 7, "lookup failed"),
		("", "7", "rehearsal", 7, "lookup failed"),
		("v1.2.3", "0", "dry_run", 2, "release or rehearsal"),
	] {
		let dir = tempfile::tempdir().unwrap();
		let args = dir.path().join("args");
		let bin = dir.path().join("bin");
		fs::create_dir(&bin).unwrap();
		let stub = bin.join("gh");
		fs::write(
			&stub,
			"#!/usr/bin/env bash\nprintf '%s\\n' \"$@\" > \"$GH_ARGS\"\nif [ \"$API_RC\" -ne 0 ]; then echo 'lookup failed' >&2; exit \"$API_RC\"; fi\nprintf '%s\\n' \"$PUBLISHED_TAGS\"\n",
		)
		.unwrap();
		{
			use std::os::unix::fs::PermissionsExt;
			fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
		}
		let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
		let output = Command::new("bash")
			.arg(&script)
			.arg("v1.2.3")
			.arg(mode)
			.env("PATH", path)
			.env("GH_ARGS", &args)
			.env("GH_REPO", "example/repo")
			.env("API_RC", api_rc)
			.env("PUBLISHED_TAGS", tags)
			.env_remove("PODUP_PUBLISHED_TAGS")
			.output()
			.unwrap();
		assert_eq!(output.status.code(), Some(expected_rc), "{output:?}");
		let text = format!(
			"{}{}",
			String::from_utf8_lossy(&output.stdout),
			String::from_utf8_lossy(&output.stderr)
		);
		assert!(text.contains(diagnostic), "{text}");
		if mode != "dry_run" {
			assert_eq!(
				fs::read_to_string(args).unwrap(),
				"api\n--paginate\nrepos/example/repo/releases\n--jq\n.[] | select(.draft == false) | .tag_name\n"
			);
		}
	}
}
