use crate::compose::ComposeFile;
use crate::parse_str;

fn check(yaml: &str) -> crate::error::Result<()> {
	let mut file: ComposeFile = parse_str(yaml).unwrap();
	crate::compose::normalize_default_network(&mut file);
	super::super::validate::validate(&file)
}

fn service(extra: &str) -> String {
	format!("services:\n  web:\n    image: x\n{extra}")
}

#[test]
fn distinct_scale_and_replicas_name_the_service_and_both_values() {
	let yaml = service("    scale: 2\n    deploy:\n      replicas: 3\n");
	let text = check(&yaml).expect_err("distinct counts").to_string();
	assert!(text.contains("service 'web'"), "{text}");
	assert!(text.contains("'scale' (2)"), "{text}");
	assert!(text.contains("'deploy.replicas' (3)"), "{text}");
}

#[test]
fn equal_scale_and_replicas_are_accepted() {
	let yaml = service("    scale: 3\n    deploy:\n      replicas: 3\n");
	check(&yaml).expect("equal counts");
}

#[test]
fn either_count_alone_is_accepted() {
	check(&service("    scale: 2\n")).expect("scale alone");
	check(&service("    deploy:\n      replicas: 3\n")).expect("replicas alone");
}
