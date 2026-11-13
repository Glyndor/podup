//! The `sensitive_host_source` check: a host file or directory that reaches
//! the container or its build through a field other than `volumes:`.

use std::path::{Path, PathBuf};

use podup::compose::types::{BuildConfig, ComposeFile};

use super::super::super::check_sensitive_bind::{
	host_path_exposure, normalize_host_path, normalize_slashes, HostExposure,
};
use super::super::super::HostContext;
use super::host_paths::{home_dir, resolve};
use super::{finding, Finding};

/// Directories under the home that hold credentials, paired with what they
/// hold. Matched as prefixes of the resolved path, below the home directory
/// `~` expands to when the audit runs.
const HOME_CREDENTIALS: &[(&str, &str)] = &[
	(".ssh", "SSH keys"),
	(".aws", "AWS credentials"),
	(".gnupg", "GnuPG keys"),
	(".kube", "Kubernetes credentials"),
	(".docker", "container registry credentials"),
	(".config/gcloud", "Google Cloud credentials"),
];

/// Files under `/etc` named on their own because what they hold is worse
/// than the general system config the `/etc` entry describes.
const ETC_SECRETS: &[(&str, &str)] = &[
	("/etc/shadow", "password hashes of the host accounts"),
	("/etc/sudoers", "the host's sudo rules"),
	("/etc/sudoers.d", "the host's sudo rules"),
];

/// One host path a container or its build reads, outside `volumes:`.
struct HostSource {
	/// The field that names it, as the reason message prints it.
	origin: String,
	/// The path as the compose file wrote it.
	written: String,
	/// Where the engine reads it.
	resolved: PathBuf,
}

/// A host path the container or its build reads by another route than a
/// bind mount, which resolves to a sensitive location. The routes are a
/// `file:` secret or config the container uses (the top-level definition,
/// reached through the service's `secrets:`, `configs:` or `build.secrets:`),
/// an `env_file:`, a local `build.context`, its `build.dockerfile` and every
/// local `build.additional_contexts` entry. One finding per route.
///
/// Sensitive means the paths `sensitive_bind_mount` judges, plus the home
/// directory itself, the credential directories under it, and the password
/// and sudo files under `/etc`. A path under the home that holds no
/// credentials stays silent: projects live there.
///
/// Every path is resolved as the engine resolves a bind source, so `~` is
/// the home directory and a relative path is joined to the project
/// directory. The engine joins `env_file:` and build paths to the project
/// directory without expanding `~`; this check expands it anyway, because a
/// compose file that names the home there means the home, and Docker Compose
/// reads it so.
pub fn check_sensitive_host_source(name: &str, host: &HostContext<'_>) -> Vec<Finding> {
	let home = home_dir().map(|h| normalize_host_path(&h.to_string_lossy()));
	let mut out = Vec::new();
	for source in host_sources(host) {
		let resolved = source.resolved.to_string_lossy();
		// The path as written (slashes folded) first, then with `.` and `..`
		// folded: a symbolic link can make the two differ, and either one
		// landing on the list is a finding.
		let hit = [normalize_slashes(&resolved), normalize_host_path(&resolved)]
			.into_iter()
			.find_map(|path| exposure(&path, home.as_deref()).map(|what| (path, what)));
		let Some((path, what)) = hit else {
			continue;
		};
		let HostSource {
			origin, written, ..
		} = source;
		let reason = if written == path {
			format!("{origin} {written} is a sensitive host path ({what})")
		} else {
			format!("{origin} {written} resolves to {path}, a sensitive host path ({what})")
		};
		out.push(finding(name, "sensitive_host_source", &reason));
	}
	out
}

/// What a normalized absolute path exposes, or `None` when it is not
/// sensitive. `home` is the normalized home directory, when there is one.
fn exposure(path: &str, home: Option<&str>) -> Option<&'static str> {
	let under = |prefix: &str| path == prefix || path.starts_with(&format!("{prefix}/"));
	if let Some(home) = home.filter(|h| *h != "/") {
		if path == home {
			return Some("the whole home directory");
		}
		if let Some((_, what)) = HOME_CREDENTIALS
			.iter()
			.find(|(dir, _)| under(&format!("{home}/{dir}")))
		{
			return Some(what);
		}
	}
	if let Some((_, what)) = ETC_SECRETS.iter().find(|(file, _)| under(file)) {
		return Some(what);
	}
	match host_path_exposure(path)? {
		HostExposure::RuntimeSocket => Some("the container runtime socket"),
		HostExposure::Path(what) => Some(what),
	}
}

/// Every host path the written container reads outside `volumes:`.
fn host_sources(host: &HostContext<'_>) -> Vec<HostSource> {
	let service = host.written;
	let base = host.base_dir;
	let mut out = Vec::new();
	let mut push = |origin: String, written: &str, resolved: PathBuf| {
		out.push(HostSource {
			origin,
			written: written.to_string(),
			resolved,
		});
	};
	for secret in &service.secrets {
		if let Some(file) = secret_file(host.file, secret.source()) {
			push("a secret's file:".to_string(), file, resolve(file, base));
		}
	}
	for config in &service.configs {
		if let Some(file) = config_file(host.file, config.source()) {
			push("a config's file:".to_string(), file, resolve(file, base));
		}
	}
	for entry in service.env_file.to_entries() {
		push(
			"env_file:".to_string(),
			entry.path(),
			resolve(entry.path(), base),
		);
	}
	if let Some(build) = &service.build {
		build_sources(build, host.file, base, &mut push);
	}
	out
}

/// The host paths a `build:` reads: its build secrets, its local context,
/// the Dockerfile inside or beside that context, and its local additional
/// contexts. A remote context is fetched by the daemon, not read from the
/// host, and an inline Dockerfile is no host file.
fn build_sources(
	build: &BuildConfig,
	file: &ComposeFile,
	base: &Path,
	push: &mut impl FnMut(String, &str, PathBuf),
) {
	for secret in build.secrets() {
		if let Some(path) = secret_file(file, secret.source()) {
			let origin = "a build secret's file:".to_string();
			push(origin, path, resolve(path, base));
		}
	}
	let context = build.context();
	if !is_remote(context) {
		let context_dir = resolve(context, base);
		if let (None, Some(dockerfile)) = (build.dockerfile_inline(), build.dockerfile()) {
			// The engine reads the Dockerfile relative to the context.
			let path = resolve(dockerfile, &context_dir);
			push("build.dockerfile:".to_string(), dockerfile, path);
		}
		push("build.context:".to_string(), context, context_dir);
	}
	for (name, value) in build.additional_contexts() {
		if !is_remote(&value) && !value.starts_with("service:") {
			let origin = format!("build.additional_contexts: {name}:");
			push(origin, &value, resolve(&value, base));
		}
	}
}

/// A context the daemon fetches itself: a URL, an image or a git remote.
fn is_remote(value: &str) -> bool {
	value.contains("://") || value.starts_with("git@")
}

/// The `file:` of the top-level secret `name`, when it has one.
fn secret_file<'a>(file: &'a ComposeFile, name: &str) -> Option<&'a str> {
	let secret = file.secrets.get(name)?;
	if secret.external == Some(true) {
		return None;
	}
	secret.file.as_deref()
}

/// The `file:` of the top-level config `name`, when it has one.
fn config_file<'a>(file: &'a ComposeFile, name: &str) -> Option<&'a str> {
	let config = file.configs.get(name)?;
	if config.external == Some(true) {
		return None;
	}
	config.file.as_deref()
}
