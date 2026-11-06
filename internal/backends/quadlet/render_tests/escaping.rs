use super::*;

// --- escape_unit_value (bug: incomplete systemd value escaping) ----------

#[test]
fn sanitize_value_strips_control_characters() {
	assert_eq!(sanitize_value("plain"), "plain");
	assert_eq!(sanitize_value("a\nb\tc\r"), "abc");
}

#[test]
fn env_file_paths_are_single_literal_words() {
	for (path, expected) in [
		(r#"a b"c\d.env"#, "\"a b\\\"c\\\\d.env\""),
		("%h", "%%h"),
		("$x", "$$x"),
		("simple.env", "simple.env"),
	] {
		let mut section = Section::new("Container");
		section.add("EnvironmentFile", path.to_string());
		assert_eq!(
			section.render(),
			format!("[Container]\nEnvironmentFile={expected}\n"),
			"env_file path {path:?}"
		);
	}
}

#[test]
fn escape_word_split_value_with_whitespace_is_quoted() {
	// An Environment value with whitespace must be quoted so systemd keeps it
	// as one value instead of splitting it into bogus extra entries.
	assert_eq!(
		escape_unit_value("Environment", "JAVA_OPTS=-Xmx512m -Xms256m"),
		"\"JAVA_OPTS=-Xmx512m -Xms256m\""
	);
	// A Label is word-split too.
	assert_eq!(
		escape_unit_value("Label", "note=hello world"),
		"\"note=hello world\""
	);
	// A scalar key that is not word-split keeps whitespace unquoted.
	assert_eq!(
		escape_unit_value("Description", "web (podup)"),
		"web (podup)"
	);
}

#[test]
fn escape_trailing_backslash_is_quoted_and_escaped() {
	// A value ending in a backslash would otherwise continue onto, and
	// swallow, the next directive line; it must be quoted and escaped.
	let out = escape_unit_value("Environment", "WINPATH=C:\\tmp\\");
	assert!(!out.ends_with('\\') || out.ends_with("\\\""));
	assert_eq!(out, "\"WINPATH=C:\\\\tmp\\\\\"");
}

#[test]
fn word_split_value_keeps_quotes_backslashes_and_apostrophes() {
	// A word-split value is read back with the quotes and the escapes
	// honoured, so a `"`, a `'` or a `\` in the middle of it has to be
	// written escaped: written raw, the reader eats the quote, swallows the
	// character after the backslash, or leaves the entry with an unclosed
	// quote and drops it.
	for (key, value) in [
		("Environment", r#"APP_CONFIG={"a":1}"#),
		("Environment", "NAME=O'Brien"),
		("Environment", r"REGEX=^\d+$"),
		("Label", r#"note={"x":"y"}"#),
		("Annotation", r#"org.example={"a":1}"#),
		("Sysctl", r#"net.ipv4.conf.all.rp_filter="1""#),
		("Secret", r#"tok={"a":1}"#),
	] {
		let out = escape_unit_value(key, value);
		let (words, closed) = exec_words(&out);
		assert!(closed, "{key} value {value:?} left a quote open: {out}");
		assert_eq!(
			words,
			vec![value.to_string()],
			"{key} value {value:?} was rewritten to {out}"
		);
	}
}

#[test]
fn word_split_value_keeps_newlines_as_escapes() {
	// A multi-line value (a PEM key, a script) has to reach the container with
	// its line breaks intact, and it must stay on one physical line of the
	// unit file: a raw newline there would split the directive.
	let value = "-----BEGIN KEY-----\nsecond line\n-----END KEY-----";
	let out = escape_unit_value("Environment", &format!("TLS_KEY={value}"));
	assert!(
		!out.contains('\n'),
		"the rendered value must stay on one line: {out}"
	);
	let (words, closed) = exec_words(&out);
	assert!(closed, "the rendered value left a quote open: {out}");
	assert_eq!(words, vec![format!("TLS_KEY={value}")], "got: {out}");
}

#[test]
fn escape_percent_is_doubled_for_literal() {
	// systemd specifiers like %h must be passed through literally, not
	// expanded at unit-activation time, matching docker-compose semantics.
	assert_eq!(escape_unit_value("Environment", "HOME=%h"), "HOME=%%h");
	assert_eq!(escape_unit_value("Image", "img%U"), "img%%U");
}

#[test]
fn escape_arg_line_keys_are_left_intact() {
	// PodmanArgs/Exec/Entrypoint encode their own whitespace splitting and
	// must not be quoted or have `%` doubled.
	assert_eq!(
		escape_unit_value("PodmanArgs", "--security-opt apparmor=foo"),
		"--security-opt apparmor=foo"
	);
	assert_eq!(
		escape_unit_value("Exec", "sh -c \"echo %s\""),
		"sh -c \"echo %s\""
	);
}

// --- quote_podman_arg_value (smuggle guard for the seven sites) -----------

#[test]
fn quote_podman_arg_value_always_quotes_and_doubles_percent() {
	// The seven interpolation sites call this on a raw compose value before
	// splicing it into a `--flag=value` template. The result must be one
	// systemd-token, regardless of whether the value has whitespace.
	assert_eq!(quote_podman_arg_value("512m"), "\"512m\"");
	// A value carrying whitespace must still be one quoted token; the
	// smuggled `--privileged` would otherwise become its own argv element.
	let quoted = quote_podman_arg_value("0 --privileged -v /:/hostfs2");
	assert_eq!(quoted, "\"0 --privileged -v /:/hostfs2\"");
}

#[test]
fn quote_podman_arg_value_strips_control_characters() {
	// A newline injection must be flattened; the sanitiser strips it so the
	// value stays one physical line inside the PodmanArgs= directive.
	assert_eq!(
		quote_podman_arg_value("ok\nExecStartPre=/bin/rm -rf /"),
		"\"okExecStartPre=/bin/rm -rf /\""
	);
}

#[test]
fn quote_podman_arg_value_doubles_percent_for_literal() {
	// systemd specifiers like %h must not be expanded; doubling to %%h is
	// what Environment= does (#1734). podman receives the literal `%h`.
	assert_eq!(quote_podman_arg_value("%h/mem"), "\"%%h/mem\"");
}

#[test]
fn quote_podman_arg_value_escapes_backslash_and_quote() {
	// A backslash inside the quoted group would fold the next physical
	// line, swallowing whatever directive follows; an unescaped `"` would
	// terminate the quoted group early.
	assert_eq!(quote_podman_arg_value("back\\slash"), "\"back\\\\slash\"");
	assert_eq!(quote_podman_arg_value("with\"quote"), "\"with\\\"quote\"");
}
