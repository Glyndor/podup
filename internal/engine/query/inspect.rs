//! Container inspection commands: top, port, images, and log attachment.

use futures_util::StreamExt;

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};
use crate::libpod::types::image::ImageInspect;
use crate::libpod::{urlencoded, LogOutput, API_PREFIX};

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
			// a service's process block is not rendered twice — matching docker
			// compose top.
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
					Err(e) => tracing::warn!("top {container_name}: {e}"),
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
		let container_name = self.replica_name_at(service_name, service, index)?;

		let path = format!(
			"{API_PREFIX}/containers/{}/json",
			urlencoded(&container_name),
		);
		let info = self
			.client
			.get_json::<crate::libpod::types::container::ContainerInspect>(&path)
			.await
			.map_err(ComposeError::Podman)?;

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
				// An image not present locally is listed with an empty ID rather
				// than silently dropped, so the output is never quietly incomplete.
				Err(e) => {
					tracing::warn!("images {name}: {e}");
					rows.push((name.clone(), repo, tag, String::new()));
				}
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
		let container = self.first_replica_name(service_name, service);
		let is_tty = service.tty.unwrap_or(false);

		let path = format!(
			"{API_PREFIX}/containers/{}/logs?stdout=true&stderr=true&follow=true",
			urlencoded(&container),
		);
		let resp = self
			.client
			.get_stream(&path)
			.await
			.map_err(ComposeError::Podman)?;
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

/// Resolve the `(port, proto)` for `port` from a `PORT` or `PORT/proto` argument,
/// the `/proto` suffix overriding the `--protocol` flag — matching
/// `docker compose port`. Pure so the parsing is unit-tested.
fn parse_port_proto<'a>(private_port: &'a str, proto_flag: &'a str) -> Result<(u16, &'a str)> {
	let (port, proto) = match private_port.split_once('/') {
		Some((p, pr)) => (p, pr),
		None => (private_port, proto_flag),
	};
	let port: u16 = port.parse().map_err(|_| {
		ComposeError::InvalidPort(format!(
			"port '{private_port}' is not a valid PORT or PORT/proto"
		))
	})?;
	Ok((port, proto))
}

/// Deduplicate a list of strings, preserving first-seen order.
fn dedup_preserving_order(items: &[String]) -> Vec<String> {
	let mut seen = std::collections::HashSet::new();
	items
		.iter()
		.filter(|s| seen.insert(s.as_str()))
		.cloned()
		.collect()
}

/// Split an image reference into `(repository, tag)` for the `images` table.
///
/// A trailing `:tag` is only a tag when the segment after it has no `/` (so a
/// `registry:port/name` host is not mis-split), mirroring the guard in
/// `export.rs`. A `name@sha256:...` digest reference has no tag, shown as
/// `<none>` like docker, and the long digest never bloats the TAG column.
fn split_repo_tag(image_ref: &str) -> (String, String) {
	if let Some((repo, _digest)) = image_ref.split_once('@') {
		return (repo.to_string(), "<none>".to_string());
	}
	match image_ref.rsplit_once(':') {
		Some((repo, tag)) if !tag.contains('/') => (repo.to_string(), tag.to_string()),
		_ => (image_ref.to_string(), "latest".to_string()),
	}
}

/// Align a `top` table (the title row followed by process rows) into
/// space-padded columns sized to the widest cell, returning one rendered line
/// per input row (titles first). All but the last column are left-padded; the
/// last is left ragged to avoid trailing whitespace.
fn align_top_columns(titles: &[String], processes: &[Vec<String>]) -> Vec<String> {
	let mut rows: Vec<&[String]> = Vec::with_capacity(processes.len() + 1);
	if !titles.is_empty() {
		rows.push(titles);
	}
	for p in processes {
		rows.push(p);
	}
	let col_count = rows.iter().map(|r| r.len()).max().unwrap_or(0);
	let mut widths = vec![0usize; col_count];
	for r in &rows {
		for (i, cell) in r.iter().enumerate() {
			widths[i] = widths[i].max(cell.chars().count());
		}
	}
	rows.iter()
		.map(|r| {
			r.iter()
				.enumerate()
				.map(|(i, cell)| {
					if i + 1 == r.len() {
						cell.clone()
					} else {
						format!("{cell:<width$}", width = widths[i])
					}
				})
				.collect::<Vec<_>>()
				.join("  ")
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use super::{align_top_columns, dedup_preserving_order, parse_port_proto, split_repo_tag};

	#[test]
	fn split_repo_tag_plain_name_and_tag() {
		assert_eq!(
			split_repo_tag("nginx:1.25"),
			("nginx".into(), "1.25".into())
		);
		assert_eq!(split_repo_tag("nginx"), ("nginx".into(), "latest".into()));
	}

	#[test]
	fn split_repo_tag_registry_with_port_is_not_a_tag() {
		// The ':' belongs to the registry host:port, not a tag.
		assert_eq!(
			split_repo_tag("registry:5000/team/app"),
			("registry:5000/team/app".into(), "latest".into())
		);
		assert_eq!(
			split_repo_tag("registry:5000/team/app:v2"),
			("registry:5000/team/app".into(), "v2".into())
		);
	}

	#[test]
	fn split_repo_tag_digest_has_no_tag() {
		let (repo, tag) = split_repo_tag(
			"docker.io/library/alpine@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
		);
		assert_eq!(repo, "docker.io/library/alpine");
		assert_eq!(tag, "<none>");
	}

	#[test]
	fn dedup_preserving_order_keeps_first_occurrence() {
		let out =
			dedup_preserving_order(&["web".into(), "db".into(), "web".into(), "cache".into()]);
		assert_eq!(out, vec!["web", "db", "cache"]);
	}

	#[test]
	fn align_top_columns_pads_to_widest_cell() {
		let titles = vec!["PID".to_string(), "CMD".to_string()];
		let processes = vec![
			vec!["1".to_string(), "bash".to_string()],
			vec!["12345".to_string(), "node".to_string()],
		];
		let lines = align_top_columns(&titles, &processes);
		assert_eq!(lines.len(), 3);
		// First column is padded to the widest value ("12345" = 5 chars).
		assert!(lines[0].starts_with("PID  "));
		assert!(lines[1].starts_with("1      "));
		// No tabs in the aligned output.
		assert!(lines.iter().all(|l| !l.contains('\t')));
	}

	#[test]
	fn bare_port_uses_flag_proto() {
		assert_eq!(parse_port_proto("80", "tcp").unwrap(), (80, "tcp"));
	}

	#[test]
	fn suffix_overrides_flag_proto() {
		assert_eq!(parse_port_proto("53/udp", "tcp").unwrap(), (53, "udp"));
	}

	#[test]
	fn non_numeric_port_is_rejected() {
		assert!(parse_port_proto("http", "tcp").is_err());
		assert!(parse_port_proto("abc/tcp", "tcp").is_err());
	}
}
