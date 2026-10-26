use super::*;

/// `podup logs --wrong-flag` reaches `format_clap_error` with an
/// `UnknownArgument` error and the rendered output must lead with the
/// binary's `podup:` prefix and the bold-red `error:` label, matching
/// `exit_status::print_error` and the `tracing` formatter. The clap usage
/// block follows, unprefixed, so a script that greps `podup:` knows what
/// category the failure is.
#[test]
fn format_clap_error_prefixes_an_unknown_argument() {
	use clap::Parser;
	let err = match Cli::try_parse_from(["podup", "logs", "--wrong-flag"]) {
		Err(e) => e,
		Ok(_) => panic!("--wrong-flag is not a valid flag"),
	};
	let rendered = format_clap_error(&err);
	assert!(
		rendered.starts_with("podup: "),
		"missing `podup:` prefix: {rendered:?}"
	);
	assert!(
		rendered.contains("error"),
		"missing the bold-red `error:` label: {rendered:?}"
	);
	assert!(
		rendered.contains("--wrong-flag"),
		"the offending flag is missing from the message: {rendered:?}"
	);
	// clap still emits the usage block; the prefix applies to the leading
	// line only and the rest stays verbatim so a user can read it.
	assert!(
		rendered.contains("Usage:"),
		"the clap usage block is missing: {rendered:?}"
	);
}

/// `podup scale` requires a `SERVICE=N` positional, so omitting it is a
/// `MissingRequiredArgument`. The error must read the same as any other:
/// `podup: error: …`, not a bare clap line.
#[test]
fn format_clap_error_prefixes_a_missing_required_argument() {
	use clap::Parser;
	let err = match Cli::try_parse_from(["podup", "scale"]) {
		Err(e) => e,
		Ok(_) => panic!("scale requires at least one SERVICE=N pair"),
	};
	let rendered = format_clap_error(&err);
	assert!(
		rendered.starts_with("podup: "),
		"missing `podup:` prefix: {rendered:?}"
	);
	assert!(
		rendered.contains("error"),
		"missing the bold-red `error:` label: {rendered:?}"
	);
}

/// An enum-typed arg (`--rmi` on `down`, `RmiScope`) must surface the
/// same way. clap's enum-validation errors carry the kind `InvalidValue`,
/// which is a different code path than unknown args; the prefix must
/// apply uniformly.
#[test]
fn format_clap_error_prefixes_an_invalid_enum_value() {
	use clap::Parser;
	let err = match Cli::try_parse_from(["podup", "down", "--rmi", "bogus"]) {
		Err(e) => e,
		Ok(_) => panic!("--rmi takes `all` or `local`, not `bogus`"),
	};
	let rendered = format_clap_error(&err);
	assert!(
		rendered.starts_with("podup: "),
		"missing `podup:` prefix: {rendered:?}"
	);
	assert!(
		rendered.contains("error"),
		"missing the bold-red `error:` label: {rendered:?}"
	);
	assert!(
		rendered.contains("bogus"),
		"the offending value is missing from the message: {rendered:?}"
	);
}
