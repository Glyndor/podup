use super::*;

fn dotenv(content: &str) -> HashMap<String, String> {
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(dir.path().join(".env"), content).unwrap();
	load_dotenv(dir.path())
}

#[test]
fn dotenv_value_refers_to_an_earlier_key() {
	let map = dotenv("A=Hello\nB=${A} World\nC=$A!\n");
	assert_eq!(map["B"], "Hello World");
	assert_eq!(map["C"], "Hello!");
}

#[test]
fn dotenv_value_uses_modifiers() {
	let map = dotenv("A=Hello\nC=${PODUP_TEST_UNSET_DOTENV:-dflt}\nE=${A:+alt}\n");
	assert_eq!(map["C"], "dflt");
	assert_eq!(map["E"], "alt");
}

#[test]
fn dotenv_single_quoted_value_stays_literal() {
	let map = dotenv("A=Hello\nD='${A}'\n");
	assert_eq!(map["D"], "${A}");
}

#[test]
fn dotenv_value_refers_to_the_process_environment() {
	let path = std::env::var("PATH").unwrap();
	let map = dotenv("C=$PATH/foo\n");
	assert_eq!(map["C"], format!("{path}/foo"));
}

#[test]
fn dotenv_unset_reference_is_empty() {
	let map = dotenv("B=x${PODUP_TEST_UNSET_DOTENV}y\n");
	assert_eq!(map["B"], "xy");
}

#[test]
fn dotenv_later_key_is_not_visible_to_an_earlier_value() {
	let map = dotenv("B=${A}\nA=late\n");
	assert_eq!(map["B"], "");
}

#[test]
fn dotenv_unusable_reference_keeps_the_value_as_written() {
	let map = dotenv("B=${PODUP_TEST_UNSET_DOTENV:?required}\nOK=1\n");
	assert_eq!(map["B"], "${PODUP_TEST_UNSET_DOTENV:?required}");
	assert_eq!(map["OK"], "1");
}

#[test]
fn env_file_value_refers_to_a_key_of_an_earlier_file() {
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(dir.path().join("a.env"), "A=one\n").unwrap();
	std::fs::write(dir.path().join("b.env"), "B=${A}-two\nA=three\n").unwrap();
	let vars =
		build_vars_with_env_files_strict(dir.path(), &["a.env".to_string(), "b.env".to_string()])
			.unwrap();
	assert_eq!(vars["B"], "one-two");
	assert_eq!(vars["A"], "three");
}

#[test]
fn env_file_single_quoted_value_stays_literal() {
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(dir.path().join("a.env"), "A=x\nL='${A}'\nI=\"${A}\"\n").unwrap();
	let vars = build_vars_with_env_files_strict(dir.path(), &["a.env".to_string()]).unwrap();
	assert_eq!(vars["L"], "${A}");
	assert_eq!(vars["I"], "x");
}

#[test]
fn env_file_unusable_reference_is_an_error() {
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(
		dir.path().join("a.env"),
		"B=${PODUP_TEST_UNSET_DOTENV:?required}\n",
	)
	.unwrap();
	let err = build_vars_with_env_files_strict(dir.path(), &["a.env".to_string()]).unwrap_err();
	assert!(err.to_string().contains("required"), "{err}");
}

#[test]
fn dotenv_layer_leaves_the_process_environment_out() {
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(dir.path().join(".env"), "ONLY_IN_FILE=1\n").unwrap();
	let layer = dotenv_layer(dir.path(), &[]).unwrap();
	assert_eq!(layer.len(), 1);
	assert_eq!(layer["ONLY_IN_FILE"], "1");
}
