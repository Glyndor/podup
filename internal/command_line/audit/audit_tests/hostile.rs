use super::*;

use crate::cli::ConfigFormat;
use crate::startup::{render_config_to, ConfigOutput};
use std::path::Path;

// ---------------------------------------------------------------------------
// #1746 entry 2: terminal-escape injection in five sinks.
//
// The detail lines emitted by `podup audit` and the four `podup config
// --services/--volumes/--images/--profiles/--hash` projections print
// user-controlled compose values verbatim. A compose file authored by a
// third party can therefore inject raw terminal escapes; the cross-review
// noted that the bytes survive a pipe, because the rendering path is just
// `println!("{user_value}")`.
//
// The five sinks:
//
//   1. `audit` table detail lines (`render_table_to`)
//   2. `config --services` (`render_config_to`)
//   3. `config --volumes`  (`render_config_to`)
//   4. `config --images`   (`render_config_to`)
//   5. `config --profiles` (`render_config_to`)
//
// The audit table cells and the JSON renderer are intentionally out of
// scope: the table goes through `Table::print` -> `fit_cell` ->
// `sanitize_cell`, and `serde_json` escapes every control character. The
// "16 raw ESC bytes" payload the issue names only reaches the operator
// through the five unescaped sinks above, so this test pins those five
// and ignores the rest.
//
// The fix routes every user-controlled value through
// `podup::ui::sanitize_cell`, the same gate the table already uses. The
// test asserts that the bytes never appear verbatim, regardless of which
// user-controlled slot the attacker writes them into.
//
// The YAML parser itself rejects raw control characters in scalars, so
// the attack reaches the parsed model through env-var interpolation:
// the compose file says `image: $HOSTILE` and the operator's
// environment sets `HOSTILE` to the byte sequence. The interpolated
// scalar fallback in `merge::interpolate_scalar` stores the resolved
// string verbatim when the bytes are not a valid YAML scalar. The test
// constructs the `ComposeFile` directly with the interpolated values to
// pin the renderer behaviour without coupling to the parser's own
// rejection (which the issue does not ask to change).
// ---------------------------------------------------------------------------

/// The payload the issue names: a 16-byte sequence that, raw, asks the
/// terminal to clear the screen and move the cursor home. The exact
/// content does not matter; what matters is that the byte stream the
/// renderer produces for a sink using the value does not contain any
/// raw `\x1b` byte.
const HOSTILE: &str = "\x1b[2J\x1b[HINJECTED";

/// A `ComposeFile` whose every user-controlled slot the renderers
/// consume carries the same hostile payload. The service name is left
/// valid (compose validation rejects ESC chars in service names, so
/// that vector is not reachable from a YAML file in production); the
/// other slots are not validated against control characters and reach
/// the renderers verbatim when the typed model is built with them.
///
/// Specifically, each field is the value a different sink prints:
///
///   - `services.web.image`: the audit reason (via
///     `check_unpinned_image`) AND the `config --images` projection
///   - `services.web.cap_add`: the audit reason (via
///     `check_dangerous_capability`)
///   - `services.web.profiles`: the `config --profiles` projection
///   - `volumes[HOSTILE]`: the `config --volumes` projection
///
/// `Service` and `ComposeFile` are `#[non_exhaustive]`, so the test
/// builds a clean file from a safe YAML first, then mutates the typed
/// model in place to plant the hostile payload. This is the same shape
/// a hostile env-var interpolation (`image: $HOSTILE` with
/// `HOSTILE=\x1b[2J:latest`) would leave the model in: a string field
/// carrying a sequence of bytes the YAML parser would have refused, but
/// the typed model carries without question.
fn hostile_compose() -> podup::compose::types::ComposeFile {
	let yaml = "services:\n  web:\n    image: nginx:1.27\n    cap_add: [SYS_ADMIN]\n    \
	            profiles: [safe]\nvolumes:\n  data: null\n";
	let mut file = podup::parse_str(yaml).expect("safe compose parses");
	let web = file.services.get_mut("web").expect("web present");
	web.image = Some(format!("{HOSTILE}:latest"));
	web.cap_add = vec![HOSTILE.to_string()];
	web.profiles = vec![HOSTILE.to_string()];
	// The named volume reaches the `config --volumes` projection
	// directly. Volumes are not validated against control characters;
	// a hostile env interpolation would land here.
	let vol = file
		.volumes
		.shift_remove("data")
		.expect("data volume present");
	file.volumes.insert(HOSTILE.to_string(), vol);
	file
}

fn assert_no_hostile(label: &str, bytes: &[u8]) {
	// The hostile payload is a literal byte sequence, not a generic
	// "any ESC byte" check. Podup's own styling always emits SGR
	// sequences (`\x1b[1m`, `\x1b[0m`), so a raw byte count would
	// alert on those too; the meaningful signal is whether the exact
	// payload the attacker planted reaches the byte stream.
	let haystack = String::from_utf8_lossy(bytes);
	assert!(
		!haystack.contains(HOSTILE),
		"{label} carried the hostile payload verbatim; the attacker's compose file reached the \
		 operator's terminal. Output was: {haystack}",
	);
}

#[test]
fn hostile_compose_does_not_inject_escapes_into_audit_table_detail_lines() {
	// Sink 1: the audit table's per-finding detail lines. The service name
	// and the reason both come from user-controlled compose data; both
	// must be sanitized through `sanitize_cell` before they reach the byte
	// stream the operator (or a pipe) sees.
	let file = hostile_compose();
	let report = audit_file(&file, &[]);
	let services = audited_containers(&file);
	let mut buf: Vec<u8> = Vec::new();
	render_table_to(&mut buf, &services, &report).expect("render");
	assert_no_hostile("audit detail lines", &buf);
	// Positive control: the renderer did produce output, and the host's
	// word (sanitized) is still in the byte stream somewhere. Without
	// this assertion a test that simply returned no output would also
	// pass `assert_no_hostile`.
	let out = String::from_utf8_lossy(&buf);
	assert!(
		out.contains("INJECTED"),
		"the host's word is gone entirely: {out}"
	);
}

#[test]
fn hostile_compose_does_not_inject_escapes_into_config_list_projections() {
	// Sinks 2..5: the four `config` list-projection branches all print a
	// value taken from the compose file. They share the `_to` writer so a
	// single test covers all four.
	let file = hostile_compose();
	let mut buf: Vec<u8> = Vec::new();

	let projections: [(&str, ConfigOutput); 4] = [
		(
			"--services",
			ConfigOutput {
				services: true,
				..Default::default()
			},
		),
		(
			"--volumes",
			ConfigOutput {
				volumes: true,
				..Default::default()
			},
		),
		(
			"--images",
			ConfigOutput {
				images: true,
				..Default::default()
			},
		),
		(
			"--profiles",
			ConfigOutput {
				profiles: true,
				..Default::default()
			},
		),
	];
	for (name, out) in projections {
		buf.clear();
		render_config_to(
			&mut buf,
			&file,
			&ConfigFormat::Yaml,
			&out,
			"proj",
			Path::new("/proj"),
		)
		.unwrap_or_else(|e| panic!("{name} render: {e}"));
		assert_no_hostile(name, &buf);
	}

	// Sink 6: `config --hash` also prints the service name. The branch
	// runs the same `sanitize_cell` gate as the other projections.
	buf.clear();
	render_config_to(
		&mut buf,
		&file,
		&ConfigFormat::Yaml,
		&ConfigOutput {
			hash: Some("*".to_string()),
			..Default::default()
		},
		"proj",
		Path::new("/proj"),
	)
	.expect("--hash render");
	assert_no_hostile("--hash", &buf);
	// The 64-character hex hash is bytes we own, so the only sanitized
	// thing in this line is the service name. The print still produces a
	// line per service, so the line count is exactly one.
	assert_eq!(buf.iter().filter(|&&b| b == b'\n').count(), 1);
}
