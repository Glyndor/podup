//! Switching between include and extends must not restart resource guards.

use super::file_process::{command, run, Outcome};
use super::{assert_refused, assert_survived, compose_dir, write};
use std::path::Path;

const MIB: usize = 1024 * 1024;

fn service(name: &str, word: &str, refs: usize) -> String {
	let aliases = vec!["*word"; refs].join(", ");
	format!(
		"x-word: &word '{word}'\nservices:\n  {name}:\n    image: alpine\n    command: [{aliases}]\n"
	)
}

/// root -> include child -> extends bridge -> include leaf. The root also
/// includes the leaf, exercising a shared target reached by different routes.
fn graph(dir: &Path, refs: usize, word: &str, cycle: bool) {
	write(dir, ".env", &format!("BIG={}\n", "x".repeat(MIB)));
	write(
		dir,
		"compose.yaml",
		&format!(
			"include: [child.yaml, leaf.yaml]\n{}",
			service("root", word, refs)
		),
	);
	let child = service("child", word, refs).replace(
		"    image: alpine\n",
		"    extends: {file: bridge.yaml, service: base}\n",
	);
	write(dir, "child.yaml", &child);
	write(
		dir,
		"bridge.yaml",
		&format!("include: [leaf.yaml]\n{}", service("base", word, refs)),
	);
	let back = if cycle {
		"include: [compose.yaml]\n"
	} else {
		""
	};
	write(
		dir,
		"leaf.yaml",
		&format!("{back}{}", service("leaf", word, refs)),
	);
}

fn load(dir: &Path) -> Outcome {
	let mut cmd = command(dir, &["config"]);
	cmd.env_remove("BIG");
	let outcome = run(cmd);
	assert_survived("mixed include/extends graph", &outcome);
	assert!(!outcome.timed_out, "mixed graph exceeded the deadline");
	outcome
}

#[test]
fn a_small_mixed_graph_loads() {
	for word in ["route-marker", "${BIG}"] {
		let dir = compose_dir();
		graph(dir.path(), 1, word, false);
		let outcome = load(dir.path());
		assert_eq!(outcome.code, Some(0), "{}", outcome.text);
		assert!(
			!outcome.text.contains("${BIG}"),
			"interpolation did not run"
		);
	}
}

#[test]
fn aliases_cannot_restart_interpolation_budget_at_a_route_boundary() {
	let dir = compose_dir();
	// Each document substitutes 6 MiB through aliases, under the 16 MiB cap.
	// The root, child and external base alone exceed it together.
	graph(dir.path(), 6, "${BIG}", false);
	let outcome = load(dir.path());
	assert_refused("mixed interpolation budget", &outcome);
	assert!(
		outcome.text.contains("bytes of substituted output"),
		"the graph failed for another reason: {}",
		outcome.text
	);
}

#[test]
fn a_flat_alias_attack_in_an_external_base_is_refused() {
	let dir = compose_dir();
	graph(dir.path(), 1, "small", false);
	// Flat aliases avoid relying on the YAML library's nested repetition cap.
	write(
		dir.path(),
		"bridge.yaml",
		&format!(
			"include: [leaf.yaml]\n{}",
			service("base", &"y".repeat(60_000), 90)
		),
	);
	let outcome = load(dir.path());
	assert_refused("mixed alias budget", &outcome);
	assert!(outcome.text.contains("alias"), "{}", outcome.text);
}

#[test]
fn a_cycle_crossing_include_and_extends_stays_bounded() {
	let dir = compose_dir();
	graph(dir.path(), 1, "small", true);
	// Make the back edge belong to the external base, not the root's direct
	// include path: root -> child -> bridge -> leaf -> child.
	write(dir.path(), "compose.yaml", "include: [child.yaml]\n");
	write(
		dir.path(),
		"leaf.yaml",
		"include: [child.yaml]\nservices:\n  leaf:\n    image: alpine\n",
	);
	let outcome = load(dir.path());
	// A loader may ignore include directives in an extends-only source. That
	// is bounded too; following them must either finish or diagnose the cycle.
	assert!(
		outcome.text.len() < 16 * 1024,
		"cycle output grew without bound"
	);
}
