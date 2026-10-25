//! Profile filtering for jobs: a profiled job is ignored unless its profile is
//! active, and activation of what it needs never leaks into what `up` starts.

use std::collections::HashSet;

use crate::compose::parse_str;
use crate::compose::types::ComposeFile;
use crate::engine::project::profiles::{
	enabled_profile_services, retain_active_profiles_with_targets,
};

const FILE: &str = "\
services:
  web:
    image: nginx
  db:
    image: postgres
    profiles: [data]
jobs:
  plain:
    image: alpine
    triggers: {manual: true}
  ops-job:
    image: alpine
    profiles: [ops]
    depends_on: [db]
    triggers: {manual: true}
  chained:
    image: alpine
    profiles: [chain]
    depends_on: [ops-job]
    triggers: {manual: true}
";

fn kept(active: &[&str], targets: &[&str]) -> (Vec<String>, Vec<String>) {
	let mut file: ComposeFile = parse_str(FILE).unwrap();
	let active: Vec<String> = active.iter().map(|s| s.to_string()).collect();
	let targets: Vec<String> = targets.iter().map(|s| s.to_string()).collect();
	retain_active_profiles_with_targets(&mut file, &active, &targets);
	(
		file.services.keys().cloned().collect(),
		file.jobs.keys().cloned().collect(),
	)
}

#[test]
fn a_profiled_job_is_ignored_without_its_profile() {
	let (services, jobs) = kept(&["x"], &[]);
	assert_eq!(jobs, ["plain"]);
	assert_eq!(services, ["web"]);
}

#[test]
fn an_active_profile_keeps_the_job_but_never_activates_its_service() {
	// ops-job needs db, which sits behind `data`. Only `ops` is active, so db
	// stays out and the job, which could not be satisfied, is ignored with it.
	let (services, jobs) = kept(&["ops"], &[]);
	assert_eq!(jobs, ["plain"]);
	assert_eq!(
		services,
		["web"],
		"a job must not pull db into config or audit"
	);
}

#[test]
fn a_job_is_kept_when_the_profile_of_its_service_is_active_too() {
	let (services, jobs) = kept(&["ops", "data"], &[]);
	assert_eq!(jobs, ["plain", "ops-job"]);
	assert!(services.contains(&"db".to_string()));
}

#[test]
fn a_job_ignored_for_its_dependency_takes_the_jobs_that_need_it() {
	// chain needs ops-job, which was ignored for needing db.
	let (_, jobs) = kept(&["ops", "chain"], &[]);
	assert_eq!(jobs, ["plain"]);
	let (_, jobs) = kept(&["ops", "chain", "data"], &[]);
	assert_eq!(jobs, ["plain", "ops-job", "chained"]);
}

#[test]
fn the_wildcard_keeps_every_job() {
	let (services, jobs) = kept(&["*"], &[]);
	assert_eq!(jobs.len(), 3);
	assert!(services.contains(&"db".to_string()));
}

#[test]
fn naming_a_job_activates_its_profile() {
	let (_, jobs) = kept(&["x", "data"], &["ops-job"]);
	assert!(jobs.contains(&"ops-job".to_string()), "{jobs:?}");
}

#[test]
fn what_only_a_job_needs_is_not_started_by_up() {
	// `up` asks `enabled_profile_services`, which knows nothing of jobs: a
	// service behind an inactive profile that only a job depends on stays down.
	let file: ComposeFile = parse_str(FILE).unwrap();
	let enabled = enabled_profile_services(&file, &HashSet::from(["ops".to_string()]), &[]);
	assert!(!enabled.contains("db"), "{enabled:?}");
	assert!(enabled.contains("web"));
}

#[test]
fn a_file_without_jobs_is_filtered_as_before() {
	let mut file: ComposeFile =
		parse_str("services:\n  a:\n    image: x\n  b:\n    image: x\n    profiles: [p]\n")
			.unwrap();
	retain_active_profiles_with_targets(&mut file, &[], &[]);
	assert_eq!(file.services.len(), 1);
}
