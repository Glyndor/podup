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
			target_services.to_vec()
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
						if let Some(titles) = &result.titles {
							crate::ui::print_bold_header(&titles.join("\t"));
						}
						if let Some(processes) = &result.processes {
							for row in processes {
								println!("{}", row.join("\t"));
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
		self.images_with_options(file, &[], super::ImagesOptions::default())
			.await
	}

	/// List service images with `docker compose images`-style options:
	/// `-q/--quiet` (IDs only) and `--format` (table | json). When
	/// `target_services` is non-empty, only those services are listed (an unknown
	/// name is an error), matching `docker compose images [SERVICE...]`.
	pub async fn images_with_options(
		&self,
		file: &ComposeFile,
		target_services: &[String],
		opts: super::ImagesOptions,
	) -> Result<()> {
		for name in target_services {
			if !file.services.contains_key(name) {
				return Err(ComposeError::ServiceNotFound(name.clone()));
			}
		}
		// Collect rows first so quiet/json modes can render without the header.
		let mut rows: Vec<(String, String, String, String)> = Vec::new();
		for (name, service) in &file.services {
			if !target_services.is_empty() && !target_services.iter().any(|t| t == name) {
				continue;
			}
			let image_ref = match &service.image {
				Some(img) => img.clone(),
				None if service.build.is_some() => format!("{name}:latest"),
				None => continue,
			};
			let path = format!("{API_PREFIX}/images/{}/json", urlencoded(&image_ref));
			match self.client.get_json::<ImageInspect>(&path).await {
				Ok(img) => {
					let (repo, tag) = image_ref
						.rsplit_once(':')
						.map(|(r, t)| (r.to_string(), t.to_string()))
						.unwrap_or_else(|| (image_ref.clone(), "latest".to_string()));
					let id = img.id.trim_start_matches("sha256:").get(..12).unwrap_or("");
					rows.push((name.clone(), repo, tag, id.to_string()));
				}
				Err(e) => tracing::warn!("images {name}: {e}"),
			}
		}

		if opts.quiet {
			for (_, _, _, id) in &rows {
				println!("{id}");
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
		self.attach_with_index(file, service_name, None).await
	}

	/// Like [`Engine::attach`] but targets a specific replica via `--index`
	/// (1-based); `None` uses the first replica. This is what lets `attach` reach
	/// a scaled service's later replicas.
	pub async fn attach_with_index(
		&self,
		file: &ComposeFile,
		service_name: &str,
		index: Option<u32>,
	) -> Result<()> {
		let service = file
			.services
			.get(service_name)
			.ok_or_else(|| ComposeError::ServiceNotFound(service_name.into()))?;
		let container = self.replica_name_at(service_name, service, index)?;
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
/// `docker compose port`. The port must be a canonical decimal (no leading `+`,
/// sign, or leading zeros), at most one `/` is allowed, and the protocol is
/// normalised to lowercase and restricted to `tcp`/`udp` — so a typo errors
/// instead of silently printing nothing. Pure so the parsing is unit-tested.
fn parse_port_proto(private_port: &str, proto_flag: &str) -> Result<(u16, String)> {
	let invalid = || {
		ComposeError::InvalidPort(format!(
			"port '{private_port}' is not a valid PORT or PORT/proto"
		))
	};

	let mut parts = private_port.split('/');
	let port_str = parts.next().unwrap_or_default();
	let proto = parts.next().unwrap_or(proto_flag);
	// `PORT/proto/extra` has more than one segment and is rejected outright.
	if parts.next().is_some() {
		return Err(invalid());
	}

	// `u16::from_str` accepts `+80` and `080`; reject those non-canonical
	// spellings so they no longer fall through to an empty lookup.
	if port_str.is_empty()
		|| !port_str.bytes().all(|b| b.is_ascii_digit())
		|| (port_str.len() > 1 && port_str.starts_with('0'))
	{
		return Err(invalid());
	}
	let port: u16 = port_str.parse().map_err(|_| invalid())?;

	let proto = proto.to_ascii_lowercase();
	if proto != "tcp" && proto != "udp" {
		return Err(ComposeError::InvalidPort(format!(
			"protocol '{proto}' is invalid for port '{private_port}' (expected tcp or udp)"
		)));
	}
	Ok((port, proto))
}

#[cfg(test)]
mod tests {
	use super::parse_port_proto;

	#[test]
	fn bare_port_uses_flag_proto() {
		assert_eq!(
			parse_port_proto("80", "tcp").unwrap(),
			(80, "tcp".to_string())
		);
	}

	#[test]
	fn suffix_overrides_flag_proto() {
		assert_eq!(
			parse_port_proto("53/udp", "tcp").unwrap(),
			(53, "udp".to_string())
		);
	}

	#[test]
	fn proto_is_normalised_to_lowercase() {
		assert_eq!(
			parse_port_proto("80", "TCP").unwrap(),
			(80, "tcp".to_string())
		);
		assert_eq!(
			parse_port_proto("53/UDP", "tcp").unwrap(),
			(53, "udp".to_string())
		);
	}

	#[test]
	fn non_numeric_port_is_rejected() {
		assert!(parse_port_proto("http", "tcp").is_err());
		assert!(parse_port_proto("abc/tcp", "tcp").is_err());
	}

	#[test]
	fn non_canonical_port_is_rejected() {
		// A leading '+', a sign, or leading zeros all parse via u16 but diverge
		// from docker's port spec.
		for bad in ["+80", "080", "0080", "-1", " 80"] {
			assert!(parse_port_proto(bad, "tcp").is_err(), "`{bad}` should fail");
		}
	}

	#[test]
	fn extra_slash_segment_is_rejected() {
		assert!(parse_port_proto("80/tcp/extra", "tcp").is_err());
	}

	#[test]
	fn invalid_proto_is_rejected() {
		assert!(parse_port_proto("80", "sctp").is_err());
		assert!(parse_port_proto("80/bogus", "tcp").is_err());
		assert!(parse_port_proto("80", "").is_err());
	}
}
