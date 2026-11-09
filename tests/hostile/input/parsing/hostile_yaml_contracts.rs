//! CLI YAML policy, checked against Docker Compose v5.7.0 `config` locally.
//! Docker refuses duplicates and invalid merge sources, and strips foreign
//! tags without executing them. Podup currently refuses multiple documents;
//! Docker v5.7.0 combines them. These frozen expectations need no Docker at run
//! time. Parser details are sanitized by the CLI, so errors below pin the
//! public diagnostic, not the dependency's potentially secret-bearing text.

use std::fs;

use serde_yaml::Value;
use tempfile::TempDir;

#[path = "../../support/file_process.rs"]
mod file_process;
use file_process::{command, run, Outcome};

struct Case {
	dir: TempDir,
}

impl Case {
	fn new(yaml: &str) -> Self {
		let dir = tempfile::tempdir().unwrap();
		fs::write(dir.path().join("compose.yaml"), yaml).unwrap();
		fs::write(dir.path().join("sentinel"), "unchanged").unwrap();
		Self { dir }
	}

	fn config(&self, overlay: Option<&str>) -> Outcome {
		let original = fs::read(self.dir.path().join("compose.yaml")).unwrap();
		let mut cmd = command(self.dir.path(), &[]);
		// Do not let the caller's compose settings or interpolation variables
		// change a fixture. The binary path is absolute; config needs no PATH.
		cmd.env_clear()
			.env("HOME", self.dir.path())
			.env("XDG_CONFIG_HOME", self.dir.path().join("config"))
			.env("XDG_DATA_HOME", self.dir.path().join("data"))
			.env("XDG_RUNTIME_DIR", self.dir.path())
			.env("NO_COLOR", "1");
		if let Some(yaml) = overlay {
			fs::write(self.dir.path().join("overlay.yaml"), yaml).unwrap();
			cmd.args(["-f", "overlay.yaml"]);
		}
		let outcome = run({
			cmd.arg("config");
			cmd
		});
		assert!(!outcome.timed_out, "config exceeded its deadline");
		assert!(outcome.code.is_some(), "config was killed");
		assert!(outcome.rss_kib < 256 * 1024, "resident memory ceiling");
		assert!(outcome.text.len() < 16 * 1024, "output ceiling");
		assert!(!outcome.text.contains("panicked at"), "{}", outcome.text);
		assert_eq!(
			fs::read(self.dir.path().join("compose.yaml")).unwrap(),
			original
		);
		assert_eq!(
			fs::read_to_string(self.dir.path().join("sentinel")).unwrap(),
			"unchanged"
		);
		assert!(!self.dir.path().join("side-effect").exists());
		let mut names: Vec<_> = fs::read_dir(self.dir.path())
			.unwrap()
			.map(|entry| entry.unwrap().file_name())
			// An instrumented run (cargo llvm-cov) drops a `.profraw` file in the
			// working directory of every child it spawns; it is not a write by
			// podup.
			.filter(|name| !name.to_string_lossy().ends_with(".profraw"))
			.collect();
		names.sort();
		let mut expected = vec!["compose.yaml", "sentinel"];
		if overlay.is_some() {
			expected.push("overlay.yaml");
		}
		expected.sort();
		assert_eq!(names, expected, "config wrote unexpected files");
		outcome
	}

	fn refused(&self, diagnostic: &str) {
		let outcome = self.config(None);
		assert_eq!(outcome.code, Some(1), "{}", outcome.text);
		// The message names the file that failed to parse, right after
		// "compose file"; the rest is the fixed diagnostic.
		let named = diagnostic.replacen(
			"compose file",
			&format!(
				"compose file '{}'",
				self.dir.path().join("compose.yaml").display()
			),
			1,
		);
		assert_eq!(outcome.text, format!("podup: error: {named}\n"));
	}

	fn accepted(&self, overlay: Option<&str>) -> Value {
		let outcome = self.config(overlay);
		assert_eq!(outcome.code, Some(0), "{}", outcome.text);
		assert!(
			!outcome.text.contains('!'),
			"tags survived: {}",
			outcome.text
		);
		serde_yaml::from_str(&outcome.text).expect("config must render one YAML document")
	}
}

#[test]
fn duplicate_keys_are_refused() {
	Case::new("services:\n  web:\n    image: alpine\n    image: busybox\n")
		.refused("failed to parse compose file at line 3, column 5");
}

#[test]
fn multiple_documents_are_refused() {
	Case::new("services:\n  web: {image: alpine}\n---\nservices:\n  other: {image: busybox}\n")
		.refused("failed to parse compose file");
}

#[test]
fn foreign_python_tag_is_inert_and_stripped() {
	// An unsafe Python YAML constructor would execute this during the load.
	// The relative marker stays inside this test's private working directory.
	let case = Case::new("services:\n  web:\n    image: alpine\n    command: !!python/object/apply:os.system ['echo executed > side-effect']\n");
	let value = case.accepted(None);
	assert_eq!(value["services"]["web"]["image"], "alpine");
	assert_eq!(
		value["services"]["web"]["command"],
		serde_yaml::to_value(["echo executed > side-effect"]).unwrap()
	);
}

fn merge_case(source: &str) -> Case {
	Case::new(&format!(
		"services:\n  web:\n    image: alpine\n    <<: {source}\n"
	))
}

#[test]
fn scalar_merge_source_is_refused() {
	merge_case("scalar").refused("failed to parse compose file");
}

#[test]
fn null_merge_source_is_refused() {
	merge_case("null").refused("failed to parse compose file");
}

#[test]
fn non_mapping_merge_list_is_refused() {
	merge_case("[one, two]").refused("failed to parse compose file");
}

#[test]
fn mapping_then_scalar_merge_list_is_refused() {
	merge_case("[{environment: {KEPT: inherited}}, 5]").refused("failed to parse compose file");
}

#[test]
fn scalar_then_mapping_merge_list_is_refused() {
	merge_case("[5, {environment: {KEPT: inherited}}]").refused("failed to parse compose file");
}

#[test]
fn ordinary_anchors_preserve_inheritance_and_explicit_values() {
	let case = Case::new("x-base: &base {image: alpine, environment: {KEPT: inherited}, command: [base]}\nservices:\n  web:\n    <<: *base\n    image: busybox\n");
	let value = case.accepted(None);
	let web = &value["services"]["web"];
	assert_eq!(web["image"], "busybox");
	assert_eq!(web["environment"]["KEPT"], "inherited");
	assert_eq!(web["command"], serde_yaml::to_value(["base"]).unwrap());
	assert!(web.get("<<").is_none());
}

#[test]
fn mapping_merge_sequences_preserve_first_source_precedence() {
	let value = merge_case("[{image: busybox, environment: {KEPT: first}}, {environment: {KEPT: second}, command: [inherited]}]").accepted(None);
	let web = &value["services"]["web"];
	assert_eq!(web["image"], "alpine");
	assert_eq!(web["environment"]["KEPT"], "first");
	assert_eq!(web["command"], serde_yaml::to_value(["inherited"]).unwrap());
	assert!(web.get("<<").is_none());
}

#[test]
fn reset_clears_an_inherited_environment() {
	let case = Case::new(
		"services:\n  web: {image: alpine, environment: {REMOVED: base}, command: [kept]}\n",
	);
	let value = case.accepted(Some("services:\n  web:\n    environment: !reset {}\n"));
	let web = &value["services"]["web"];
	assert_eq!(web["image"], "alpine");
	assert!(web.get("environment").is_none());
	assert_eq!(web["command"], serde_yaml::to_value(["kept"]).unwrap());
}

#[test]
fn override_replaces_an_inherited_environment() {
	let case = Case::new(
		"services:\n  web: {image: alpine, environment: {REMOVED: base}, command: [kept]}\n",
	);
	let value = case.accepted(Some(
		"services:\n  web:\n    environment: !override {KEPT: replacement}\n",
	));
	let web = &value["services"]["web"];
	assert_eq!(web["image"], "alpine");
	assert_eq!(
		web["environment"],
		serde_yaml::from_str::<Value>("{KEPT: replacement}").unwrap()
	);
	assert_eq!(web["command"], serde_yaml::to_value(["kept"]).unwrap());
}
