use super::*;

fn parse(y: &str) -> Directives {
	collect(&serde_yaml::from_str::<Value>(y).unwrap())
}

#[test]
fn collects_override_and_reset_per_service() {
	let d = parse(
		"services:\n  web:\n    ports: !override [\"9090:80\"]\n    dns: !reset []\n  db:\n    image: x\n",
	);
	let web = d.get("web").expect("web has tags");
	assert_eq!(web.get("ports"), Some(&MergeTag::Override));
	assert_eq!(web.get("dns"), Some(&MergeTag::Reset));
	assert!(!d.contains_key("db"), "a service with no tags is absent");
}

#[test]
fn untagged_document_yields_nothing() {
	assert!(parse("services:\n  web:\n    ports: [\"80:80\"]\n").is_empty());
}

/// A tag this tool does not define is ignored, not rejected; the document
/// may be valid for something else.
#[test]
fn unknown_tag_is_ignored() {
	let d = parse("services:\n  web:\n    ports: !whatever [\"80:80\"]\n");
	assert!(d.is_empty(), "{d:?}");
}

#[test]
fn a_document_without_services_is_not_an_error() {
	assert!(parse("volumes:\n  data:\n").is_empty());
}

/// Stripping is what makes a tag mean the same thing on every key. Left in,
/// serde decides: a `Vec` field ignores it and an untagged enum refuses the
/// whole file.
#[test]
fn strip_removes_tags_at_any_depth() {
	let mut v: Value = serde_yaml::from_str(
		"services:\n  web:\n    ports: !override [\"9090:80\"]\n    dns: !reset []\n",
	)
	.unwrap();
	strip(&mut v);
	let out = serde_yaml::to_string(&v).unwrap();
	assert!(!out.contains("!override"), "{out}");
	assert!(!out.contains("!reset"), "{out}");
	// The wrapped value survives: stripping removes the tag, not the data.
	assert!(out.contains("9090:80"), "{out}");
}

/// The document is otherwise untouched, including key order, which the
/// `config` output depends on.
#[test]
fn strip_preserves_an_untagged_document_verbatim() {
	let text = "services:\n  web:\n    image: alpine\n    ports:\n    - 80:80\n";
	let mut v: Value = serde_yaml::from_str(text).unwrap();
	let before = serde_yaml::to_string(&v).unwrap();
	strip(&mut v);
	assert_eq!(serde_yaml::to_string(&v).unwrap(), before);
}

#[test]
fn entry_resets_list_every_section() {
	let raw: Value = serde_yaml::from_str(
		"services:\n  t: !reset null\n  keep:\n    image: x\nnetworks:\n  n1: !reset null\nvolumes:\n  v1: !reset null\nsecrets:\n  s1: !reset null\n",
	)
	.unwrap();
	let mut found = entry_resets(&raw);
	found.sort();
	assert_eq!(
		found,
		vec![
			("networks", "n1".to_string()),
			("secrets", "s1".to_string()),
			("services", "t".to_string()),
			("volumes", "v1".to_string()),
		]
	);
}

#[test]
fn a_tag_inside_an_entry_is_not_an_entry_reset() {
	let raw: Value = serde_yaml::from_str("services:\n  web:\n    ports: !reset []\n").unwrap();
	assert!(entry_resets(&raw).is_empty());
}

#[test]
fn drop_entry_resets_leaves_the_other_entries() {
	let mut raw: Value = serde_yaml::from_str(
		"networks:\n  n1: !reset null\n  n2: {}\nservices:\n  a: !reset null\n  b:\n    image: x\n",
	)
	.unwrap();
	drop_entry_resets(&mut raw);
	assert!(raw["networks"].get("n1").is_none());
	assert!(raw["networks"].get("n2").is_some());
	assert!(raw["services"].get("a").is_none());
	assert!(raw["services"].get("b").is_some());
}
