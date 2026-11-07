//! Interpolation of `env_file:` values: the Compose Specification applies it to
//! unquoted and double-quoted values and Docker Compose does too. Each test uses
//! variable names of its own, because the process environment is shared by every
//! test in the binary.

use super::*;
use crate::compose::types::EnvFileEntry;

fn load(dir: &std::path::Path, files: &[(&str, &str)]) -> HashMap<String, String> {
	let mut entries = Vec::new();
	for (name, text) in files {
		std::fs::write(dir.join(name), text).unwrap();
		entries.push(EnvFileEntry::Path((*name).to_string()));
	}
	load_env_file_entries(&entries, dir).unwrap()
}

#[test]
fn unquoted_and_double_quoted_values_are_interpolated_and_single_quoted_are_not() {
	std::env::set_var("PODUP_EFI_HOST", "/home/zz");
	let dir = tempfile::tempdir().unwrap();
	let m = load(
		dir.path(),
		&[(
			"a.env",
			"A=${PODUP_EFI_HOST}\nB=\"dq $PODUP_EFI_HOST\"\nC='sq ${PODUP_EFI_HOST}'\nD=$PODUP_EFI_HOST/x\n",
		)],
	);
	assert_eq!(m["A"], "/home/zz");
	assert_eq!(m["B"], "dq /home/zz");
	assert_eq!(m["C"], "sq ${PODUP_EFI_HOST}");
	assert_eq!(m["D"], "/home/zz/x");
}

#[test]
fn defaults_escapes_and_unset_variables_follow_the_compose_syntax() {
	std::env::remove_var("PODUP_EFI_UNSET");
	std::env::set_var("PODUP_EFI_EMPTY", "");
	let dir = tempfile::tempdir().unwrap();
	let m = load(
		dir.path(),
		&[(
			"a.env",
			"E=${PODUP_EFI_UNSET:-dflt}\nF=$$dollar\nG=a$PODUP_EFI_UNSET\nH=pre${PODUP_EFI_EMPTY:-e}post\nI=${PODUP_EFI_UNSET:+alt}\n",
		)],
	);
	assert_eq!(m["E"], "dflt");
	assert_eq!(m["F"], "$dollar");
	assert_eq!(m["G"], "a");
	assert_eq!(m["H"], "preepost");
	assert_eq!(m["I"], "");
}

#[test]
fn a_value_sees_the_keys_set_above_it_and_in_the_files_before_it_but_not_below() {
	let dir = tempfile::tempdir().unwrap();
	let m = load(
		dir.path(),
		&[
			("a.env", "PODUP_EFI_ONE=1\nTWO=${PODUP_EFI_ONE}x\nLATER_REF=${PODUP_EFI_LATER}x\nPODUP_EFI_LATER=l\n"),
			("b.env", "THREE=${PODUP_EFI_ONE}${PODUP_EFI_LATER}y\n"),
		],
	);
	assert_eq!(m["TWO"], "1x");
	assert_eq!(m["LATER_REF"], "x");
	assert_eq!(m["THREE"], "1ly");
}

#[test]
fn the_process_environment_wins_over_a_key_of_the_file_when_a_value_refers_to_it() {
	std::env::set_var("PODUP_EFI_BOTH", "from-process");
	let dir = tempfile::tempdir().unwrap();
	let m = load(
		dir.path(),
		&[(
			"a.env",
			"PODUP_EFI_BOTH=from-file\nREF=${PODUP_EFI_BOTH}_ref\n",
		)],
	);
	assert_eq!(m["REF"], "from-process_ref");
	assert_eq!(m["PODUP_EFI_BOTH"], "from-file");
}

#[test]
fn the_project_dot_env_is_part_of_the_environment_a_value_can_refer_to() {
	std::env::remove_var("PODUP_EFI_ONLY_IN_DOTENV");
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(
		dir.path().join(".env"),
		"PODUP_EFI_ONLY_IN_DOTENV=fromdotenv\n",
	)
	.unwrap();
	let m = load(dir.path(), &[("a.env", "U=${PODUP_EFI_ONLY_IN_DOTENV}\n")]);
	assert_eq!(m["U"], "fromdotenv");
}

#[test]
fn an_unterminated_modifier_is_an_error_not_a_literal() {
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(dir.path().join("a.env"), "M=${PODUP_EFI_UNSET:-dflt\n").unwrap();
	let err = load_env_file_entries(&[EnvFileEntry::Path("a.env".into())], dir.path());
	assert!(err.is_err(), "{err:?}");
}

#[test]
fn a_bare_key_passed_through_from_the_host_is_not_interpolated() {
	std::env::set_var("PODUP_EFI_BARE", "has-$dollar-${sign}");
	let dir = tempfile::tempdir().unwrap();
	let m = load(dir.path(), &[("a.env", "PODUP_EFI_BARE\n")]);
	assert_eq!(m["PODUP_EFI_BARE"], "has-$dollar-${sign}");
}

#[test]
fn many_values_that_expand_a_large_variable_share_one_budget() {
	// 40 references to a 1 MiB variable are 40 MiB of output, past the 16 MiB a
	// document may expand to; each value alone is far below every cap.
	// In the project `.env`, not the process environment: a megabyte-long variable
	// there would be inherited by every child process other tests spawn.
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(
		dir.path().join(".env"),
		format!("PODUP_EFI_BIG={}\n", "x".repeat(1024 * 1024)),
	)
	.unwrap();
	let text: String = (0..40)
		.map(|i| format!("K{i}=${{PODUP_EFI_BIG}}\n"))
		.collect();
	std::fs::write(dir.path().join("a.env"), text).unwrap();
	let err = load_env_file_entries(&[EnvFileEntry::Path("a.env".into())], dir.path());
	assert!(err.is_err(), "40 MiB of expansion was accepted");
}

#[test]
fn a_few_values_that_expand_a_large_variable_still_load() {
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(
		dir.path().join(".env"),
		format!("PODUP_EFI_BIG2={}\n", "x".repeat(1024 * 1024)),
	)
	.unwrap();
	let text: String = (0..4)
		.map(|i| format!("K{i}=${{PODUP_EFI_BIG2}}\n"))
		.collect();
	let m = load(dir.path(), &[("a.env", &text)]);
	assert_eq!(m["K3"].len(), 1024 * 1024);
}

#[test]
fn a_value_can_refer_to_the_environment_of_the_service() {
	std::env::remove_var("PODUP_EFS_URL");
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(
		dir.path().join("a.env"),
		"PODUP_EFS_END=${PODUP_EFS_URL}/v1\n",
	)
	.unwrap();
	let service_env: HashMap<String, Option<String>> = [(
		"PODUP_EFS_URL".to_string(),
		Some("https://api.example".to_string()),
	)]
	.into();
	let m = load_env_file_entries_for(
		&[EnvFileEntry::Path("a.env".into())],
		dir.path(),
		&service_env,
	)
	.unwrap();
	assert_eq!(m["PODUP_EFS_END"], "https://api.example/v1");
}

#[test]
fn the_project_wins_over_the_service_and_the_service_over_the_file() {
	std::env::set_var("PODUP_EFS_P", "project");
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(
		dir.path().join("a.env"),
		"PODUP_EFS_S=file\nPODUP_EFS_X=${PODUP_EFS_P}\nPODUP_EFS_Y=${PODUP_EFS_S}\n",
	)
	.unwrap();
	let service_env: HashMap<String, Option<String>> = [
		("PODUP_EFS_P".to_string(), Some("service".to_string())),
		("PODUP_EFS_S".to_string(), Some("service".to_string())),
	]
	.into();
	let m = load_env_file_entries_for(
		&[EnvFileEntry::Path("a.env".into())],
		dir.path(),
		&service_env,
	)
	.unwrap();
	assert_eq!(m["PODUP_EFS_X"], "project");
	assert_eq!(m["PODUP_EFS_Y"], "service");
}

#[test]
fn the_services_of_a_project_share_one_budget_for_their_env_files() {
	// Six services, each expanding a 3 MiB variable once: 3 MiB alone is fine,
	// 18 MiB together is past the 16 MiB a project may expand to.
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(
		dir.path().join(".env"),
		format!("PODUP_EFB_BIG={}\n", "x".repeat(3 * 1024 * 1024)),
	)
	.unwrap();
	std::fs::write(dir.path().join("a.env"), "K=${PODUP_EFB_BIG}\n").unwrap();
	let yaml = |n: usize| -> String {
		let services: String = (0..n)
			.map(|i| format!("  s{i}:\n    image: x\n    env_file: [a.env]\n"))
			.collect();
		format!("services:\n{services}")
	};
	let mut one = crate::compose::parse_str_raw(&yaml(1)).unwrap();
	materialize_env_files(&mut one, dir.path()).unwrap();
	let mut six = crate::compose::parse_str_raw(&yaml(6)).unwrap();
	assert!(
		materialize_env_files(&mut six, dir.path()).is_err(),
		"18 MiB of expansion across six services was accepted"
	);
}

#[test]
fn a_control_character_in_a_name_the_file_wrote_does_not_reach_an_error() {
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(
		dir.path().join("a.env"),
		"K=${PODUP_EFC_\u{1b}[31mX:?must be set}\nL=${PODUP_EFC_\u{1b}[31mY\n",
	)
	.unwrap();
	let err = load_env_file_entries(&[EnvFileEntry::Path("a.env".into())], dir.path())
		.expect_err("the first value is required and unset");
	assert!(!err.to_string().contains('\u{1b}'), "{:?}", err.to_string());
}
