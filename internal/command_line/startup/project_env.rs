//! Settings the project's env files can carry for the tool itself.
//!
//! `COMPOSE_FILE`, `COMPOSE_PATH_SEPARATOR`, `COMPOSE_PROJECT_NAME`,
//! `COMPOSE_PROFILES` and `COMPOSE_ENV_FILES` are read from the process
//! environment by the argument parser, which never sees a `.env`. This fills
//! in what the command line and the process environment left unset, so the
//! order is: a flag, then the process environment, then the project's env
//! files (the `.env`, or the `--env-file` list that replaces it).
//!
//! `COMPOSE_ENV_FILES` is the one setting that cannot come from an env file,
//! since it names the files to read.

use std::path::{Path, PathBuf};

use super::resolve::{path_separator, resolve_base_dir};
use crate::cli::Cli;

/// A non-empty value of a process environment variable.
fn process_value(name: &str) -> Option<String> {
	std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// The values the project's env files set, or none when they cannot be read:
/// a missing or malformed `--env-file` is reported by the stage that parses
/// the compose file, with its own message.
fn layer(cli: &Cli, file: Option<&Path>) -> std::collections::HashMap<String, String> {
	let dir = resolve_base_dir(
		cli.project_directory.as_deref(),
		file.unwrap_or(Path::new("")),
	);
	podup::substitute::dotenv_layer(&dir, &cli.env_file).unwrap_or_default()
}

/// Fill the compose files, project name, profiles and env files that neither
/// the command line nor the process environment set.
pub(crate) fn apply(cli: &mut Cli) {
	if cli.env_file.is_empty() {
		if let Some(list) = process_value("COMPOSE_ENV_FILES") {
			cli.env_file = split(&list, ",").map(str::to_string).collect();
		}
	}

	let mut vars = layer(cli, cli.file.first().map(PathBuf::as_path));
	let get = |vars: &std::collections::HashMap<String, String>, name: &str| {
		process_value(name).or_else(|| vars.get(name).filter(|v| !v.is_empty()).cloned())
	};

	if cli.file.is_empty() {
		let from_files = vars.get("COMPOSE_FILE").is_some_and(|v| !v.is_empty());
		let own_sep = process_value("COMPOSE_PATH_SEPARATOR").is_none()
			&& vars.contains_key("COMPOSE_PATH_SEPARATOR");
		if from_files || own_sep {
			if let Some(list) = get(&vars, "COMPOSE_FILE") {
				let sep = path_separator(get(&vars, "COMPOSE_PATH_SEPARATOR").as_deref());
				cli.file = split(&list, &sep).map(PathBuf::from).collect();
				// The project directory is where the first file sits, so the env
				// files of that directory are the ones the rest of the settings
				// come from.
				vars = layer(cli, cli.file.first().map(PathBuf::as_path));
			}
		}
	}

	if cli.project.is_none() && std::env::var_os("COMPOSE_PROJECT_NAME").is_none() {
		cli.project = vars
			.get("COMPOSE_PROJECT_NAME")
			.filter(|v| !v.is_empty())
			.cloned();
	}
	if cli.profile.is_empty() && std::env::var_os("COMPOSE_PROFILES").is_none() {
		if let Some(list) = vars.get("COMPOSE_PROFILES") {
			cli.profile = split(list, ",").map(str::to_string).collect();
		}
	}
}

fn split<'a>(list: &'a str, sep: &'a str) -> impl Iterator<Item = &'a str> {
	list.split(sep).filter(|part| !part.is_empty())
}
