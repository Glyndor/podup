//! The asset-contract fixture installs what the release signs with, from one file.
//!
//! `sign-requirements.txt` holds the versions and hashes of `cryptography` and
//! the two packages it needs, and Dependabot keeps it current. The fixture used
//! to write the same three versions inline, twice, and the copies drifted
//! (`cryptography` 50.0.0 against 50.0.2, `pycparser` 3.0 against 3.12) without
//! anything saying so. This fails when a version is written in the workflow
//! again, when an install step stops reading the file, or when a change to the
//! file would not run the fixture.

use std::fs;
use std::path::Path;

fn repo(rel: &str) -> String {
	let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
	fs::read_to_string(&path).unwrap_or_else(|e| panic!("{rel} is readable: {e}"))
}

const MANIFEST: &str = ".github/scripts/release/sign-requirements.txt";
const PACKAGES: [&str; 3] = ["cryptography", "cffi", "pycparser"];

/// One line per problem found in `workflow` against the packages in `manifest`.
fn problems(workflow: &str, manifest: &str) -> Vec<String> {
	let mut out = Vec::new();
	for name in PACKAGES {
		if !manifest
			.lines()
			.any(|l| l.starts_with(&format!("{name}==")))
		{
			out.push(format!(
				"{MANIFEST} does not pin {name}; this test would check nothing"
			));
		}
	}
	for (number, line) in workflow.lines().enumerate() {
		let code = line.split('#').next().unwrap_or("");
		for name in PACKAGES {
			if code.contains(&format!("{name}==")) || code.contains(&format!("'{name}==")) {
				out.push(format!(
					"line {}: {name} is pinned here: {}",
					number + 1,
					line.trim()
				));
			}
		}
	}
	let installs: Vec<&str> = workflow
		.lines()
		.filter(|l| l.contains("pip install") && l.contains("fixturevenv"))
		.collect();
	if installs.len() != 2 {
		out.push(format!(
			"expected the shell and the PowerShell install of the fixture venv, found {}",
			installs.len()
		));
	}
	for line in installs {
		if !line.contains(&format!("-r {MANIFEST}")) {
			out.push(format!(
				"an install does not read {MANIFEST}: {}",
				line.trim()
			));
		}
	}
	// Both `paths:` filters (push and pull request) must list the manifest.
	let listed = workflow
		.lines()
		.filter(|l| l.trim() == format!("- \"{MANIFEST}\""))
		.count();
	if listed != 2 {
		out.push(format!(
			"{MANIFEST} is in {listed} of the 2 `paths:` filters"
		));
	}
	out
}

#[test]
fn the_fixture_installs_from_the_signing_manifest() {
	let found = problems(
		&repo(".github/workflows/asset-contract.yml"),
		&repo(MANIFEST),
	);
	assert!(found.is_empty(), "\n{}\n", found.join("\n"));
}

#[test]
fn an_inline_pin_a_stale_install_and_a_missing_path_are_each_reported() {
	let manifest = "cffi==1\ncryptography==2\npycparser==3\n";
	let good = format!(
		"  - \"{MANIFEST}\"\n  - \"{MANIFEST}\"\n\
		 pip install fixturevenv -r {MANIFEST}\npip install fixturevenv -r {MANIFEST}\n"
	);
	assert!(problems(&good, manifest).is_empty());
	let pinned = good.replace("-r", "'cryptography==50.0.0' -r");
	assert!(problems(&pinned, manifest)
		.iter()
		.any(|p| p.contains("pinned here")));
	let stale = good.replacen(&format!("-r {MANIFEST}"), "pycparser==3.0", 1);
	assert!(problems(&stale, manifest).len() >= 2);
	let unlisted = good.replacen(&format!("  - \"{MANIFEST}\"\n"), "", 1);
	assert!(problems(&unlisted, manifest)
		.iter()
		.any(|p| p.contains("of the 2")));
}
