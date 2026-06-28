//! Query and observation commands: ps, logs, exec, pull, remove_orphans.

use std::io::Write;

use futures_util::StreamExt;

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};
use crate::libpod::types::exec::{
	ExecCreateConfig, ExecCreateResponse, ExecInspect, ExecStartConfig,
};
use crate::libpod::{urlencoded, LogOutput, API_PREFIX};

use super::Engine;
use crate::libpod::types::container::{ContainerListEntry, ContainerPort};

mod inspect;
mod log_prefix;
mod logs;

pub use logs::LogsOptions;

/// Human-readable status for `ps`. Podman's libpod list endpoint leaves
/// `Status` empty and reports the machine state in `State`, so fall back to it
/// rather than rendering a blank column.
fn display_status(c: &ContainerListEntry) -> &str {
	if c.status.is_empty() {
		&c.state
	} else {
		&c.status
	}
}

/// Render a container's published ports the way `docker compose ps` does, e.g.
/// `0.0.0.0:8080->80/tcp`. An unset host IP means "all interfaces", shown as
/// `0.0.0.0` (libpod commonly omits it) to match Docker/Podman output.
fn format_ports(ports: &[ContainerPort]) -> String {
	ports
		.iter()
		.map(|p| {
			let proto = p
				.protocol
				.as_deref()
				.map(|proto| format!("/{proto}"))
				.unwrap_or_default();
			let host_ip = p
				.host_ip
				.as_deref()
				.filter(|s| !s.is_empty())
				.unwrap_or("0.0.0.0");
			format!(
				"{host_ip}:{}->{}{proto}",
				p.host_port.unwrap_or(0),
				p.container_port
			)
		})
		.collect::<Vec<_>>()
		.join(", ")
}

/// Options for [`Engine::exec`], mirroring `docker compose exec` flags.
#[derive(Default)]
pub struct ExecOptions {
	/// Extra environment variables (`KEY=VAL`), `-e/--env`.
	pub env: Vec<String>,
	/// Run as this user, `-u/--user`.
	pub user: Option<String>,
	/// Working directory inside the container, `-w/--workdir`.
	pub workdir: Option<String>,
	/// Run with extended privileges, `--privileged`.
	pub privileged: bool,
	/// Detach: start the exec and return without streaming output, `-d/--detach`.
	pub detach: bool,
	/// 1-based replica index for a scaled service, `--index` (default: first).
	pub index: Option<u32>,
}

/// Options for [`Engine::ps_with_options`].
#[derive(Default)]
pub struct PsOptions {
	/// Include stopped containers, `-a/--all` (default: running only).
	pub all: bool,
	/// Print only container IDs, `-q/--quiet`.
	pub quiet: bool,
	/// Emit JSON instead of the table, `--format json`.
	pub json: bool,
}

/// Options for [`Engine::images_with_options`].
#[derive(Default)]
pub struct ImagesOptions {
	/// Print only image IDs, `-q/--quiet`.
	pub quiet: bool,
	/// Emit JSON instead of the table, `--format json`.
	pub json: bool,
}

/// Map a libpod error from an `exec`/`attach` target into a friendly
/// [`ComposeError::NotRunning`] when it means the container is absent (404) or
/// stopped ("can only create exec sessions on running containers"), so the user
/// sees "service X is not running" instead of a raw HTTP 404/500. Any other
/// failure passes through unchanged. Pure so it is unit-tested.
fn map_not_running(e: crate::libpod::PodmanError, service_name: &str) -> ComposeError {
	let not_running = e.is_status(404)
		|| matches!(
			&e,
			crate::libpod::PodmanError::Api { message, .. }
				if {
					let m = message.to_ascii_lowercase();
					m.contains("can only create exec sessions on running containers")
						|| m.contains("is not running")
						|| m.contains("no such container")
				}
		);
	if not_running {
		ComposeError::NotRunning(service_name.to_string())
	} else {
		ComposeError::Podman(e)
	}
}

impl Engine {
	/// List running containers for this project as a table (default options).
	pub async fn ps(&self, file: &ComposeFile) -> Result<()> {
		self.ps_with_options(file, PsOptions::default()).await
	}

	/// List containers with `docker compose ps`-style options: `-a/--all`
	/// (include stopped), `-q/--quiet` (IDs only), and `--format` (table | json).
	pub async fn ps_with_options(&self, _file: &ComposeFile, opts: PsOptions) -> Result<()> {
		let label = format!("podup.project={}", self.project);
		let filters = serde_json::json!({ "label": [label] });
		let path = format!(
			"{API_PREFIX}/containers/json?all={}&filters={}",
			opts.all,
			urlencoded(&filters.to_string()),
		);

		let containers = self
			.client
			.get_json::<Vec<crate::libpod::types::container::ContainerListEntry>>(&path)
			.await
			.map_err(ComposeError::Podman)?;

		let name_of = |c: &crate::libpod::types::container::ContainerListEntry| {
			c.names.join(", ").trim_start_matches('/').to_string()
		};

		if opts.quiet {
			for c in &containers {
				let id = c.id.get(..12).unwrap_or(&c.id);
				println!("{id}");
			}
			return Ok(());
		}

		if opts.json {
			let rows: Vec<_> = containers
				.iter()
				.map(|c| {
					serde_json::json!({
						"Name": name_of(c),
						"Image": c.image,
						"Status": display_status(c),
						"ID": c.id,
					})
				})
				.collect();
			println!(
				"{}",
				serde_json::to_string_pretty(&rows).unwrap_or_default()
			);
			return Ok(());
		}

		crate::ui::print_bold_header(&format!(
			"{:<40} {:<30} {:<20} PORTS",
			"NAME", "IMAGE", "STATUS"
		));
		for c in &containers {
			let ports = format_ports(&c.ports);
			let status = crate::ui::status_cell(display_status(c), 20);
			println!("{:<40} {:<30} {status} {ports}", name_of(c), c.image);
		}

		Ok(())
	}

	/// Run a command in the first replica of the named service with default
	/// options. Exits with the command's exit code.
	pub async fn exec(
		&self,
		file: &ComposeFile,
		service_name: &str,
		cmd: Vec<String>,
	) -> Result<()> {
		self.exec_with_options(file, service_name, cmd, ExecOptions::default())
			.await
	}

	/// Run a command in a service container with `docker compose exec`-style
	/// overrides (env, user, workdir, privileged, detach, replica index).
	pub async fn exec_with_options(
		&self,
		file: &ComposeFile,
		service_name: &str,
		cmd: Vec<String>,
		opts: ExecOptions,
	) -> Result<()> {
		let service = file
			.services
			.get(service_name)
			.ok_or_else(|| ComposeError::ServiceNotFound(service_name.into()))?;
		if cmd.is_empty() {
			return Err(ComposeError::Unsupported(format!(
				"exec into service '{service_name}' requires a command to run"
			)));
		}
		let container_name = self.replica_name_at(service_name, service, opts.index)?;

		let exec_cfg = ExecCreateConfig {
			cmd: Some(cmd),
			attach_stdout: Some(true),
			attach_stderr: Some(true),
			user: opts.user.clone(),
			working_dir: opts.workdir.clone(),
			privileged: opts.privileged.then_some(true),
			env: (!opts.env.is_empty()).then(|| opts.env.clone()),
			..Default::default()
		};
		let create_path = format!(
			"{API_PREFIX}/containers/{}/exec",
			urlencoded(&container_name),
		);
		let resp: ExecCreateResponse = self
			.client
			.post_json(&create_path, &exec_cfg)
			.await
			.map_err(|e| map_not_running(e, service_name))?;
		let exec_id = resp.id;

		// `-d/--detach`: start the exec and return without streaming output or
		// waiting for the exit code. The server returns immediately, so the
		// response body is dropped.
		if opts.detach {
			let start_cfg = ExecStartConfig {
				detach: true,
				tty: false,
			};
			let start_path = format!("{API_PREFIX}/exec/{}/start", urlencoded(&exec_id));
			let _ = self
				.client
				.post_json_stream(&start_path, &start_cfg)
				.await
				.map_err(ComposeError::Podman)?;
			return Ok(());
		}

		let start_cfg = ExecStartConfig {
			detach: false,
			tty: false,
		};
		let start_path = format!("{API_PREFIX}/exec/{}/start", urlencoded(&exec_id));
		let start_resp = self
			.client
			.post_json_stream(&start_path, &start_cfg)
			.await
			.map_err(ComposeError::Podman)?;
		let mut stream = crate::libpod::parse_multiplexed(start_resp.into_body());

		// Lock stdout once for the whole stream instead of re-acquiring the lock
		// (and issuing a syscall) per frame; stdout is ours exclusively on this
		// path. stderr is locked per frame because the tracing subscriber also
		// writes there: holding its lock across the await loop would starve
		// concurrent log emissions. Flush after each frame so exec streams
		// promptly.
		{
			let mut out = std::io::stdout().lock();
			while let Some(msg) = stream.next().await {
				match msg.map_err(ComposeError::Podman)? {
					LogOutput::StdOut { message } => {
						let _ = out.write_all(String::from_utf8_lossy(&message).as_bytes());
						let _ = out.flush();
					}
					LogOutput::StdErr { message } => {
						let mut err = std::io::stderr().lock();
						let _ = err.write_all(String::from_utf8_lossy(&message).as_bytes());
						let _ = err.flush();
					}
				}
			}
		}

		let inspect_path = format!("{API_PREFIX}/exec/{}/json", urlencoded(&exec_id));
		let inspect: ExecInspect = self
			.client
			.get_json(&inspect_path)
			.await
			.map_err(ComposeError::Podman)?;
		if let Some(code) = inspect.exit_code {
			if code != 0 {
				return Err(ComposeError::RunExited(code));
			}
		}

		Ok(())
	}

	/// Names of this project's containers (by label) that the current compose file
	/// no longer defines — the orphans, shared by removal and the warning.
	async fn orphan_container_names(&self, file: &ComposeFile) -> Result<Vec<String>> {
		let label = format!("podup.project={}", self.project);
		let filters = serde_json::json!({ "label": [label] });
		let path = format!(
			"{API_PREFIX}/containers/json?all=true&filters={}",
			urlencoded(&filters.to_string()),
		);

		let running = self
			.client
			.get_json::<Vec<crate::libpod::types::container::ContainerListEntry>>(&path)
			.await
			.map_err(ComposeError::Podman)?;

		let known: std::collections::HashSet<String> = file
			.services
			.iter()
			.flat_map(|(n, s)| self.replica_names(n, s))
			.collect();

		let names: Vec<String> = running
			.iter()
			.flat_map(|c| c.names.iter())
			.map(|raw| raw.trim_start_matches('/').to_string())
			.collect();
		Ok(filter_orphans(names, &known))
	}

	/// Remove containers labelled for this project that are not defined in the current compose file.
	pub async fn remove_orphans(&self, file: &ComposeFile) -> Result<()> {
		for name in self.orphan_container_names(file).await? {
			tracing::info!("removing orphan container {name}");
			let rm_path = format!("{API_PREFIX}/containers/{}?force=true", urlencoded(&name));
			if let Err(e) = self.client.delete_ok(&rm_path).await {
				tracing::debug!("orphan delete {name}: {e}");
			}
		}
		Ok(())
	}

	/// Warn (without removing) when this project has orphan containers and
	/// `--remove-orphans` was not given, matching docker compose's `up`.
	pub async fn warn_orphans(&self, file: &ComposeFile) -> Result<()> {
		let orphans = self.orphan_container_names(file).await?;
		if !orphans.is_empty() {
			eprintln!(
				"Found orphan container(s) ({}) for this project. If you removed or renamed a \
				 service in your compose file, run with --remove-orphans to remove them.",
				orphans.join(", ")
			);
		}
		Ok(())
	}
}

/// The subset of `names` not present in `known` (the orphan containers). Pure so
/// the membership logic is unit-tested without a live Podman socket.
fn filter_orphans(names: Vec<String>, known: &std::collections::HashSet<String>) -> Vec<String> {
	names.into_iter().filter(|n| !known.contains(n)).collect()
}

#[cfg(test)]
mod tests {
	use super::{display_status, filter_orphans, format_ports, map_not_running};
	use crate::libpod::types::container::{ContainerListEntry, ContainerPort};
	use std::collections::{HashMap, HashSet};

	#[test]
	fn filter_orphans_keeps_only_unknown_names() {
		let known: HashSet<String> = ["web-1".to_string(), "db".to_string()].into();
		let names = vec![
			"web-1".to_string(),
			"db".to_string(),
			"old-cache".to_string(),
		];
		assert_eq!(filter_orphans(names, &known), vec!["old-cache".to_string()]);
	}

	#[test]
	fn filter_orphans_empty_when_all_known() {
		let known: HashSet<String> = ["web".to_string()].into();
		assert!(filter_orphans(vec!["web".to_string()], &known).is_empty());
	}

	fn entry(status: &str, state: &str) -> ContainerListEntry {
		ContainerListEntry {
			id: "abc123".into(),
			names: vec!["/web".into()],
			image: "alpine".into(),
			status: status.into(),
			state: state.into(),
			ports: vec![],
			labels: HashMap::new(),
		}
	}

	#[test]
	fn display_status_falls_back_to_state_when_status_empty() {
		// Podman 5's libpod list endpoint sends an empty `Status` and the real
		// machine state in `State` — `ps` must show the latter, not a blank.
		assert_eq!(display_status(&entry("", "running")), "running");
		assert_eq!(display_status(&entry("", "exited")), "exited");
	}

	#[test]
	fn display_status_prefers_status_when_present() {
		assert_eq!(
			display_status(&entry("Up 2 seconds", "running")),
			"Up 2 seconds"
		);
	}

	#[test]
	fn format_ports_defaults_missing_host_ip_to_all_interfaces() {
		let p = ContainerPort {
			host_ip: None,
			host_port: Some(8080),
			container_port: 80,
			protocol: Some("tcp".into()),
			..Default::default()
		};
		assert_eq!(
			format_ports(std::slice::from_ref(&p)),
			"0.0.0.0:8080->80/tcp"
		);
	}

	#[test]
	fn format_ports_keeps_explicit_host_ip() {
		let p = ContainerPort {
			host_ip: Some("127.0.0.1".into()),
			host_port: Some(5432),
			container_port: 5432,
			..Default::default()
		};
		assert_eq!(
			format_ports(std::slice::from_ref(&p)),
			"127.0.0.1:5432->5432"
		);
	}

	#[test]
	fn map_not_running_maps_404_and_stopped() {
		use crate::error::ComposeError;
		use crate::libpod::PodmanError;
		let e404 = PodmanError::Api {
			status: 404,
			message: "no such container: web".into(),
		};
		assert!(matches!(
			map_not_running(e404, "web"),
			ComposeError::NotRunning(s) if s == "web"
		));
		let e500 = PodmanError::Api {
			status: 500,
			message: "can only create exec sessions on running containers".into(),
		};
		assert!(matches!(
			map_not_running(e500, "web"),
			ComposeError::NotRunning(_)
		));
		// An unrelated error passes through unchanged.
		let other = PodmanError::Api {
			status: 500,
			message: "disk full".into(),
		};
		assert!(matches!(
			map_not_running(other, "web"),
			ComposeError::Podman(_)
		));
	}
}
