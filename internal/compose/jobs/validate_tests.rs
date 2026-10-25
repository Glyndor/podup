//! The rules that tie jobs to services: one namespace, no service depending on
//! a job, `extends` inside its own kind, and cycles that pass through a job.

use crate::compose::types::ComposeFile;
use crate::compose::{parse_files_with_env_files, parse_str, validate_config};
use crate::error::ComposeError;

/// A manual job body with the given extra lines (each already indented four
/// spaces under the job name).
fn job(name: &str, extra: &str) -> String {
	format!("  {name}:\n    image: alpine\n    triggers: {{manual: true}}\n{extra}")
}

fn error_of(yaml: &str) -> String {
	parse_str(yaml)
		.err()
		.unwrap_or_else(|| panic!("this file must be refused:\n{yaml}"))
		.to_string()
}

#[test]
fn a_job_and_a_service_with_one_name_are_refused() {
	let yaml = format!("services:\n  db:\n    image: x\njobs:\n{}", job("db", ""));
	let message = error_of(&yaml);
	assert!(
		message.contains("the name 'db' is declared as a service and as a job"),
		"{message}"
	);
}

#[test]
fn the_name_clash_is_found_whichever_section_comes_first() {
	let yaml = format!("jobs:\n{}services:\n  db:\n    image: x\n", job("db", ""));
	assert!(error_of(&yaml).contains("the name 'db' is declared as a service and as a job"));
}

#[test]
fn a_clash_between_two_files_is_found_after_the_merge() {
	let dir = tempfile::tempdir().unwrap();
	let base = dir.path().join("base.yaml");
	let over = dir.path().join("over.yaml");
	std::fs::write(&base, "services:\n  db:\n    image: x\n").unwrap();
	std::fs::write(&over, format!("jobs:\n{}", job("db", ""))).unwrap();
	let err = parse_files_with_env_files(&[base, over], &[]).expect_err("clash across files");
	assert!(matches!(err, ComposeError::Job(_)), "{err:?}");
	assert!(err.to_string().contains("'db'"), "{err}");
}

#[test]
fn a_clash_with_a_control_character_name_is_escaped() {
	let yaml = "services:\n  \"a\\u001bb\":\n    image: x\njobs:\n  \"a\\u001bb\":\n    image: x\n    triggers: {manual: true}\n";
	let message = error_of(yaml);
	assert!(!message.chars().any(char::is_control), "{message:?}");
	assert!(message.contains("a\\u{1b}b"), "{message}");
}

#[test]
fn a_service_cannot_depend_on_a_job() {
	for depends in [
		"depends_on: [nightly]",
		"depends_on:\n      nightly:\n        condition: service_completed_successfully",
		"volumes_from: [nightly]",
		"links: [nightly]",
		"network_mode: service:nightly",
	] {
		let yaml = format!(
			"services:\n  web:\n    image: x\n    {depends}\njobs:\n{}",
			job("nightly", "")
		);
		let message = error_of(&yaml);
		assert!(
			message.contains("service 'web' depends on job 'nightly'"),
			"{depends}: {message}"
		);
	}
}

#[test]
fn a_job_may_depend_on_a_service_and_on_another_job() {
	let yaml = format!(
		"services:\n  db:\n    image: x\njobs:\n{}{}",
		job("migrate", "    depends_on: [db]\n"),
		job("report", "    depends_on: [migrate, db]\n")
	);
	let file = parse_str(&yaml).expect("jobs depend on services and jobs");
	validate_config(&file).expect("and the graph is sound");
}

#[test]
fn a_job_cannot_extend_a_service() {
	let yaml = format!(
		"services:\n  base:\n    image: x\njobs:\n{}",
		job("nightly", "    extends: base\n")
	);
	let message = error_of(&yaml);
	assert!(
		message.contains("job 'nightly' extends 'base', which is a service"),
		"{message}"
	);
}

#[test]
fn a_service_cannot_extend_a_job() {
	let yaml = format!(
		"services:\n  web:\n    image: x\n    extends: nightly\njobs:\n{}",
		job("nightly", "")
	);
	let message = error_of(&yaml);
	assert!(
		message.contains("service 'web' extends 'nightly', which is a job"),
		"{message}"
	);
}

#[test]
fn a_job_extends_another_job_and_keeps_its_own_triggers() {
	let yaml = "\
jobs:
  base:
    image: alpine:3
    environment: {A: '1'}
    triggers: {manual: true}
  child:
    extends: base
    environment: {B: '2'}
    triggers:
      manual: false
      schedule: ['0 3 * * *']
";
	let file = parse_str(yaml).unwrap();
	let child = &file.jobs["child"];
	assert_eq!(
		child.service.image.as_deref(),
		Some("alpine:3"),
		"image inherited"
	);
	assert!(child.service.extends.is_none(), "extends is consumed");
	let env = child.service.environment.to_map();
	assert!(env.contains_key("A") && env.contains_key("B"), "{env:?}");
	assert!(
		!child.triggers.allows_manual(),
		"triggers are the job's own"
	);
	assert_eq!(child.triggers.schedule.len(), 1);
}

#[test]
fn a_job_extends_a_job_declared_later() {
	let yaml = format!(
		"jobs:\n{}{}",
		job("child", "    extends: base\n").replace("    image: alpine\n", ""),
		job("base", "")
	);
	let file = parse_str(&yaml).unwrap();
	assert_eq!(file.jobs["child"].service.image.as_deref(), Some("alpine"));
}

#[test]
fn a_job_extends_cycle_is_refused() {
	let yaml = format!(
		"jobs:\n{}{}",
		job("a", "    extends: b\n"),
		job("b", "    extends: a\n")
	);
	let message = error_of(&yaml);
	assert!(message.contains("circular extends"), "{message}");
}

#[test]
fn a_job_cannot_extend_itself() {
	let message = error_of(&format!("jobs:\n{}", job("a", "    extends: a\n")));
	assert!(message.contains("job 'a' extends itself"), "{message}");
}

#[test]
fn a_job_extends_an_unknown_job_by_name() {
	let message = error_of(&format!("jobs:\n{}", job("a", "    extends: ghost\n")));
	assert!(
		message.contains("job 'a' extends unknown job 'ghost'"),
		"{message}"
	);
}

#[test]
fn a_job_extends_chain_has_a_depth_limit() {
	// Declared child first: each job is then resolved before its base, which
	// is the order that builds the deepest chain.
	let mut yaml = String::from("jobs:\n");
	for index in (1..=40).rev() {
		yaml.push_str(&job(
			&format!("j{index}"),
			&format!("    extends: j{}\n", index - 1),
		));
	}
	yaml.push_str(&job("j0", ""));
	let message = error_of(&yaml);
	assert!(message.contains("exceeds maximum depth"), "{message}");
	assert!(message.contains("job '"), "{message}");
}

#[test]
fn a_job_cycle_through_depends_on_is_refused_by_validation() {
	let yaml = format!(
		"jobs:\n{}{}",
		job("a", "    depends_on: [b]\n"),
		job("b", "    depends_on: [a]\n")
	);
	let file: ComposeFile = parse_str(&yaml).expect("the cycle is a graph property");
	let err = validate_config(&file).expect_err("a cycle between jobs has no run order");
	assert!(
		matches!(err, ComposeError::CircularDependency(_)),
		"{err:?}"
	);
	assert!(err.to_string().contains('a') && err.to_string().contains('b'));
}

#[test]
fn a_job_depending_on_something_undefined_is_refused_by_validation() {
	let file = parse_str(&format!("jobs:\n{}", job("a", "    depends_on: [ghost]\n"))).unwrap();
	let err = validate_config(&file).expect_err("ghost is not defined");
	assert!(matches!(err, ComposeError::ServiceNotFound(_)), "{err:?}");
}

#[test]
fn a_jobs_only_file_is_a_valid_project() {
	let file = parse_str(&format!("jobs:\n{}", job("a", ""))).unwrap();
	validate_config(&file).expect("a file with only jobs has something to run");
}

#[test]
fn a_file_with_neither_services_nor_jobs_is_still_refused() {
	let file = parse_str("jobs: {}\n").unwrap();
	assert!(validate_config(&file).is_err());
}

#[test]
fn a_job_without_image_or_build_is_refused_by_validation() {
	let file = parse_str("jobs:\n  a:\n    triggers: {manual: true}\n").unwrap();
	let err = validate_config(&file).expect_err("a job needs something to run");
	assert!(matches!(err, ComposeError::NoImageOrBuild(_)), "{err:?}");
}

#[test]
fn a_job_with_an_undefined_network_is_refused_by_validation() {
	let yaml = "jobs:\n  a:\n    image: x\n    networks: [ghost]\n    triggers: {manual: true}\n";
	let file = parse_str(yaml).unwrap();
	assert!(validate_config(&file).is_err());
}
