//! Argument value parsers for the CLI.

/// Parse a `SERVICE=N` scale argument into a `(service, replicas)` pair.
///
/// Rejects a missing `=`, an empty service name, a non-numeric count, and `N=0`
/// (use `down`/`stop` to remove a service, not `scale=0`).
pub(crate) fn parse_scale_pair(value: &str) -> Result<(String, u32), String> {
	let (service, count) = value
		.split_once('=')
		.ok_or_else(|| format!("expected SERVICE=N, got `{value}`"))?;
	if service.is_empty() {
		return Err(format!("missing service name in `{value}`"));
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
		for bad in ["web", "=3", "web=", "web=x", "web=0", "web=-1"] {
			assert!(parse_scale_pair(bad).is_err(), "`{bad}` should be rejected");
		}
	}
}
