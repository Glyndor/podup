use super::CountOrAll;

#[test]
fn count_or_all_named_returns_minus_one() {
	assert_eq!(CountOrAll::Named("all".into()).to_i64(), -1);
}

#[test]
fn count_or_all_n_returns_value() {
	assert_eq!(CountOrAll::N(4).to_i64(), 4);
}

// `deploy.resources.limits.pids`

fn pids_of(yaml: &str) -> crate::error::Result<Option<i64>> {
	let file = crate::parse_str(yaml)?;
	let limits = file.services["a"]
		.deploy
		.as_ref()
		.and_then(|d| d.resources.as_ref())
		.and_then(|r| r.limits.as_ref())
		.and_then(|l| l.pids);
	Ok(limits)
}

const PIDS: &str =
	"services:\n  a:\n    image: x\n    deploy:\n      resources:\n        limits:\n          pids: ";

/// `docker compose config` accepts `-1` here (no limit), the same value
/// `pids_limit:` already takes. It failed the whole file with a YAML error that
/// named neither the service nor the key.
#[test]
fn a_pids_limit_of_minus_one_means_unlimited() {
	assert_eq!(pids_of(&format!("{PIDS}-1\n")).unwrap(), Some(-1));
}

#[test]
fn an_ordinary_pids_limit_is_kept() {
	assert_eq!(pids_of(&format!("{PIDS}200\n")).unwrap(), Some(200));
}

/// A value past `i64` saturates instead of wrapping to a negative number, which
/// would read as `-1`.
#[test]
fn a_pids_limit_past_i64_saturates() {
	assert_eq!(
		pids_of(&format!("{PIDS}18446744073709551615\n")).unwrap(),
		Some(i64::MAX)
	);
}

#[test]
fn an_absent_pids_limit_stays_absent() {
	let yaml = "services:\n  a:\n    image: x\n    deploy:\n      resources:\n        limits:\n          memory: 64m\n";
	assert_eq!(pids_of(yaml).unwrap(), None);
}

#[test]
fn the_engine_forwards_minus_one_as_the_pid_limit() {
	let file = crate::parse_str(&format!("{PIDS}-1\n")).unwrap();
	assert_eq!(crate::effective_pids_limit(&file.services["a"]), Some(-1));
}
