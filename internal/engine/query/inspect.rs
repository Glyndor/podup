//! Container inspection commands: top, port, images, and log attachment.

use futures_util::StreamExt;

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};
use crate::libpod::types::image::ImageInspect;
use crate::libpod::{urlencoded, LogOutput, API_PREFIX};

use super::inspect_util::{
	align_top_columns, dedup_preserving_order, is_running_status, parse_port_proto, select_replica,
	split_repo_tag,
};
use super::Engine;

impl Engine {
	/// Display running processes in each service container (`docker compose top`).
	///
	/// If `target_services` is empty, all services are queried.
	pub async fn top(&self, file: &ComposeFile, target_services: &[String]) -> Result<()> {
		self.top_with_options(file, target_services, false).await
	}

	/// `top` with `docker compose top`-style options: `--format json` emits a
	/// structured array of `{Container, Titles, Processes}` instead of the table.
	pub async fn top_with_options(
		&self,
		file: &ComposeFile,
		target_services: &[String],
		json: bool,
	) -> Result<()> {
		let names: Vec<String> = if target_services.is_empty() {
			file.services.keys().cloned().collect()
		} else {
			for name in target_services {
				if !file.services.contains_key(name) {
					return Err(crate::error::ComposeError::ServiceNotFound(name.clone()));
				}
			}
			// Deduplicate repeated positionals (`top web web`) preserving order, so
			// a service's process block is not rendered twice and we avoid redundant
			// `/top` API calls — matching docker compose top.
			dedup_preserving_order(target_services)
		};

		let mut json_rows: Vec<serde_json::Value> = Vec::new();
		for name in &names {
			let service = &file.services[name];
			for container_name in self.live_replica_names(name, service).await? {
				let path = format!(
					"{API_PREFIX}/containers/{}/top",
					urlencoded(&container_name),
				);
				match self
					.client
					.get_json::<crate::libpod::types::container::TopResponse>(&path)
					.await
				{
					Ok(result) if json => json_rows.push(serde_json::json!({
						"Container": container_name,
						"Titles": result.titles,
						"Processes": result.processes,
					})),
					Ok(result) => {
						crate::ui::print_bold_header(&container_name);
						// Space-pad columns to the widest cell (header + rows) rather
						// than tab-joining, so the table is aligned as the help promises.
						let titles = result.titles.clone().unwrap_or_default();
						let processes = result.processes.clone().unwrap_or_default();
						let aligned = align_top_columns(&titles, &processes);
						if let Some((header, rows)) = aligned.split_first() {
							crate::ui::print_bold_header(header);
							for row in rows {
								println!("{row}");
							}
						}
					}
					// A not-created container (404) is tolerated; any other failure
					// (e.g. a stopped container's HTTP 500, or an unreachable socket)
					// is a real error that must surface with a non-zero exit instead
					// of being swallowed into a warning.
					Err(e) if e.is_status(404) => {
						tracing::debug!("top {container_name}: {e}")
					}
					Err(e) => return Err(ComposeError::Podman(e)),
				}
			}
		}
		if json {
			println!(
				"{}",
				serde_json::to_string_pretty(&json_rows).unwrap_or_default()
			);
		}
		Ok(())
	}

	/// Print the public port for a given private port of a service container.
	///
	/// `proto` should be `"tcp"` or `"udp"`. Prints `HOST:PORT` to stdout.
	pub async fn port(
		&self,
		file: &ComposeFile,
		service_name: &str,
		private_port: &str,
		proto: &str,
	) -> Result<()> {
		self.port_with_index(file, service_name, private_port, proto, None)
			.await
	}

	/// Like [`Engine::port`] but targets a specific replica via `--index`
	/// (1-based); `None` uses the first replica.
	pub async fn port_with_index(
		&self,
		file: &ComposeFile,
		service_name: &str,
		private_port: &str,
		proto: &str,
		index: Option<u32>,
	) -> Result<()> {
		let (port, proto) = parse_port_proto(private_port, proto)?;

		let service = file
			.services
			.get(service_name)
			.ok_or_else(|| crate::error::ComposeError::ServiceNotFound(service_name.into()))?;
		// Resolve against the containers Podman actually has, not the static
		// compose replica count: a service scaled purely via CLI `--scale` has no
		// `scale:` in the file, so the static count is 1 and would target the
		// never-created un-indexed base name. `live_replica_names` falls back to
		// the static names only when nothing is running yet.
		let live = self.live_replica_names(service_name, service).await?;
		let container_name = select_replica(live, service_name, index)?;

		let path = format!(
			"{API_PREFIX}/containers/{}/json",
			urlencoded(&container_name),
		);
		let info = match self
			.client
			.get_json::<crate::libpod::types::container::ContainerInspect>(&path)
			.await
		{
			Ok(info) => info,
			// Translate a missing container into a friendly not-found rather than
			// surfacing a raw podman 404.
			Err(e) if e.is_status(404) => {
				return Err(crate::error::ComposeError::ServiceNotFound(format!(
					"{service_name} (no running container '{container_name}')"
				)));
			}
			Err(e) => return Err(ComposeError::Podman(e)),
		};

		let key = format!("{port}/{proto}");
		let binding = info
			.network_settings
			.and_then(|ns| ns.ports.get(&key).cloned().flatten())
			.and_then(|bindings| bindings.into_iter().next());

		match binding {
			Some(b) => {
				let host = b.host_ip.as_deref().unwrap_or("0.0.0.0");
				let port = b.host_port.as_deref().unwrap_or("");
				println!("{host}:{port}");
			}
			None => println!(),
		}
		Ok(())
	}

	/// List images used by each service as a table (default options).
	pub async fn images(&self, file: &ComposeFile) -> Result<()> {
		self.images_with_options(file, super::ImagesOptions::default())
			.await
	}

	/// List service images with `docker compose images`-style options:
	/// `-q/--quiet` (IDs only) and `--format` (table | json).
	pub async fn images_with_options(
		&self,
		file: &ComposeFile,
		opts: super::ImagesOptions,
	) -> Result<()> {
		// Collect rows first so quiet/json modes can render without the header.
		let mut rows: Vec<(String, String, String, String)> = Vec::new();
		for (name, service) in &file.services {
			let image_ref = match &service.image {
				Some(img) => img.clone(),
				None if service.build.is_some() => format!("{name}:latest"),
				None => continue,
			};
			let (repo, tag) = split_repo_tag(&image_ref);
			let path = format!("{API_PREFIX}/images/{}/json", urlencoded(&image_ref));
			match self.client.get_json::<ImageInspect>(&path).await {
				Ok(img) => {
					let id = img.id.trim_start_matches("sha256:").get(..12).unwrap_or("");
					rows.push((name.clone(), repo, tag, id.to_string()));
				}
				// A 404 means the image is simply not present locally — list it with
				// an empty ID rather than silently dropping it, matching docker
				// compose. Any other error (a connection failure / unreachable
				// socket, or an HTTP 500) is a real failure that must propagate with
				// a non-zero exit rather than printing an empty table and exiting 0.
				Err(e) if e.is_status(404) => {
					tracing::debug!("images {name}: not present ({e})");
					rows.push((name.clone(), repo, tag, String::new()));
				}
				Err(e) => return Err(ComposeError::Podman(e)),
			}
		}

		if opts.quiet {
			// Deduplicate IDs so services sharing an image emit it once, like
			// docker compose images -q. Empty IDs (not-pulled) are skipped.
			let mut seen = std::collections::HashSet::new();
			for (_, _, _, id) in &rows {
				if !id.is_empty() && seen.insert(id.as_str()) {
					println!("{id}");
				}
			}
			return Ok(());
		}
		if opts.json {
			let json: Vec<_> = rows
				.iter()
				.map(|(svc, repo, tag, id)| {
					serde_json::json!({
						"Service": svc, "Repository": repo, "Tag": tag, "ID": id,
					})
				})
				.collect();
			println!(
				"{}",
				serde_json::to_string_pretty(&json).unwrap_or_default()
			);
			return Ok(());
		}

		crate::ui::print_bold_header(&format!(
			"{:<30} {:<25} {:<15} {:<20}",
			"SERVICE", "REPOSITORY", "TAG", "IMAGE ID"
		));
		for (svc, repo, tag, id) in &rows {
			println!("{svc:<30} {repo:<25} {tag:<15} {id:<20}");
		}
		Ok(())
	}

	/// Attach to a single service container's output (`docker compose attach`).
	///
	/// Streams the first replica's stdout/stderr (follow) to this process's
	/// stdout/stderr with no prefix, until the container stops. podup never
	/// attaches STDIN (it allocates no TTY), so this is output-only.
	pub async fn attach(&self, file: &ComposeFile, service_name: &str) -> Result<()> {
		let service = file
			.services
			.get(service_name)
			.ok_or_else(|| ComposeError::ServiceNotFound(service_name.into()))?;
		// Resolve against the containers Podman actually has so a service scaled at
		// runtime (`up --scale=3` → `…-1`/`…-2`/`…-3`) attaches to a real replica
		// instead of the unsuffixed base name, which would 404. Pick the
		// lowest-numbered live container for a stable choice.
		let mut live = self
			.list_project_container_names(Some(service_name))
			.await?;
		live.sort();
		let container = live.into_iter().next().ok_or_else(|| {
			ComposeError::Unsupported(format!(
				"attach: no running container for service '{service_name}'"
			))
		})?;
		let is_tty = service.tty.unwrap_or(false);

		// `docker compose attach` errors when the target is not running. Without
		// this check the libpod logs endpoint replays the *entire* history of a
		// stopped container and then ends the stream, so `attach` would print the
		// whole log and exit 0. Inspect the state first and fail closed otherwise.
		let inspect_path = format!("{API_PREFIX}/containers/{}/json", urlencoded(&container));
		let info = self
			.client
			.get_json::<crate::libpod::types::container::ContainerInspect>(&inspect_path)
			.await
			.map_err(ComposeError::Podman)?;
		let status = info.state.and_then(|s| s.status).unwrap_or_default();
		if !is_running_status(&status) {
			let shown = if status.is_empty() {
				"unknown"
			} else {
				&status
			};
			return Err(ComposeError::Unsupported(format!(
				"cannot attach to {container}: container is not running (state: {shown})"
			)));
		}

		let path = format!(
			"{API_PREFIX}/containers/{}/logs?{}",
			urlencoded(&container),
			attach_log_query(),
		);
		// A service that exists in the compose file but has no created container
		// answers 404 here; surface a friendly "service X is not running" instead
		// of leaking a raw libpod HTTP 404, mirroring the ServiceNotFound a service
		// absent from compose gets.
		let resp = match self.client.get_stream(&path).await {
			Ok(r) => r,
			Err(e) if e.is_status(404) => {
				return Err(ComposeError::NotRunning(service_name.into()))
			}
			Err(e) => return Err(ComposeError::Podman(e)),
		};
		let mut stream = if is_tty {
			crate::libpod::parse_raw(resp.into_body())
		} else {
			crate::libpod::parse_multiplexed(resp.into_body())
		};
		while let Some(msg) = stream.next().await {
			match msg {
				Ok(LogOutput::StdOut { message }) => {
					print!("{}", String::from_utf8_lossy(&message));
				}
				Ok(LogOutput::StdErr { message }) => {
					eprint!("{}", String::from_utf8_lossy(&message));
				}
				Err(_) => break,
			}
		}
		Ok(())
	}

	/// Attach to log streams for all services with `attach: true` (the default). Streams are multiplexed to stdout with a service-name prefix.
	pub async fn attach_logs(&self, file: &ComposeFile) -> Result<()> {
		self.attach_logs_with_options(file, false).await
	}

	/// Like [`Engine::attach_logs`] but with `up --timestamps` support: when
	/// `timestamps` is set, each streamed line carries the libpod RFC3339
	/// timestamp prefix.
	pub async fn attach_logs_with_options(
		&self,
		file: &ComposeFile,
		timestamps: bool,
	) -> Result<()> {
		// Carry (display_name, container_name, is_tty) so the log parser matches
		// the container's framing mode: TTY containers emit raw bytes; non-TTY
		// containers emit multiplexed 8-byte-header frames.
		let attached: Vec<(String, String, bool)> = file
			.services
			.iter()
			.filter(|(_, s)| s.attach.unwrap_or(true))
			.flat_map(|(name, s)| {
				let proj_prefix = format!("{}-", self.project);
				let is_tty = s.tty.unwrap_or(false);
				self.replica_names(name, s).into_iter().map(move |cname| {
					let display = cname
						.strip_prefix(proj_prefix.as_str())
						.map(|s| s.to_string())
						.unwrap_or_else(|| cname.clone());
					(display, cname, is_tty)
				})
			})
			.collect();

		if attached.is_empty() {
			return Ok(());
		}

		let streams: Vec<_> = attached
			.iter()
			.map(|(display, cname, is_tty)| {
				let prefix = display.clone();
				let path = format!(
					"{API_PREFIX}/containers/{}/logs?stdout=true&stderr=true&follow=true&timestamps={timestamps}",
					urlencoded(cname),
				);
				let client = &self.client;
				let is_tty = *is_tty;
				async move {
					let resp = match client.get_stream(&path).await {
						Ok(r) => r,
						Err(e) => {
							tracing::warn!("attach_logs {prefix}: {e}");
							return;
						}
					};
					// TTY containers produce raw bytes (stdout/stderr merged).
					// Non-TTY containers produce multiplexed frames with 8-byte headers.
					let mut stream = if is_tty {
						crate::libpod::parse_raw(resp.into_body())
					} else {
						crate::libpod::parse_multiplexed(resp.into_body())
					};
					while let Some(msg) = stream.next().await {
						match msg {
							Ok(LogOutput::StdOut { message }) => {
								print!("{prefix} | {}", String::from_utf8_lossy(&message));
							}
							Ok(LogOutput::StdErr { message }) => {
								eprint!("{prefix} | {}", String::from_utf8_lossy(&message));
							}
							Err(_) => break,
						}
					}
				}
			})
			.collect();

		#[cfg(unix)]
		{
			use tokio::signal::unix::{signal, SignalKind};
			let mut sigterm = signal(SignalKind::terminate()).expect("SIGTERM handler");
			tokio::select! {
				_ = futures_util::future::join_all(streams) => {}
				_ = tokio::signal::ctrl_c() => {}
				_ = sigterm.recv() => {}
			}
		}
		#[cfg(not(unix))]
		tokio::select! {
			_ = futures_util::future::join_all(streams) => {}
			_ = tokio::signal::ctrl_c() => {}
		}

		Ok(())
	}
}

/// Query string for `attach`: a live-only stdout/stderr stream. `tail=0`
/// suppresses the historical log backlog so attach shows live output (matching
/// `docker compose attach`) instead of replaying the container's whole history.
fn attach_log_query() -> &'static str {
	"stdout=true&stderr=true&follow=true&tail=0"
}

#[cfg(test)]
mod tests {
	use super::attach_log_query;

	#[test]
	fn attach_query_suppresses_log_backlog() {
		// `tail=0` means attach streams live output only, not the full history.
		let q = attach_log_query();
		assert!(q.contains("follow=true"), "got: {q}");
		assert!(q.contains("tail=0"), "got: {q}");
	}
}
