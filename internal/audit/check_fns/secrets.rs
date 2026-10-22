use podup::compose::types::{ComposeFile, Service};

use super::{finding, Finding};

/// Field-name segments the `secret_in_environment` check matches after
/// splitting: `PASSWORD`, `SECRET`, `TOKEN`, `KEY`. Comparison is segment
/// equality, case-insensitive (segments are upper-cased before the
/// lookup). Do not rename this back to a substring list: `contains` here
/// reported `MONKEY_HABITAT` and `MD_Keywords` as hard-coded secrets
/// (#1709), and the name is what says which of the two this is.
const SECRET_NAME_SEGMENTS: &[&str] = &["PASSWORD", "SECRET", "TOKEN", "KEY"];

/// Split an environment key into upper-case segments. Splits on the
/// separators `_`, `-`, and `.`, and at camelCase (lower-to-upper) and
/// PascalCase (upper-upper-followed-by-lower) boundaries. The camelCase
/// split is what kept `apiToken` matching under the new rule; without
/// it the helper would have treated `apiToken` as a single segment and
/// the check would have stopped flagging it after the substring match
/// was removed.
pub fn segments(name: &str) -> Vec<String> {
	let mut out = Vec::new();
	for part in name.split(['_', '-', '.']) {
		if part.is_empty() {
			continue;
		}
		// Env keys are conventional identifiers and the keyword list is
		// ASCII; working in bytes keeps the boundary check free of a
		// UTF-8 decode at every step. A non-ASCII byte is upper-cased
		// as-is and never splits on either side.
		let bytes = part.as_bytes();
		let mut start = 0;
		let mut i = 1;
		while i < bytes.len() {
			let prev = bytes[i - 1];
			let cur = bytes[i];
			// Lower-to-upper: `aB`. A non-ASCII lead byte counts as the
			// lower side: before this, `contains` flagged `NKey` spelled
			// with a leading n-tilde, and dropping that would have been a
			// silent false negative rather than the false positive #1709
			// set out to remove. The split index is always an ASCII byte,
			// so it is always a char boundary.
			let lower_upper =
				(prev.is_ascii_lowercase() || !prev.is_ascii()) && cur.is_ascii_uppercase();
			// Upper-upper-followed-by-lower: `ABc` (PascalCase).
			let upper_upper_lower = prev.is_ascii_uppercase()
				&& cur.is_ascii_uppercase()
				&& bytes.get(i + 1).is_some_and(|b| b.is_ascii_lowercase());
			if lower_upper || upper_upper_lower {
				out.push(part[start..i].to_ascii_uppercase());
				start = i;
			}
			i += 1;
		}
		out.push(part[start..].to_ascii_uppercase());
	}
	out
}

/// `environment:` key whose name, split into segments on `_`, `-`, `.`,
/// and camelCase/PascalCase boundaries, has a segment equal to
/// `PASSWORD|SECRET|TOKEN|KEY` (case-insensitive) AND carries a value in
/// compose (literal or interpolated from `${VAR}`). Four exemptions
/// survive and are asserted: a bare key that inherits from the host, a
/// key whose name merely contains a secret-looking segment while its
/// value is not one (`PASSTHROUGH: "true"`, segment filter), a key whose
/// value resolved to the empty literal (`LOG_LEVEL=` for an unrelated
/// key, segment filter), and a `_FILE` key whose value has the shape of
/// a path.
///
/// The `_FILE` suffix is the documented convention for keeping a secret
/// out of the environment: the value is a path to a file the application
/// reads at runtime. The official Postgres, MariaDB, and MySQL images all
/// spell this `<NAME>_FILE` and point it at a Docker/Kubernetes secrets
/// mount, the canonical one being `/run/secrets/<name>`. The path is not
/// the secret itself, so the key carrying it is not a secret in the
/// environment when the value is path-shaped.
///
/// The exemption is the suffix AND the value's shape, not either alone:
/// the suffix names the convention, the shape is what makes it a path
/// reference. The shape test is the three prefixes an absolute or
/// relative path can take (`/run/secrets/pg`, `./secrets/pg`, `../pg`);
/// a literal password written into a `_FILE` key (`POSTGRES_PASSWORD_FILE:
/// hunter2-real-password`) is still a literal password and the check
/// flags it. Values that merely contain a slash but do not start with one
/// of the three prefixes (`relative/path`, `a/b`) read as plain text, not
/// as a path, and are flagged the same way. Verifying what the path
/// points to (file permissions, mount provenance) is a different audit
/// concern and is out of scope here: the check decides on shape, not on
/// whether the file exists.
///
/// The risk is the same in both value shapes: the value ends up in the
/// container's environment whether it was written literally or injected
/// from the operator's environment through `${VAR}`. A gate whose verdict
/// depends on whether the variable happens to be exported in the caller's
/// shell is not a gate, so both shapes are flagged. A `_FILE` key is
/// judged on the same resolved value: `${VAR}` holding a path stays
/// silent, `${VAR}` holding anything else is flagged, and an unset
/// `${VAR}` resolves to the empty string, which is not a path and is
/// flagged too.
///
/// Service-local: the check is a positional grep on the `environment:` map
/// of this service. These are surfaced so the operator can move them to
/// `secrets:`; the wider question of whether the project declares
/// `secrets:` is not in scope.
pub fn check_secret_in_environment(
	name: &str,
	service: &Service,
	_file: &ComposeFile,
) -> Vec<Finding> {
	let mut out = Vec::new();
	for (key, value) in service.environment.to_map() {
		let key_segments = super::segments(&key);
		if !key_segments
			.iter()
			.any(|s| SECRET_NAME_SEGMENTS.contains(&s.as_str()))
		{
			continue;
		}
		if value.is_none() {
			// Bare key: inherited from the host. Not a published secret.
			continue;
		}
		// The `_FILE` suffix is the operator's signal that the value is a
		// path to a file the application reads at runtime, not a secret
		// in the environment. Tested as "the last segment is FILE" so the
		// `_`, `-`, `.` and camelCase split the helper already does is
		// reused: `POSTGRES_PASSWORD_FILE`, `MY_KEY_FILE`,
		// `KEY-FILE`, `KeyFile` all reach the same conclusion; a name
		// like `PASSWORD_FILE_BACKUP` does not and still fires.
		//
		// The suffix alone is not enough: a literal password written
		// into a `_FILE` key (`POSTGRES_PASSWORD_FILE:
		// hunter2-real-password`) is still a literal password. The
		// exemption holds only when the value is path-shaped: it starts
		// with `/`, `./`, or `../`. Any other value falls through to the
		// flagging rule like a value under any other secret-bearing key.
		if key_segments.last().is_some_and(|s| s == "FILE")
			&& value
				.as_deref()
				.is_some_and(|v| v.starts_with('/') || v.starts_with("./") || v.starts_with("../"))
		{
			continue;
		}
		// The empty-value branch used to skip here. That branch is what
		// made the verdict depend on the caller's shell: an unset
		// `${VAR}` reference resolves to the empty string at parse time,
		// so `KEY=${VAR}` produced an empty value and was skipped, while
		// `KEY=${VAR}` with `VAR` exported produced the literal and was
		// flagged. The risk is the same in both cases: the secret ends up
		// in the container's environment whether it was authored as a
		// literal or interpolated from `${VAR}`. Move it to `secrets:`.
		out.push(finding(
			name,
			"secret_in_environment",
			&format!("environment: {key} is set in compose; move it to secrets:"),
		));
	}
	out
}
