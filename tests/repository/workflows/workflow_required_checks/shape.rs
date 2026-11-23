//! Every required status check still names a job that exists.
//!
//! GitHub emits a check called `<caller job id> / <inner job name>` for a job
//! that calls a reusable, and just `<job name>` for a job defined in the
//! workflow itself. `CONTRIBUTING.md` says it in the repository's own words:
//! "Job ids are load-bearing". Rename either half and the emitted name
//! changes, the ruleset still requires the old string, and every pull request
//! sits BLOCKED with nothing anywhere saying why. The rename is a one-word
//! edit that no test in this tree read: `Supported Podman majors`,
//! `rust / Coverage` and `develop-only` appeared zero times under `tests/`.
//!
//! The same defect was measured in `Glyndor/apt` before `ce59d2c` closed it
//! there: renaming `shell:` to `shellx:` in its `tests.yml` left every suite
//! green while two required checks stopped being reported.
//!
//! The list below is a LITERAL, on purpose, and the acceptance for the issue
//! says so: a list derived from the tree cannot notice a job that was removed,
//! because the derivation removes the name at the same time. It mirrors the
//! branch rulesets and has to be updated WITH them. Changing the ruleset and
//! not this list leaves a real required check unverified; changing this list
//! and not the ruleset guards a phantom GitHub will never report.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// The checks required on `main`, exactly as GitHub emits them.
///
/// Two notes that the issue text and the tree disagree on, recorded here
/// rather than in a commit message nobody reads twice:
///
/// `rust / Format & lint` is what `reusable-rust-ci.yml` emits. The issue
/// wrote `rust / Format`, and no job in this tree is named `Format`. A
/// required check that is never reported blocks every pull request, and
/// pull requests merge on both branches, so `rust / Format` cannot be the
/// required string. If the ruleset does say `Format`, the repository is
/// already in the trap this file exists to close and the RULESET is the
/// side that needs the edit.
///
/// The issue counts twelve on `main` and eleven on `develop` and names
/// eleven, one of which is `main`-only. Eleven minus one is ten, so one
/// required name is missing from the issue text. It resolves today, since
/// both branches have merged recently, so it is not a phantom. It could not
/// be identified without reading the ruleset itself, which needs the
/// Settings UI. The gap is real and is reported, not papered over: when the
/// twelfth name is read off the ruleset it belongs in this list.
pub const REQUIRED_ON_MAIN: &[&str] = &[
	"rust / Format & lint",
	"rust / Test",
	"rust / Coverage",
	"rust / Doc warnings",
	"rust / MSRV",
	"rust / Extra platforms",
	// The gate that certifies every skip of the Rust pipeline. The six names
	// above it are the suites it covers; the ruleset moves to the gate once
	// this job is on `develop`, and the suites leave this list then.
	"rust / Gate",
	// What a pull request into `main` is asked for. The same commit has just been
	// through the merge queue and a push to `develop`, whose runs carry a plan that
	// looked at a narrower range; the promotion has its own names, produced only by
	// a pull request whose base is `main`, which runs every suite.
	"rust / Gate (promotion)",
	"line-limit / line limit (promotion)",
	"workflow-lint / workflow-lint (promotion)",
	"dco / Signed-off-by present on every commit (promotion)",
	"Supported Podman majors (promotion)",
	"Supported Podman majors",
	"line-limit / line limit",
	"dco / Signed-off-by present on every commit",
	"workflow-lint / workflow-lint",
	"develop-only / develop-only",
];

/// Required on `main` and NOT on `develop`.
///
/// `main-guard.yml` triggers only on pull requests into `main`, so on a
/// `develop` pull request the check is never reported at all. Requiring it
/// on `develop` would block every merge there with an unanswerable check,
/// which is what `main-guard.yml`'s own header explains. Flattening the two
/// lists into one would hide that asymmetry, so they are separate.
const MAIN_ONLY: &[&str] = &[
	"develop-only / develop-only",
	"rust / Gate (promotion)",
	"line-limit / line limit (promotion)",
	"workflow-lint / workflow-lint (promotion)",
	"dco / Signed-off-by present on every commit (promotion)",
	"Supported Podman majors (promotion)",
];

/// One job, as much of it as the check name depends on.
pub struct Job {
	pub name: Option<String>,
	pub uses: Option<String>,
}

/// Read the `jobs:` block of a workflow: job id to its `name:` and `uses:`.
///
/// This reads the file with a real YAML parser. The reader that was here walked
/// the text by indentation and ignored every line it did not recognise, which
/// is how a job written some other way, or a key written twice, could be seen
/// one way here and run another way by GitHub. `serde_yaml` refuses a duplicate
/// key, so a workflow that has one fails here instead of being read as its
/// last value.
pub fn jobs_of(workflow: &str) -> BTreeMap<String, Job> {
	let spec: serde_yaml::Value = serde_yaml::from_str(workflow)
		.unwrap_or_else(|e| panic!("the workflow does not parse: {e}"));
	let mut jobs = BTreeMap::new();
	let Some(map) = spec.get("jobs").and_then(serde_yaml::Value::as_mapping) else {
		return jobs;
	};
	for (id, job) in map {
		let id = id.as_str().expect("a job id is a string").to_string();
		let field = |key: &str| {
			job.get(key)
				.and_then(serde_yaml::Value::as_str)
				.map(str::to_string)
		};
		// A name may end in an expression that says which kind of run produced
		// it; the name a ruleset requires is what comes before.
		let name = field("name").map(|n| match n.find("${{") {
			Some(at) => n[..at].trim_end().to_string(),
			None => n,
		});
		jobs.insert(
			id,
			Job {
				name,
				uses: field("uses"),
			},
		);
	}
	jobs
}

/// Every `*.yml` in a workflows directory, keyed by file name.
pub fn workflows(dir: &Path) -> BTreeMap<String, BTreeMap<String, Job>> {
	let mut out = BTreeMap::new();
	let entries =
		fs::read_dir(dir).unwrap_or_else(|e| panic!("{} is readable: {e}", dir.display()));
	for entry in entries {
		let path = entry.expect("dir entry").path();
		if path.extension().map(|e| e == "yml") != Some(true) {
			continue;
		}
		let base = path
			.file_name()
			.and_then(|n| n.to_str())
			.expect("file name")
			.to_string();
		let body = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{base} is readable: {e}"));
		out.insert(base, jobs_of(&body));
	}
	out
}

/// Report every required check whose emitted name no longer resolves.
///
/// One line per unresolved check, naming BOTH halves so a rename of either is
/// reported by the name that stopped being emitted rather than by a count:
///
/// ```text
/// rust / Coverage: no workflow pairs job id 'rust' with a job named 'Coverage'
/// ```
///
/// Empty when every check is in place. A reusable never emits a required check
/// itself, so callers only are walked for the first half.
pub fn unresolved(
	tree: &BTreeMap<String, BTreeMap<String, Job>>,
	required: &[&str],
) -> Vec<String> {
	let mut out = Vec::new();
	for named in required {
		// A promotion name is the job's name plus the suffix its expression adds.
		let check: &str = named.strip_suffix(" (promotion)").unwrap_or(named);
		let (job_id, job_name) = match check.split_once(" / ") {
			Some((id, name)) => (Some(id), name),
			// No slash: a job defined in the workflow itself, whose check name
			// is its `name:` alone. `Supported Podman majors` is this shape.
			None => (None, check),
		};
		let mut found = false;
		for (file, jobs) in tree {
			if file.starts_with("reusable-") {
				continue;
			}
			let candidates: Vec<&Job> = match job_id {
				Some(id) => jobs.get(id).into_iter().collect(),
				None => jobs.values().collect(),
			};
			for job in candidates {
				// Direct shape: the job carries the name itself.
				if job.name.as_deref() == Some(job_name) {
					found = true;
					break;
				}
				// Caller shape: follow `uses:` and look for an inner job whose
				// `name:` is the second half. Only valid for a two-part name;
				// a caller emits `<id> / <inner>`, never the inner alone.
				if job_id.is_none() {
					continue;
				}
				let Some(uses) = job.uses.as_deref() else {
					continue;
				};
				let target = uses
					.trim_start_matches("./")
					.rsplit('/')
					.next()
					.unwrap_or(uses);
				let Some(inner) = tree.get(target) else {
					continue;
				};
				if inner.values().any(|j| j.name.as_deref() == Some(job_name)) {
					found = true;
					break;
				}
			}
			if found {
				break;
			}
		}
		if !found {
			out.push(match job_id {
				Some(id) => format!(
					"{named}: no workflow pairs job id '{id}' with a job named '{job_name}'"
				),
				None => format!("{named}: no workflow has a job named '{job_name}'"),
			});
		}
	}
	out
}

/// Read a workflow with line endings normalised to `\n`.
///
/// The Windows runner checks the tree out with CRLF, so a multi-line needle
/// written with `\n` does not appear in the file text and every plant would
/// report "this file no longer carries what the case plants against" on
/// Windows only. `str::lines` hides this from the parser, which is why the
/// tests over the real tree pass there and the plants did not.
pub fn read_workflow(path: &Path) -> String {
	fs::read_to_string(path)
		.unwrap_or_else(|e| panic!("{} is readable: {e}", path.display()))
		.replace("\r\n", "\n")
}

pub fn workflows_dir() -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows")
}

fn required_on_develop() -> Vec<&'static str> {
	REQUIRED_ON_MAIN
		.iter()
		.copied()
		.filter(|c| !MAIN_ONLY.contains(c))
		.collect()
}

#[test]
fn every_check_required_on_main_is_emitted_by_a_job_that_exists() {
	let tree = workflows(&workflows_dir());
	let gone = unresolved(&tree, REQUIRED_ON_MAIN);
	assert!(
		gone.is_empty(),
		"the ruleset on main requires status checks no job emits any more. Each \
		 one leaves every pull request into main BLOCKED with nothing reporting \
		 the check. Either restore the name or change the ruleset and this \
		 list together:\n  {}",
		gone.join("\n  ")
	);
}

#[test]
fn every_check_required_on_develop_is_emitted_by_a_job_that_exists() {
	let tree = workflows(&workflows_dir());
	let gone = unresolved(&tree, &required_on_develop());
	assert!(
		gone.is_empty(),
		"the ruleset on develop requires status checks no job emits any more:\n  {}",
		gone.join("\n  ")
	);
}

/// The two branches require different sets, and the difference is not
/// cosmetic: `main-guard.yml` runs only on pull requests into `main`, so
/// `develop-only / develop-only` is never reported on a `develop` pull
/// request. One flattened list would either miss the check on `main` or
/// block every merge into `develop` on a check that cannot arrive.
#[test]
fn develop_requires_the_main_set_minus_the_main_only_checks() {
	let develop = required_on_develop();
	assert_eq!(
		develop.len(),
		REQUIRED_ON_MAIN.len() - MAIN_ONLY.len(),
		"every main-only check must be in the main list to be subtracted from it"
	);
	for check in MAIN_ONLY {
		assert!(
			REQUIRED_ON_MAIN.contains(check),
			"{check} is listed as main-only but is not required on main"
		);
		assert!(
			!develop.contains(check),
			"{check} is required on develop, where main-guard.yml never reports it"
		);
	}

	// The asymmetry has to hold in the workflow too, not only in this list.
	let body = fs::read_to_string(workflows_dir().join("main-guard.yml"))
		.expect("main-guard.yml is readable");
	assert!(
		body.contains("branches: [\"main\"]"),
		"main-guard.yml no longer scopes its pull_request trigger to main. It \
		 would then report develop-only on develop pull requests too, and the \
		 split between these two lists stops describing anything."
	);
}
