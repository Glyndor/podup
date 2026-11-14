//! `label_file:` in an exported unit: the files are read when the unit is
//! generated, and each label becomes a `Label=` line, as `up` sends it.

use crate::parse_str;
use crate::quadlet::generate_at;
use crate::quadlet::tests::unit_named;

/// Generate service `s` of project `p` from `dir` and return its unit and the
/// warnings.
fn render(dir: &std::path::Path, service_body: &str) -> (String, String) {
	let yaml = format!("services:\n  s:\n    image: app:1.0\n{service_body}");
	let file = parse_str(&yaml).unwrap();
	let out = generate_at(&file, "p", dir);
	let unit = unit_named(&out, "p-s.container").contents.clone();
	(unit, out.warnings.join("\n"))
}

#[test]
fn each_label_in_a_label_file_becomes_a_label_line() {
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(
		dir.path().join("app.labels"),
		"# owner\ntier=web\n\nteam=core\n",
	)
	.unwrap();
	let (unit, warnings) = render(dir.path(), "    label_file: ./app.labels\n");
	assert!(unit.lines().any(|l| l == "Label=tier=web"), "{unit}");
	assert!(unit.lines().any(|l| l == "Label=team=core"), "{unit}");
	assert!(!warnings.contains("label_file"), "{warnings}");
}

#[test]
fn a_label_in_labels_wins_over_the_same_key_in_a_label_file() {
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(dir.path().join("app.labels"), "tier=from-file\n").unwrap();
	let (unit, _) = render(
		dir.path(),
		"    label_file: app.labels\n    labels:\n      tier: from-labels\n",
	);
	let tiers: Vec<&str> = unit
		.lines()
		.filter(|l| l.starts_with("Label=tier="))
		.collect();
	assert_eq!(tiers, ["Label=tier=from-labels"], "{unit}");
}

#[test]
fn every_file_of_a_label_file_list_is_read() {
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(dir.path().join("a.labels"), "a=1\n").unwrap();
	std::fs::write(dir.path().join("b.labels"), "b=2\n").unwrap();
	let (unit, _) = render(dir.path(), "    label_file: [a.labels, b.labels]\n");
	assert!(unit.lines().any(|l| l == "Label=a=1"), "{unit}");
	assert!(unit.lines().any(|l| l == "Label=b=2"), "{unit}");
}

#[test]
fn a_label_file_that_cannot_be_read_still_yields_the_unit() {
	let dir = tempfile::tempdir().unwrap();
	let (unit, warnings) = render(dir.path(), "    label_file: missing.labels\n");
	assert!(unit.contains("Label=podup.service=s"), "{unit}");
	assert!(
		warnings.contains("label_file: cannot read") && warnings.contains("missing.labels"),
		"{warnings}"
	);
}

#[test]
fn a_label_the_engine_rejects_is_left_out_and_reported() {
	// `up` refuses the file; generation has no error channel, so the unit
	// drops that label, keeps the rest and says which line it skipped.
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(dir.path().join("app.labels"), "ok=1\nbad\u{1b}key=2\n").unwrap();
	let (unit, warnings) = render(dir.path(), "    label_file: app.labels\n");
	assert!(unit.lines().any(|l| l == "Label=ok=1"), "{unit}");
	assert!(!unit.contains("bad"), "{unit}");
	assert!(warnings.contains("line 2"), "{warnings}");
}
