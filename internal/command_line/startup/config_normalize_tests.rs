use super::*;

#[test]
fn yaml11_bool_detection_is_case_insensitive_and_scoped() {
	for tok in [
		"yes", "Yes", "YES", "no", "on", "OFF", "y", "N", "true", "False",
	] {
		assert!(looks_like_yaml11_bool(tok), "{tok} should match");
	}
	for tok in ["hello", "yess", "0", "onoff", "", "nullish"] {
		assert!(!looks_like_yaml11_bool(tok), "{tok} should not match");
	}
}

#[test]
fn quote_yaml11_booleans_quotes_only_plain_bool_scalars() {
	// `yes`/`off` are quoted; an already-quoted `'null'`, a normal string, a
	// nested key, and a non-bool value are all left exactly as serde_yaml wrote
	// them. A `: ` inside a quoted value must not trip the splitter.
	let input = "environment:\n  FROM_A: yes\n  FROM_B: off\n  FROM_C: 'null'\n  NORMAL: hello\n  COLON: 'a: b'\nports:\n- on\n- '8080:80'\n";
	let out = quote_yaml11_booleans(input);
	assert!(out.contains("FROM_A: 'yes'"), "got: {out}");
	assert!(out.contains("FROM_B: 'off'"), "got: {out}");
	assert!(out.contains("FROM_C: 'null'"), "double-quoting: {out}");
	assert!(out.contains("NORMAL: hello"), "got: {out}");
	assert!(out.contains("COLON: 'a: b'"), "got: {out}");
	assert!(out.contains("- 'on'"), "sequence bool item: {out}");
	assert!(out.contains("- '8080:80'"), "got: {out}");
	// The trailing newline is preserved.
	assert!(out.ends_with('\n'));
}

#[test]
fn quote_yaml11_booleans_leaves_block_scalar_content_alone() {
	// Lines inside `|`, `|-` and `>` blocks are string content. The entry
	// after each block, and the sibling key after a block that opens a
	// sequence item (`- note: |`), are entries again and still get quoted.
	let input = "\
labels:
  a: |
    enabled: yes

    mode: off
  b: yes
command:
- |-
  on
- off
volumes:
- note: >
    y: n
  flag: no
x:
  - - |
      - yes
  - true
";
	let expected = "\
labels:
  a: |
    enabled: yes

    mode: off
  b: 'yes'
command:
- |-
  on
- 'off'
volumes:
- note: >
    y: n
  flag: 'no'
x:
  - - |
      - yes
  - 'true'
";
	assert_eq!(quote_yaml11_booleans(input), expected);
}

#[test]
fn quote_yaml11_booleans_keeps_every_string_the_serializer_wrote() {
	// Real serializer output for blocks under an explicit key (`? note: |`),
	// a multiline key written as `? |`, and a tagged block (`!reset |`).
	// Reading the quoted text back must give the same value, and the
	// single-line `no` after each block must still be quoted.
	for doc in [
		"? {note: \"enabled: yes\\n\", flag: \"no\"}\n: \"off\"\n",
		"? [{note: \"enabled: yes\\n\", flag: \"no\"}]\n: \"off\"\n",
		"\"enabled: yes\\nmode: off\\n\": value\nflag: \"no\"\n",
		"note: !reset \"enabled: yes\\nmode: off\\n\"\nflag: \"no\"\n",
	] {
		let value: serde_yaml::Value = serde_yaml::from_str(doc).unwrap();
		let emitted = serde_yaml::to_string(&value).unwrap();
		let quoted = quote_yaml11_booleans(&emitted);
		let back: serde_yaml::Value = serde_yaml::from_str(&quoted).unwrap();
		assert_eq!(back, value, "emitted:\n{emitted}quoted:\n{quoted}");
		assert!(
			quoted.contains("flag: 'no'"),
			"emitted:\n{emitted}quoted:\n{quoted}"
		);
	}
}

#[test]
fn block_scalar_parent_reads_the_header_column() {
	assert_eq!(block_scalar_parent("  a: |"), Some(2));
	assert_eq!(block_scalar_parent("- note: >-"), Some(2));
	assert_eq!(block_scalar_parent("  - - key: |2+"), Some(6));
	assert_eq!(block_scalar_parent("- |"), Some(0));
	assert_eq!(block_scalar_parent("  - - |"), Some(4));
	assert_eq!(block_scalar_parent("? note: |"), Some(2));
	assert_eq!(block_scalar_parent("? - note: |"), Some(4));
	assert_eq!(block_scalar_parent("? |"), Some(0));
	assert_eq!(block_scalar_parent("note: !reset |"), Some(0));
	assert_eq!(block_scalar_parent("- !reset |-"), Some(0));
	// Not a header: a plain value ending in `|`, an indicator followed by
	// something else, and a bare `|` with nothing before it.
	assert_eq!(block_scalar_parent("a: b |"), None);
	assert_eq!(block_scalar_parent("a: b- |"), None);
	assert_eq!(block_scalar_parent("a: |x"), None);
	assert_eq!(block_scalar_parent("   |"), None);
}

#[cfg(unix)]
#[test]
fn short_bind_source_resolves_relative_only() {
	let base = Path::new("/home/user/proj");
	// A relative `.`-prefixed source is resolved against the project dir, keeping
	// the target and options intact.
	assert_eq!(
		rewrite_short_bind("./data:/data:ro", base).as_deref(),
		Some("/home/user/proj/data:/data:ro")
	);
	// `..` collapses lexically.
	assert_eq!(
		rewrite_short_bind("../shared:/s", base).as_deref(),
		Some("/home/user/shared:/s")
	);
	// Absolute sources, named volumes, and `~` are left untouched.
	assert!(rewrite_short_bind("/abs:/data", base).is_none());
	assert!(rewrite_short_bind("named:/data", base).is_none());
	assert!(rewrite_short_bind("~/x:/data", base).is_none());
	// A colon-less spec (anonymous volume target) is not a bind.
	assert!(rewrite_short_bind("/data", base).is_none());
}

#[cfg(unix)]
#[test]
fn resolve_bind_sources_rewrites_short_and_long_binds() {
	let mut file = podup::parse_str(
		"services:\n  web:\n    image: nginx\n    volumes:\n      - ./data:/data:ro\n      - type: bind\n        source: ./logs\n        target: /logs\n      - named:/cache\n",
	)
	.unwrap();
	resolve_bind_sources(&mut file, Path::new("/srv/app"));
	let mounts = &file.services["web"].volumes;
	match &mounts[0] {
		VolumeMount::Short(s) => assert_eq!(s, "/srv/app/data:/data:ro"),
		other => panic!("expected short, got {other:?}"),
	}
	match &mounts[1] {
		VolumeMount::Long { source, .. } => {
			assert_eq!(source.as_deref(), Some("/srv/app/logs"))
		}
		other => panic!("expected long bind, got {other:?}"),
	}
	// The named volume is untouched.
	match &mounts[2] {
		VolumeMount::Short(s) => assert_eq!(s, "named:/cache"),
		other => panic!("expected short, got {other:?}"),
	}
}

/// A source that starts with a separator is an absolute host path on every
/// platform. On Windows `Path::is_absolute` says no to a bare `/`, which made
/// the audit anchor the host root under the project directory and stop seeing
/// it as the host root.
#[test]
fn a_long_form_bind_of_the_host_root_stays_the_host_root() {
	let mut file = podup::compose::parse_str(
		"services:\n  web:\n    image: x\n    volumes:\n      - type: bind\n        source: /\n        target: /host\n",
	)
	.unwrap();
	let service = file.services.get_mut("web").unwrap();
	anchor_service_bind_sources(service, Path::new("/srv/app"));
	match &service.volumes[0] {
		VolumeMount::Long { source, .. } => assert_eq!(source.as_deref(), Some("/")),
		other => panic!("expected the long form, got {other:?}"),
	}
}
