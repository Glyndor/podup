use super::*;

// --- build_label_file_labels ---

#[test]
fn label_file_parses_keys_skips_comments_and_blanks() {
	use crate::compose::types::primitives::StringOrList;
	let dir = tempfile::tempdir().unwrap();
	let path = dir.path().join("labels.env");
	std::fs::write(
		&path,
		"# a comment\n\ncom.example.team=blue\nbare-key\n  com.example.tier = gold \n",
	)
	.unwrap();

	let mut svc = default_service();
	svc.label_file = StringOrList::Single("labels.env".to_string());
	let labels = build_label_file_labels(&svc, dir.path()).unwrap();

	assert_eq!(
		labels.get("com.example.team").map(String::as_str),
		Some("blue")
	);
	// A bare key with no `=` keeps an empty value.
	assert_eq!(labels.get("bare-key").map(String::as_str), Some(""));
	// The whole line is trimmed first, then the key side is trimmed again; the
	// value keeps its leading space after `=` but loses the line's trailing space.
	assert_eq!(
		labels.get("com.example.tier").map(String::as_str),
		Some(" gold")
	);
	// Comment and blank lines contribute nothing.
	assert_eq!(labels.len(), 3);
}

#[test]
fn label_file_missing_file_is_skipped() {
	use crate::compose::types::primitives::StringOrList;
	let dir = tempfile::tempdir().unwrap();
	let mut svc = default_service();
	svc.label_file = StringOrList::Single("absent.env".to_string());
	// A missing label file warns and yields no labels rather than erroring.
	assert!(build_label_file_labels(&svc, dir.path())
		.unwrap()
		.is_empty());
}

// --- build_label_file_labels: integration ---

#[test]
fn label_file_rejects_control_char_in_value_at_parse() {
	use crate::compose::types::primitives::StringOrList;
	let dir = tempfile::tempdir().unwrap();
	let path = dir.path().join("labels.env");
	// The value carries an embedded NUL: meaningless as a label and would
	// corrupt any downstream consumer that re-parses the value.
	std::fs::write(&path, "com.example.team=bl\0ue\n").unwrap();

	let mut svc = default_service();
	svc.label_file = StringOrList::Single("labels.env".to_string());
	let err = build_label_file_labels(&svc, dir.path()).unwrap_err();
	// The error names the file and the line so the user can find the
	// offending entry without bisecting the file.
	let msg = err.to_string();
	assert!(
		msg.contains("labels.env") && msg.contains("line 1"),
		"error did not name the file and line: {msg}",
	);
}

#[test]
fn label_file_caps_at_max_entries() {
	use crate::compose::types::primitives::StringOrList;
	let dir = tempfile::tempdir().unwrap();
	let path = dir.path().join("labels.env");
	// A pathological file well past the entry cap. The 16 MiB read cap does
	// not constrain the resulting HashMap size; the per-file entry cap
	// does, and 1000 lines is past it.
	let mut content = String::new();
	for i in 0..1000 {
		content.push_str(&format!("com.example.k{i}=v\n"));
	}
	std::fs::write(&path, content).unwrap();

	let mut svc = default_service();
	svc.label_file = StringOrList::Single("labels.env".to_string());
	// The cap is reached before the 1000th line, so the call returns Err
	// rather than a silently-truncated map.
	let err = build_label_file_labels(&svc, dir.path()).unwrap_err();
	assert!(
		err.to_string().contains("TooManyEntries"),
		"expected TooManyEntries, got: {err}",
	);
}

#[test]
fn label_file_accepts_exactly_max_entries() {
	use crate::compose::types::primitives::StringOrList;
	let dir = tempfile::tempdir().unwrap();
	let path = dir.path().join("labels.env");
	// Exactly the cap is the inclusive boundary; every entry fits.
	let mut content = String::new();
	for i in 0..MAX_LABEL_FILE_ENTRIES {
		content.push_str(&format!("com.example.k{i}=v\n"));
	}
	std::fs::write(&path, content).unwrap();

	let mut svc = default_service();
	svc.label_file = StringOrList::Single("labels.env".to_string());
	let labels = build_label_file_labels(&svc, dir.path()).unwrap();
	assert_eq!(labels.len(), MAX_LABEL_FILE_ENTRIES);
}

#[test]
fn label_file_overwrite_does_not_count_against_cap() {
	use crate::compose::types::primitives::StringOrList;
	let dir = tempfile::tempdir().unwrap();
	let path = dir.path().join("labels.env");
	// First `MAX_LABEL_FILE_ENTRIES` distinct keys, then repeated overwrites
	// of the first key; the overwrites are allowed at the cap.
	let mut content = String::new();
	for i in 0..MAX_LABEL_FILE_ENTRIES {
		content.push_str(&format!("com.example.k{i}=v{i}\n"));
	}
	content.push_str("com.example.k0=overwrite\n");
	std::fs::write(&path, content).unwrap();

	let mut svc = default_service();
	svc.label_file = StringOrList::Single("labels.env".to_string());
	let labels = build_label_file_labels(&svc, dir.path()).unwrap();
	assert_eq!(labels.len(), MAX_LABEL_FILE_ENTRIES);
	assert_eq!(
		labels.get("com.example.k0").map(String::as_str),
		Some("overwrite")
	);
}
