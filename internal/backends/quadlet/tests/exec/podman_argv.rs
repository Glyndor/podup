//! Argv-tokenisation helpers used by tests to assert that a generated unit's
//! `PodmanArgs=` value cannot smuggle an extra `podman run` flag into the argv
//! quadlet's generator would build from it.
//!
//! The unit text alone is not enough: `PodmanArgs=--memory=512m --privileged`
//! is what `escape_unit_value` produced on the unfixed tree for a hostile
//! `mem_limit: "512m --privileged"`, and it reads as one innocent-looking
//! line. Podman, however, would receive `--memory=512m` and `--privileged` as
//! two separate argv elements. That is the security property the bug breaks,
//! and the only one worth asserting.
//!
//! systemd's `systemd.syntax(7)` word-splitter is the authoritative divider:
//! the directive value after `PodmanArgs=` is tokenised on whitespace while
//! honouring `"`-quoted groups (single quotes and C-style escapes are also
//! honoured by systemd, but the seven interpolation sites have no reason to
//! emit those, so a faithful `PodmanArgs=-side only needs the double-quote
//! form to be tested).

/// Tokenise one `PodmanArgs=` value the way systemd does.
///
/// Returns the argv podman would receive. Tokens coming out of a `"..."`
/// group have their quotes stripped, the way systemd does (the consumed
/// double quotes are not preserved as characters).
pub fn tokenise_podman_argv(value: &str) -> Vec<String> {
	// Work on bytes: systemd splits on ASCII whitespace only, and the generator
	// writes a non-ASCII character as one `\xNN` escape per UTF-8 byte.
	let mut out: Vec<Vec<u8>> = Vec::new();
	let mut current: Vec<u8> = Vec::new();
	let mut started = false;
	let mut in_quotes = false;
	let bytes = value.as_bytes();
	let mut i = 0;
	while i < bytes.len() {
		let c = bytes[i];
		i += 1;
		if c == b'\\' && i < bytes.len() {
			match bytes[i] {
				b'"' | b'\\' if in_quotes => {
					current.push(bytes[i]);
					i += 1;
					started = true;
					continue;
				}
				b'x' if i + 2 < bytes.len()
					&& bytes[i + 1].is_ascii_hexdigit()
					&& bytes[i + 2].is_ascii_hexdigit() =>
				{
					let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).expect("hex escape");
					current.push(u8::from_str_radix(hex, 16).expect("hex escape"));
					i += 3;
					started = true;
					continue;
				}
				_ => {}
			}
		}
		if in_quotes {
			if c == b'"' {
				in_quotes = false;
			} else {
				current.push(c);
			}
			continue;
		}
		match c {
			b'"' => {
				in_quotes = true;
				started = true;
			}
			c if c.is_ascii_whitespace() => {
				if started {
					out.push(std::mem::take(&mut current));
					started = false;
				}
			}
			_ => {
				current.push(c);
				started = true;
			}
		}
	}
	if started {
		out.push(current);
	}
	out.into_iter()
		.map(|word| String::from_utf8_lossy(&word).into_owned())
		.collect()
}

/// Parse every `PodmanArgs=` line out of a unit body, concatenate the values
/// the way systemd does (`PodmanArgs=` is multi-valued by design), and
/// tokenise the combined string. Returns the argv podman would receive when
/// the generator feeds it to `podman run`.
///
/// Comment lines (`#`) are skipped so a unit file's ownership marker does not
/// confuse the parser; the `# podup-owner: <project>` line lives at the top
/// of every generated unit and would otherwise contribute empty tokens.
pub fn podman_argv_from_unit(unit_contents: &str) -> Vec<String> {
	let mut combined = String::new();
	for raw in unit_contents.lines() {
		let line = raw.trim_end();
		if line.starts_with('#') {
			continue;
		}
		if let Some(rest) = line.strip_prefix("PodmanArgs=") {
			if !combined.is_empty() {
				combined.push('\n');
			}
			combined.push_str(rest);
		}
	}
	tokenise_podman_argv(&combined)
}

/// Assert that no `PodmanArgs=` line in `unit_contents` produces `needle` as
/// its own argv element after systemd tokenisation. Used by the seven
/// per-site tests to pin the property the bug breaks.
pub fn assert_argv_has_no_token(unit_contents: &str, needle: &str) {
	let argv = podman_argv_from_unit(unit_contents);
	assert!(
		!argv.iter().any(|tok| tok == needle),
		"unit produces {needle:?} as its own argv element; argv = {argv:#?}; unit:\n{unit_contents}"
	);
}

#[test]
fn tokenise_splits_on_whitespace() {
	assert_eq!(
		tokenise_podman_argv("--memory=512m --privileged"),
		vec!["--memory=512m", "--privileged"]
	);
}

#[test]
fn tokenise_respects_double_quotes() {
	// `--cpuset-cpus="0 --privileged"` is ONE argv element. That is what
	// the seven-site fix must produce, and what this helper proves.
	assert_eq!(
		tokenise_podman_argv(r#"--cpuset-cpus="0 --privileged""#),
		vec!["--cpuset-cpus=0 --privileged"]
	);
}

#[test]
fn multiple_podman_args_lines_concatenate() {
	// systemd accepts multiple `PodmanArgs=` lines; their values merge.
	let unit = "\
PodmanArgs=--memory=512m
PodmanArgs=--cpuset-cpus=0,1
";
	assert_eq!(
		podman_argv_from_unit(unit),
		vec!["--memory=512m", "--cpuset-cpus=0,1"]
	);
}

#[test]
fn comments_are_ignored() {
	// The podup ownership marker is the first line of every unit.
	let unit = "# podup-owner: p\nPodmanArgs=--privileged\n";
	assert_eq!(podman_argv_from_unit(unit), vec!["--privileged"]);
}
