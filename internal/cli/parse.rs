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

/// Pull-policy values podup accepts for `up --pull` / `pull --policy`. `always`,
/// `missing`, `never`, and `build` mirror `docker compose`; `newer` is Podman's
/// extension.
const PULL_POLICIES: [&str; 5] = ["always", "missing", "never", "newer", "build"];

/// Validate a `--pull` / `--policy` value at parse time, rejecting unknown values
/// with a clear message instead of silently defaulting to `missing` at runtime.
pub(crate) fn parse_pull_policy(value: &str) -> Result<String, String> {
	if PULL_POLICIES.contains(&value) {
		Ok(value.to_string())
	} else {
		Err(format!(
			"invalid pull policy `{value}` (expected one of: {})",
			PULL_POLICIES.join(", ")
		))
	}
}

/// Parse a `-t/--timeout` shutdown-grace value, rejecting negatives with a clear
/// range error rather than forwarding `-5` to Podman or letting clap report a
/// confusing "unexpected argument" for the space form.
pub(crate) fn parse_timeout(value: &str) -> Result<i32, String> {
	let secs: i32 = value
		.parse()
		.map_err(|_| format!("timeout `{value}` must be an integer number of seconds"))?;
	if secs < 0 {
		return Err(format!("timeout `{value}` must be zero or greater"));
	}
	Ok(secs)
}

#[cfg(test)]
mod tests {
	use super::{parse_pull_policy, parse_scale_pair, parse_timeout};

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

	#[test]
	fn parse_pull_policy_accepts_known_values() {
		for ok in ["always", "missing", "never", "newer", "build"] {
			assert_eq!(parse_pull_policy(ok), Ok(ok.to_string()));
		}
	}

	#[test]
	fn parse_pull_policy_rejects_unknown_values() {
		for bad in ["bogus", "Always", "if_not_present", ""] {
			assert!(
				parse_pull_policy(bad).is_err(),
				"`{bad}` should be rejected"
			);
		}
	}

	#[test]
	fn parse_timeout_accepts_zero_and_positive() {
		assert_eq!(parse_timeout("0"), Ok(0));
		assert_eq!(parse_timeout("30"), Ok(30));
	}

	#[test]
	fn parse_timeout_rejects_negative_and_non_numeric() {
		assert!(parse_timeout("-5").is_err());
		assert!(parse_timeout("abc").is_err());
	}
}
