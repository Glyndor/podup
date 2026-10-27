use crate::engine::lifecycle::run::merge_run_environment;
use std::collections::HashMap;

fn lookup<'a>(list: &'a [String], key: &str) -> Option<&'a str> {
	// Mirror downstream "later duplicate wins" semantics.
	list.iter().rev().find_map(|e| match e.split_once('=') {
		Some((k, v)) if k == key => Some(v),
		_ => None,
	})
}

#[test]
fn env_file_seeds_environment() {
	let file: HashMap<String, String> = [("FOO".to_string(), "from-file".to_string())].into();
	let list = merge_run_environment(file, HashMap::new(), Vec::new());
	assert_eq!(lookup(&list, "FOO"), Some("from-file"));
}

#[test]
fn service_environment_overrides_env_file() {
	let file: HashMap<String, String> = [("FOO".to_string(), "from-file".to_string())].into();
	let service: HashMap<String, Option<String>> =
		[("FOO".to_string(), Some("from-service".to_string()))].into();
	let list = merge_run_environment(file, service, Vec::new());
	assert_eq!(lookup(&list, "FOO"), Some("from-service"));
}

#[test]
fn dash_e_override_wins_over_all() {
	let file: HashMap<String, String> = [("FOO".to_string(), "from-file".to_string())].into();
	let service: HashMap<String, Option<String>> =
		[("FOO".to_string(), Some("from-service".to_string()))].into();
	let list = merge_run_environment(file, service, vec!["FOO=from-cli".to_string()]);
	assert_eq!(lookup(&list, "FOO"), Some("from-cli"));
}

#[test]
fn distinct_keys_from_each_layer_are_kept() {
	let file: HashMap<String, String> = [("A".to_string(), "a".to_string())].into();
	let service: HashMap<String, Option<String>> =
		[("B".to_string(), Some("b".to_string()))].into();
	let list = merge_run_environment(file, service, vec!["C=c".to_string()]);
	assert_eq!(lookup(&list, "A"), Some("a"));
	assert_eq!(lookup(&list, "B"), Some("b"));
	assert_eq!(lookup(&list, "C"), Some("c"));
}

// `init_service`: what a `run` hands to its init steps.

/// A `run` describes the container it starts: its command, its entrypoint,
/// its terminal and the mounts and labels the operator typed on the
/// command line. The init steps are separate containers, so they keep the
/// service's own values and inherit only the run's environment.
#[test]
fn init_service_keeps_the_service_fields_and_the_run_environment() {
	use crate::compose::types::{Command, EnvVars, Labels, Service, VolumeMount};

	let service: Service = serde_yaml::from_str(
		"image: base\ncommand: [app]\nentrypoint: [/app]\ntty: true\nstdin_open: true\nvolumes: [data:/data]\nlabels: {role: web}\n",
	)
	.unwrap();
	let mut run_service = service.clone();
	run_service.command = Some(Command::Exec(vec!["echo".into()]));
	run_service.entrypoint = Some(Command::Exec(vec!["sh".into()]));
	run_service.tty = Some(false);
	run_service.stdin_open = Some(false);
	run_service
		.volumes
		.push(VolumeMount::Short("/tmp:/tmp".into()));
	run_service.labels = Labels::List(vec!["extra=yes".into()]);
	run_service.user = Some("1234".into());
	run_service.working_dir = Some("/work".into());
	run_service.environment = EnvVars::List(vec!["FROM_RUN=yes".into()]);

	let init = super::init_service::init_service(&service, &run_service);
	let init_value = serde_yaml::to_value(&init).unwrap();
	let service_value = serde_yaml::to_value(&service).unwrap();
	for field in [
		"command",
		"entrypoint",
		"tty",
		"stdin_open",
		"volumes",
		"labels",
	] {
		assert_eq!(
			init_value[field], service_value[field],
			"the init step must keep the service's {field}"
		);
	}
	assert_eq!(init_value["user"], "1234");
	assert_eq!(init_value["working_dir"], "/work");
	assert_eq!(
		init_value["environment"],
		serde_yaml::to_value(EnvVars::List(vec!["FROM_RUN=yes".into()])).unwrap()
	);
}
