//! `command:` and `entrypoint:` set to null, or tagged `!reset`, clear what the
//! base file declared. Docker Compose does the same; an absent key keeps it.

fn merged(over: &str) -> crate::compose::types::ComposeFile {
	let dir = tempfile::tempdir().unwrap();
	let base = dir.path().join("base.yaml");
	let over_path = dir.path().join("over.yaml");
	std::fs::write(
		&base,
		"services:\n  web:\n    image: shared\n    command: sleep 1\n    entrypoint: [e]\n",
	)
	.unwrap();
	std::fs::write(&over_path, over).unwrap();
	super::parse_files_with_env_files(&[base, over_path], &[]).unwrap()
}

#[test]
fn null_in_the_override_clears_the_inherited_command_and_entrypoint() {
	let file = merged("services:\n  web:\n    command: null\n    entrypoint: null\n");
	assert!(file.services["web"].command.is_none());
	assert!(file.services["web"].entrypoint.is_none());
}

#[test]
fn reset_in_the_override_clears_the_inherited_command_and_entrypoint() {
	let file = merged("services:\n  web:\n    command: !reset null\n    entrypoint: !reset null\n");
	assert!(file.services["web"].command.is_none());
	assert!(file.services["web"].entrypoint.is_none());
}

#[test]
fn an_absent_key_keeps_the_inherited_command_and_entrypoint() {
	let file = merged("services:\n  web:\n    hostname: h\n");
	assert!(file.services["web"].command.is_some());
	assert!(file.services["web"].entrypoint.is_some());
}

#[test]
fn clearing_one_key_leaves_the_other() {
	let file = merged("services:\n  web:\n    command: null\n");
	assert!(file.services["web"].command.is_none());
	assert!(file.services["web"].entrypoint.is_some());
}

#[test]
fn a_third_file_without_the_key_keeps_the_clear() {
	let dir = tempfile::tempdir().unwrap();
	let files: Vec<_> = [
		"services:\n  web:\n    image: shared\n    command: sleep 1\n",
		"services:\n  web:\n    command: null\n",
		"services:\n  web:\n    hostname: h\n",
	]
	.iter()
	.enumerate()
	.map(|(i, text)| {
		let path = dir.path().join(format!("{i}.yaml"));
		std::fs::write(&path, text).unwrap();
		path
	})
	.collect();
	let file = super::parse_files_with_env_files(&files, &[]).unwrap();
	assert!(file.services["web"].command.is_none());
}
