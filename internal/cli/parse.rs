//! Argument value parsers for the CLI.

/// Parse a `SERVICE=N` scale argument into a `(service, replicas)` pair.
///
/// Rejects a missing `=`, an empty service name, a non-numeric count, and `N=0`
/// (use `down`/`stop` to remove a service, not `scale=0`). The count must be a
/// run of plain ASCII digits: a leading sign such as `+3` (which `u32::FromStr`
/// would otherwise accept) is rejected so the input contract stays consistent
/// with the already-rejected `-1`/`x`/`0x10` forms.
pub(crate) fn parse_scale_pair(value: &str) -> Result<(String, u32), String> {
	let (service, count) = value
		.split_once('=')
		.ok_or_else(|| format!("expected SERVICE=N, got `{value}`"))?;
	if service.is_empty() {
		return Err(format!("missing service name in `{value}`"));
	}
	if count.is_empty() || !count.bytes().all(|b| b.is_ascii_digit()) {
		return Err(format!(
			"replica count in `{value}` must be a non-negative integer"
		));
	}
	let replicas: u32 = count
		.parse()
		.map_err(|_| format!("replica count in `{value}` must be a non-negative integer"))?;
	if replicas == 0 {
		return Err(format!(
			"replica count in `{value}` must be at least 1; use `down`/`stop` to remove a service"
		));
	}
	Ok((service.to_string(), replicas))
}

#[cfg(test)]
mod tests {
	use super::parse_scale_pair;

	#[test]
	fn parse_scale_pair_accepts_valid() {
		assert_eq!(parse_scale_pair("web=3"), Ok(("web".to_string(), 3)));
	}

	#[test]
	fn parse_scale_pair_rejects_bad_input() {
		// `web=+3` is rejected like the other malformed counts: `u32::FromStr`
		// tolerates a leading '+', so the explicit all-digits guard is what keeps
		// the contract consistent.
		for bad in [
			"web", "=3", "web=", "web=x", "web=0", "web=-1", "web=+3", "web=0x10", "web= 3",
		] {
			assert!(parse_scale_pair(bad).is_err(), "`{bad}` should be rejected");
		}
	}
}
