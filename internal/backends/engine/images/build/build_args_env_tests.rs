//! A `build.args` entry without a value reads the project environment.
//!
//! `args: [FOO]` takes its value from the process environment and from the
//! project `.env`, the same environment a bare `environment: [FOO]` reads.
//! Only the process environment was read, so a `FOO` that lives in `.env` built
//! with an empty value.

use super::BuildOptions;
use crate::engine::Engine;

/// The `buildargs` query value libpod receives, decoded.
async fn build_args_sent(yaml: &str, dotenv: &str, opts: &BuildOptions) -> String {
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(dir.path().join("Dockerfile"), b"FROM alpine\n").unwrap();
	std::fs::write(dir.path().join(".env"), dotenv).unwrap();
	let file = crate::compose::parse_str(yaml).unwrap();
	let fake = crate::engine::fake_podman::start(|_method, target| {
		if target.contains("/build?") {
			(200, String::new())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = Engine::with_base_dir(fake.client(), "proj".into(), dir.path().to_path_buf());
	let _ = e
		.build_service("app", &file.services["app"], &file, opts)
		.await;
	let requests = fake.requests.lock().unwrap().clone();
	let request = requests
		.iter()
		.find(|r| r.contains("/build?"))
		.unwrap_or_else(|| panic!("the build endpoint was not called: {requests:?}"));
	let query = request.split("buildargs=").nth(1).unwrap_or("");
	let raw = query.split('&').next().unwrap_or("");
	percent_decode(raw)
}

fn percent_decode(raw: &str) -> String {
	let bytes = raw.as_bytes();
	let mut out = Vec::new();
	let mut i = 0;
	while i < bytes.len() {
		if bytes[i] == b'%' && i + 2 < bytes.len() {
			let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap();
			out.push(u8::from_str_radix(hex, 16).unwrap());
			i += 3;
		} else {
			out.push(bytes[i]);
			i += 1;
		}
	}
	String::from_utf8(out).unwrap()
}

/// A name no process defines, so the `.env` is the only place it can come from.
const NAME: &str = "PODUP_TEST_ONLY_IN_DOTENV_7F3A";
const YAML: &str = "services:\n  app:\n    build:\n      context: .\n      args:\n        - PODUP_TEST_ONLY_IN_DOTENV_7F3A\n";

#[tokio::test]
#[cfg(unix)]
async fn a_bare_arg_takes_its_value_from_the_project_dotenv() {
	let dotenv = format!("{NAME}=from_dotenv\n");
	let sent = build_args_sent(YAML, &dotenv, &BuildOptions::default()).await;
	assert!(
		sent.contains(&format!("\"{NAME}\":\"from_dotenv\"")),
		"{sent}"
	);
}

/// Its own name: this test sets a variable in the process, and a name the other
/// tests read would race with them.
const SHELL_NAME: &str = "PODUP_TEST_SET_IN_SHELL_2B9E";

#[test]
#[cfg(unix)]
fn the_process_environment_wins_over_the_dotenv() {
	let yaml = format!(
		"services:\n  app:\n    build:\n      context: .\n      args:\n        - {SHELL_NAME}\n"
	);
	temp_env::with_var(SHELL_NAME, Some("from_shell"), || {
		let runtime = tokio::runtime::Builder::new_current_thread()
			.enable_all()
			.build()
			.unwrap();
		let dotenv = format!("{SHELL_NAME}=from_dotenv\n");
		let sent = runtime.block_on(build_args_sent(&yaml, &dotenv, &BuildOptions::default()));
		assert!(
			sent.contains(&format!("\"{SHELL_NAME}\":\"from_shell\"")),
			"{sent}"
		);
	});
}

#[tokio::test]
#[cfg(unix)]
async fn a_bare_cli_build_arg_also_reads_the_dotenv() {
	let yaml = "services:\n  app:\n    build:\n      context: .\n";
	let opts = BuildOptions::new(false, false, vec![NAME.to_string()], false);
	let dotenv = format!("{NAME}=from_dotenv\n");
	let sent = build_args_sent(yaml, &dotenv, &opts).await;
	assert!(
		sent.contains(&format!("\"{NAME}\":\"from_dotenv\"")),
		"{sent}"
	);
}

#[tokio::test]
#[cfg(unix)]
async fn a_name_defined_nowhere_stays_empty() {
	let sent = build_args_sent(YAML, "OTHER=1\n", &BuildOptions::default()).await;
	assert!(sent.contains(&format!("\"{NAME}\":\"\"")), "{sent}");
}
