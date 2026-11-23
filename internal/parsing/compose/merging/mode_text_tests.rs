use std::path::PathBuf;

use crate::compose::types::inputs::mode::take_notices;
use crate::compose::types::{ServiceConfigRef, ServiceSecretRef, VolumeMount};
use crate::compose::{parse_files_with_env_files, parse_str, ComposeFile};

use std::cell::Cell;

use super::{FORCE_FAILURE, LINKS, PATH_NOTED};
use crate::compose::types::inputs::mode::FINAL_NUMBERS;

fn secret_yaml(mode: &str) -> String {
	format!(
		"services:\n  web:\n    image: img\n    secrets:\n      - source: s\n        mode: {mode}\nsecrets:\n  s:\n    content: x\n"
	)
}

fn config_yaml(mode: &str) -> String {
	format!(
		"services:\n  web:\n    image: img\n    configs:\n      - source: c\n        mode: {mode}\nconfigs:\n  c:\n    content: x\n"
	)
}

fn tmpfs_yaml(mode: &str) -> String {
	format!(
		"services:\n  web:\n    image: img\n    volumes:\n      - type: tmpfs\n        target: /run/x\n        tmpfs:\n          mode: {mode}\n"
	)
}

fn secret_of(file: &ComposeFile) -> Option<u32> {
	match &file.services["web"].secrets[0] {
		ServiceSecretRef::Long { mode, .. } => *mode,
		other => panic!("{other:?}"),
	}
}

fn secret_mode(yaml: &str) -> Result<Option<u32>, String> {
	parse_str(yaml)
		.map(|f| secret_of(&f))
		.map_err(|e| e.to_string())
}

fn config_mode(yaml: &str) -> Result<Option<u32>, String> {
	let file = parse_str(yaml).map_err(|e| e.to_string())?;
	match &file.services["web"].configs[0] {
		ServiceConfigRef::Long { mode, .. } => Ok(*mode),
		other => panic!("{other:?}"),
	}
}

fn tmpfs_mode(yaml: &str) -> Result<Option<u32>, String> {
	let file = parse_str(yaml).map_err(|e| e.to_string())?;
	match &file.services["web"].volumes[0] {
		VolumeMount::Long { tmpfs, .. } => Ok(tmpfs.as_ref().and_then(|t| t.mode)),
		other => panic!("{other:?}"),
	}
}

/// Every form, for a secret, a config and a tmpfs: each is the octal it spells.
#[test]
fn every_form_is_read_as_the_octal_it_is() {
	for (form, want) in [
		("0o644", 0o644),
		("0o404", 0o404),
		("0o400", 0o400),
		("0o440", 0o440),
		("0o600", 0o600),
		("0o666", 0o666),
		("0o640", 0o640),
		("0O644", 0o644),
		("0644", 0o644),
		("0400", 0o400),
		("\"0644\"", 0o644),
		("\"0o644\"", 0o644),
		("0x100", 0o400),
		("0x1a4", 0o644),
		("644", 0o644),
		("400", 0o400),
		("420", 0o420),
		("256", 0o256),
		("384", 384),
		("\"644\"", 0o644),
	] {
		assert_eq!(
			secret_mode(&secret_yaml(form)),
			Ok(Some(want)),
			"secret {form}"
		);
		assert_eq!(
			config_mode(&config_yaml(form)),
			Ok(Some(want)),
			"config {form}"
		);
		assert_eq!(
			tmpfs_mode(&tmpfs_yaml(form)),
			Ok(Some(want)),
			"tmpfs {form}"
		);
	}
}

#[test]
fn what_is_not_a_mode_is_refused_naming_the_service_the_secret_and_the_value() {
	for form in [
		"-1",
		"1.5",
		"0o8",
		"0o789",
		"0888",
		"0o17777",
		"0x1000",
		"\"abc\"",
		"\"0644x\"",
		"0b1",
	] {
		let error = secret_mode(&secret_yaml(form)).unwrap_err();
		assert!(
			error.contains("service 'web'")
				&& error.contains("secrets 's'")
				&& error.contains("invalid mode"),
			"{form}: {error}"
		);
		assert!(config_mode(&config_yaml(form)).is_err(), "config {form}");
	}
}

fn run_ok(files: &[(&str, String)]) -> (Option<u32>, (usize, usize)) {
	run(files).unwrap()
}

fn write(dir: &std::path::Path, name: &str, text: &str) -> PathBuf {
	let path = dir.join(name);
	std::fs::write(&path, text).unwrap();
	path
}

/// Parse `files` as `-f a -f b …` and return the secret mode, with the notices
/// given while doing it. The count of numbers that stood as a mode in the final
/// document is checked here for every row: a number is always noticed.
fn run(files: &[(&str, String)]) -> Result<(Option<u32>, (usize, usize)), String> {
	let dir = tempfile::tempdir().unwrap();
	let paths: Vec<PathBuf> = files
		.iter()
		.map(|(name, text)| write(dir.path(), name, text))
		.collect();
	take_notices();
	FINAL_NUMBERS.with(|c| c.set(0));
	let file = parse_files_with_env_files(&paths, &[]).map_err(|e| e.to_string())?;
	let notices = take_notices();
	let numbers = FINAL_NUMBERS.with(std::cell::Cell::get);
	assert_eq!(
		numbers, notices.0,
		"{numbers} numbers stood as a mode and {} were noticed",
		notices.0
	);
	Ok((secret_of(&file), notices))
}

fn secret_file_with(mode: &str) -> String {
	secret_yaml(mode)
}

const BASE: &str = "services:\n  web:\n    image: img\n    secrets:\n      - source: s\n        mode: 0o644\nsecrets:\n  s:\n    content: x\n";

/// What each way of building the document does with the spelling: the text is
/// recovered and read by the rules (no notice), or the number is read as an
/// integer with no leading zero and noticed.
#[test]
fn the_ways_a_document_is_built() {
	// A mode in the file itself: recovered.
	assert_eq!(
		run_ok(&[("a.yml", BASE.to_string())]),
		(Some(0o644), (0, 0))
	);

	// An anchor and its alias: recovered at the place the alias is used.
	let anchored = "x-ref: &ref\n  source: s\n  mode: 0o644\nservices:\n  web:\n    image: img\n    secrets:\n      - *ref\nsecrets:\n  s:\n    content: x\n";
	assert_eq!(
		run_ok(&[("a.yml", anchored.to_string())]),
		(Some(0o644), (0, 0))
	);

	// A merge key: the list sits under `<<`, and is recovered there.
	let merged = "x-base: &base\n  image: img\n  secrets:\n    - source: s\n      mode: 0o644\nservices:\n  web:\n    <<: *base\nsecrets:\n  s:\n    content: x\n";
	assert_eq!(
		run_ok(&[("a.yml", merged.to_string())]),
		(Some(0o644), (0, 0))
	);

	// `!override`: recovered through the tag.
	let overridden = "services:\n  web:\n    image: img\n    secrets: !override\n      - source: s\n        mode: 0o644\nsecrets:\n  s:\n    content: x\n";
	assert_eq!(
		run_ok(&[("a.yml", overridden.to_string())]),
		(Some(0o644), (0, 0))
	);

	// A second `-f` that replaces the list with `!override`: its own text.
	let second =
		"services:\n  web:\n    secrets: !override\n      - source: s\n        mode: 0o404\n";
	assert_eq!(
		run_ok(&[("a.yml", BASE.to_string()), ("b.yml", second.to_string())]),
		(Some(0o404), (0, 0))
	);

	// A second `-f` that gives another mode for the same secret: the later file
	// wins, as in `develop` and in `docker compose config`, with the text of its
	// own node (measured on 2026-11-23, in this order of rows: a, b -> result).
	let file_a = |mode: &str| secret_file_with(mode);
	for (a, b, want, notices) in [
		("0o600", "\"0644\"", 0o644, (0, 0)),
		("0o600", "0o644", 0o644, (0, 0)),
		("\"0644\"", "0o600", 0o600, (0, 0)),
		("0644", "0o404", 0o404, (0, 0)),
		// b writes 420: read as the digits 0420 and noticed, wherever a is from.
		("0644", "420", 0o420, (0, 1)),
		("0o644", "420", 0o420, (0, 1)),
	] {
		let second =
			format!("services:\n  web:\n    secrets:\n      - source: s\n        mode: {b}\n");
		assert_eq!(
			run_ok(&[("a.yml", file_a(a)), ("b.yml", second)]),
			(Some(want), notices),
			"a={a} b={b}"
		);
	}

	// `extends`: the child redefines the secret the base also has; the child wins,
	// with the text of the child's node.
	let base = "services:\n  base:\n    image: img\n    secrets:\n      - source: s\n        mode: 0o600\nsecrets:\n  s:\n    content: x\n";
	for (child_mode, want) in [("\"0644\"", 0o644), ("0o644", 0o644), ("0o404", 0o404)] {
		let child = format!(
			"services:\n  web:\n    extends:\n      file: base.yml\n      service: base\n    secrets:\n      - source: s\n        mode: {child_mode}\n"
		);
		assert_eq!(
			run_ok(&[("a.yml", child), ("base.yml", base.to_string())]),
			(Some(want), (0, 0)),
			"child {child_mode}"
		);
	}

	// An `include`: the included file is read like any other.
	let including = "include:\n  - path: inc.yml\n";
	assert_eq!(
		run_ok(&[
			("a.yml", including.to_string()),
			("inc.yml", BASE.to_string())
		]),
		(Some(0o644), (0, 0))
	);

	// `extends` across files: the service is read from its own file.
	let base = "services:\n  base:\n    image: img\n    secrets:\n      - source: s\n        mode: 0o644\nsecrets:\n  s:\n    content: x\n";
	let child = "services:\n  web:\n    extends:\n      file: base.yml\n      service: base\n";
	assert_eq!(
		run_ok(&[("a.yml", child.to_string()), ("base.yml", base.to_string())]),
		(Some(0o644), (0, 0))
	);
}

/// A mode that comes out of an interpolation keeps its text, whatever it is: the
/// text is read by the rules, and a bare integer is noticed.
#[test]
fn an_interpolated_mode_keeps_its_text() {
	for (value, want, notices) in [
		("0o644", 0o644, (0, 0)),
		("0644", 0o644, (0, 0)),
		("0x1a4", 0o644, (0, 0)),
		// `420` is text with no leading zero: read as today (0420) and noticed.
		("420", 0o420, (0, 1)),
		("644", 0o644, (0, 1)),
	] {
		temp_env::with_var("PODUP_MODE_TEST", Some(value), || {
			let got = run_ok(&[("a.yml", secret_yaml("${PODUP_MODE_TEST}"))]);
			assert_eq!(got, (Some(want), notices), "M={value}");
		});
	}
}

/// A mode written without a leading zero is read as it has always been, and said.
#[test]
fn an_integer_with_no_leading_zero_is_noticed_with_what_was_applied() {
	assert_eq!(
		run_ok(&[("a.yml", secret_yaml("644"))]),
		(Some(0o644), (0, 1))
	);
	assert_eq!(
		run_ok(&[("a.yml", secret_yaml("\"644\""))]),
		(Some(0o644), (0, 1))
	);
	assert_eq!(
		run_ok(&[("a.yml", secret_yaml("0o644"))]),
		(Some(0o644), (0, 0))
	);
}

/// When the reading cannot give the text, the number that stands is read only when
/// it is not a guess, and refused otherwise: it is never applied without being
/// known.
#[test]
fn a_number_whose_text_was_not_recovered_is_refused_unless_it_is_not_a_guess() {
	FORCE_FAILURE.with(|c| c.set(true));
	// 0o644 decoded to 420: digits as octal 0420, the number 0644. Refused.
	let refused = run(&[("a.yml", BASE.to_string())]).unwrap_err();
	assert!(
		refused.contains("service 'web'")
			&& refused.contains("secrets 's'")
			&& refused.contains("mode 420")
			&& refused.contains("quoted octal string")
			&& refused.contains("0o644"),
		"{refused}"
	);
	// 0o600 decoded to 384: its digits are not octal, so the number is the only reading.
	let agreed = run(&[("a.yml", secret_yaml("0o600"))]);
	FORCE_FAILURE.with(|c| c.set(false));
	assert_eq!(agreed, Ok((Some(0o600), (1, 0))));
}

/// A `mode:` somewhere else is not touched: a label, an extension, a port.
#[test]
fn a_mode_elsewhere_is_left_as_it_was() {
	let yaml = "x-thing:\n  mode: 0o644\nservices:\n  web:\n    image: img\n    labels:\n      mode: 0o644\n    ports:\n      - target: 80\n        published: 8080\n        mode: host\n";
	assert!(parse_str(yaml).is_ok());
}

/// A `mode:` that reaches an entry through a merge key, from an anchor defined
/// anywhere, as one mapping or as a list of them, or that carries a tag.
#[test]
fn a_mode_that_comes_through_a_merge_key_or_a_tag_keeps_its_text() {
	let through_entry = "x-mode: &m\n  mode: 0o644\nservices:\n  web:\n    image: img\n    secrets:\n      - source: s\n        <<: *m\nsecrets:\n  s:\n    content: x\n";
	assert_eq!(
		run_ok(&[("a.yml", through_entry.to_string())]),
		(Some(0o644), (0, 0))
	);

	let list_of_anchors = "x-a: &a\n  mode: 0o404\nx-b: &b\n  source: s\nservices:\n  web:\n    image: img\n    secrets:\n      - <<: [*b, *a]\nsecrets:\n  s:\n    content: x\n";
	assert_eq!(
		run_ok(&[("a.yml", list_of_anchors.to_string())]),
		(Some(0o404), (0, 0))
	);

	let on_the_service = "x-svc: &svc\n  image: img\n  secrets:\n    - source: s\n      mode: 0o404\nservices:\n  web:\n    <<: [*svc]\nsecrets:\n  s:\n    content: x\n";
	assert_eq!(
		run_ok(&[("a.yml", on_the_service.to_string())]),
		(Some(0o404), (0, 0))
	);

	let tagged = "services:\n  web:\n    image: img\n    secrets:\n      - source: s\n        mode: !override 0o644\nsecrets:\n  s:\n    content: x\n";
	assert_eq!(
		run_ok(&[("a.yml", tagged.to_string())]),
		(Some(0o644), (0, 0))
	);

	let tmpfs = "x-t: &t\n  mode: 0o1777\nservices:\n  web:\n    image: img\n    volumes:\n      - type: tmpfs\n        target: /run/x\n        tmpfs:\n          <<: *t\n";
	assert_eq!(tmpfs_mode(tmpfs), Ok(Some(0o1777)));
}

/// The cost of the walk. 20 000 numeric modes under a path 30 levels deep (merge
/// keys, which the walk follows) take a pointer each plus the steps that are theirs,
/// not a copy of the whole way; the time is the fastest of five runs, not one.
#[test]
fn twenty_thousand_modes_under_a_deep_path_cost_little_time_and_memory() {
	const MODES: usize = 20_000;
	const DEPTH: usize = 30;
	let mut doc = String::from("services:\n  web:\n    image: img\n");
	for level in 0..DEPTH {
		doc.push_str(&format!("{}<<:\n", "  ".repeat(level + 2)));
	}
	doc.push_str(&format!("{}secrets:\n", "  ".repeat(DEPTH + 2)));
	for i in 0..MODES {
		let pad = "  ".repeat(DEPTH + 3);
		doc.push_str(&format!("{pad}- source: s{i}\n{pad}  mode: 0o644\n"));
	}
	let value: serde_yaml::Value = serde_yaml::from_str(&doc).unwrap();
	let mut best = std::time::Duration::MAX;
	for _ in 0..5 {
		let mut value = value.clone();
		LINKS.with(|c| c.set(0));
		PATH_NOTED.with(|c| c.set(0));
		let started = std::time::Instant::now();
		assert_eq!(super::restore(&mut value, &doc).unwrap(), MODES);
		best = best.min(started.elapsed());
		assert_eq!(PATH_NOTED.with(Cell::get), MODES);
		// The way down is made once per reading, and each mode adds its own list
		// index and key; a copy of the whole path per mode would be 33 steps each.
		let links = LINKS.with(Cell::get);
		assert!(
			links <= 2 * (DEPTH + 10 + 2 * MODES),
			"{links} steps made for {MODES} modes"
		);
	}
	assert!(
		best < std::time::Duration::from_secs(2),
		"fastest of five: {best:?}"
	);
}

/// Numbers and texts are paired by their ordinal, and a mode that is not a number
/// (a text, an empty value, a list) takes an ordinal and no text.
#[test]
fn the_texts_go_to_the_numbers_they_belong_to_with_gaps_between() {
	let doc = "services:\n  web:\n    image: img\n    secrets:\n      - {source: a, mode: '0640'}\n      - {source: b, mode: 0o644}\n      - {source: c, mode: ~}\n      - {source: d, mode: 0o404}\n      - {source: e, mode: [1]}\n      - {source: f, mode: 0x1ff}\n      - {source: g, mode: 0o400}\n    configs:\n      - {source: h, mode: \"0o600\"}\n      - {source: i, mode: 0o1777}\n";
	let mut value: serde_yaml::Value = serde_yaml::from_str(doc).unwrap();
	assert_eq!(super::restore(&mut value, doc).unwrap(), 5);
	let modes: Vec<String> = ["secrets", "configs"]
		.iter()
		.flat_map(|k| value["services"]["web"][k].as_sequence().unwrap().clone())
		.map(|e| format!("{:?}", e["mode"]))
		.collect();
	let text = |t: &str| format!("String({t:?})");
	assert_eq!(modes[0], text("0640"));
	assert_eq!(modes[1], text("0o644"));
	assert_eq!(modes[3], text("0o404"));
	assert_eq!(modes[5], text("0x1ff"));
	assert_eq!(modes[6], text("0o400"));
	assert_eq!(modes[7], text("0o600"));
	assert_eq!(modes[8], text("0o1777"));
	assert!(modes[2].contains("Null") && modes[4].contains("Sequence"));
}

/// The pairing leaves out a number without a text and a text without a number, and
/// keeps each text with its own number.
#[test]
fn the_pairing_by_ordinal_skips_what_has_no_partner() {
	let numbers = vec![(1, 'a'), (3, 'b'), (4, 'c'), (7, 'd'), (9, 'e')];
	let texts = [0, 3, 5, 7, 8]
		.iter()
		.map(|o| (*o, format!("t{o}")))
		.collect();
	assert_eq!(
		super::pair(numbers, texts),
		vec![('b', "t3".to_string()), ('d', "t7".to_string())]
	);
	assert!(super::pair::<char>(vec![], vec![(0, "x".into())]).is_empty());
	assert!(super::pair(vec![(0, 'a')], vec![]).is_empty());
}

/// A document with more modes than the limit is refused with a message that says
/// so; at the limit it is read.
#[test]
fn a_document_with_too_many_modes_is_refused_with_a_clear_message() {
	let entries = |n: usize| {
		let mut doc = String::from("services:\n  web:\n    image: img\n    secrets:\n");
		for i in 0..n {
			doc.push_str(&format!("      - {{source: s{i}, mode: 0o644}}\n"));
		}
		doc
	};
	let (at, over) = (entries(4), entries(5));
	let mut value: serde_yaml::Value = serde_yaml::from_str(&at).unwrap();
	assert_eq!(super::restore_up_to(&mut value, &at, 4).unwrap(), 4);
	let mut value: serde_yaml::Value = serde_yaml::from_str(&over).unwrap();
	let message = super::restore_up_to(&mut value, &over, 4)
		.unwrap_err()
		.to_string();
	assert!(message.contains("more than 4 `mode:` values"), "{message}");

	// The real limit, through the whole parse.
	let many = entries(super::MAX_MODES + 1);
	let error = parse_str(&many).unwrap_err().to_string();
	assert!(error.contains("more than 100000 `mode:` values"), "{error}");
}

/// podup writes a service out and reads it back in two places while it puts a
/// project together: the merge of an `extends`, and the removal of a nested
/// reset. The modes it wrote are the bits they resolved to, and are read back as
/// those bits, not as a spelling that was lost.
#[test]
fn what_podup_writes_out_and_reads_back_keeps_its_modes() {
	// `extends` adds a second secret to the ones the base has: the base's modes go
	// through the merge as numbers and must come back as they were.
	let base = "services:\n  base:\n    image: img\n    secrets:\n      - source: s\n        mode: 0o644\n      - source: t\n        mode: 0o404\n    configs:\n      - source: c\n        mode: 0o600\n    volumes:\n      - type: tmpfs\n        target: /run/x\n        tmpfs:\n          mode: 0o1777\nsecrets:\n  s:\n    content: x\n  t:\n    content: x\n  u:\n    content: x\nconfigs:\n  c:\n    content: x\n";
	let child = "services:\n  web:\n    extends:\n      file: base.yml\n      service: base\n    secrets:\n      - source: u\n        mode: 0o440\n";
	let dir = tempfile::tempdir().unwrap();
	let a = write(dir.path(), "a.yml", child);
	write(dir.path(), "base.yml", base);
	take_notices();
	let file = parse_files_with_env_files(&[a], &[]).unwrap();
	let modes: Vec<Option<u32>> = file.services["web"]
		.secrets
		.iter()
		.map(|s| match s {
			ServiceSecretRef::Long { mode, .. } => *mode,
			other => panic!("{other:?}"),
		})
		.collect();
	assert!(
		modes.contains(&Some(0o644))
			&& modes.contains(&Some(0o404))
			&& modes.contains(&Some(0o440)),
		"{modes:?}"
	);
	assert!(matches!(
		&file.services["web"].configs[0],
		ServiceConfigRef::Long {
			mode: Some(0o600),
			..
		}
	));
	assert!(matches!(
		&file.services["web"].volumes[0],
		VolumeMount::Long { tmpfs: Some(t), .. } if t.mode == Some(0o1777)
	));
	assert_eq!(take_notices(), (0, 0), "nothing was noticed or guessed");

	// A nested reset in a later file sends the merged service through its serialised
	// form and back.
	let first = "services:\n  web:\n    image: img\n    labels:\n      a: '1'\n      b: '2'\n    secrets:\n      - source: s\n        mode: 0o644\nsecrets:\n  s:\n    content: x\n";
	let second = "services:\n  web:\n    labels:\n      a: !reset null\n";
	take_notices();
	let (mode, notices) = run_ok(&[("a.yml", first.to_string()), ("b.yml", second.to_string())]);
	assert_eq!((mode, notices), (Some(0o644), (0, 0)));
}
