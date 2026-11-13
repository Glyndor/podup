use crate::error::{ComposeError, Result};
use crate::parse_str;

fn validate_str(yaml: &str) -> Result<()> {
	let mut file = parse_str(yaml).unwrap();
	crate::compose::normalize_default_network(&mut file);
	super::super::validate::validate(&file)
}

fn service_with(lines: &str) -> String {
	format!("services:\n  web:\n    image: alpine\n{lines}")
}

#[test]
fn cpus_next_to_cpu_period_is_refused() {
	for lines in [
		"    cpus: 1.5\n    cpu_period: 50000\n",
		"    cpu_period: 50000\n    deploy: {resources: {limits: {cpus: \"1.5\"}}}\n",
	] {
		match validate_str(&service_with(lines)).unwrap_err() {
			ComposeError::Invalid(text) => {
				assert!(text.contains("cpus and cpu_period"), "{text}");
			}
			other => panic!("expected ComposeError::Invalid, got {other:?}"),
		}
	}
}

#[test]
fn either_one_alone_or_a_quota_with_a_period_is_accepted() {
	for lines in [
		"    cpus: 1.5\n",
		"    cpu_period: 50000\n    cpu_quota: 75000\n",
		"    cpus: \"\"\n    cpu_period: 50000\n",
	] {
		validate_str(&service_with(lines)).unwrap();
	}
}
