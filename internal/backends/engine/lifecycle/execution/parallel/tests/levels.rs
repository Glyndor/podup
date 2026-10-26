use super::*;

#[test]
fn filter_levels_empty_targets_keeps_all() {
	let file = file_with(&["a", "b", "c"]);
	let lv = levels(&[&["a", "b"], &["c"]]);
	let out = filter_levels(&file, lv.clone(), &[]).unwrap();
	assert_eq!(out, lv);
}

#[test]
fn filter_levels_drops_empty_levels() {
	let file = file_with(&["a", "b", "c"]);
	let lv = levels(&[&["a", "b"], &["c"]]);
	let out = filter_levels(&file, lv, &["c".into()]).unwrap();
	assert_eq!(out, levels(&[&["c"]]));
}

#[test]
fn filter_levels_unknown_target_errors() {
	let file = file_with(&["a"]);
	let lv = levels(&[&["a"]]);
	let err = filter_levels(&file, lv, &["z".into()]).unwrap_err();
	assert!(matches!(err, ComposeError::ServiceNotFound(n) if n == "z"));
}

#[test]
fn retain_levels_filters_and_drops() {
	let lv = levels(&[&["a", "b"], &["c"]]);
	let keep: HashSet<&str> = ["a", "c"].into_iter().collect();
	let out = retain_levels(lv, |n| keep.contains(n));
	assert_eq!(out, levels(&[&["a"], &["c"]]));
}
