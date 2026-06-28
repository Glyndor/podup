//! `logs` command: stream container output with `docker compose logs` options.

use futures_util::StreamExt;

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};
use crate::libpod::{urlencoded, LogOutput, API_PREFIX};

use super::log_prefix::LinePrefixer;
use super::Engine;

/// Options for [`Engine::logs_with_options`], mirroring `docker compose logs`.
#[derive(Default)]
pub struct LogsOptions {
	/// Follow log output, `-f/--follow`.
	pub follow: bool,
	/// Number of lines to show from the end, `-n/--tail` (`None` = all).
	pub tail: Option<String>,
	/// Show logs since a timestamp/relative time, `--since`.
	pub since: Option<String>,
	/// Show logs until a timestamp/relative time, `--until`.
	pub until: Option<String>,
	/// Prefix each line with an RFC3339 timestamp, `-t/--timestamps`.
	pub timestamps: bool,
}

/// Validate the `--tail`/`--since`/`--until` values client-side so a typo is
/// rejected with a clear local message instead of a raw podman HTTP 400. `tail`
/// must be `all` or a non-negative integer; `since`/`until` must be a Unix
/// timestamp or a Go-style duration (e.g. `10m`, `1h30m`) or an RFC3339-ish
/// timestamp. Pure so it is unit-tested.
fn validate_log_filters(opts: &LogsOptions) -> Result<()> {
	if let Some(tail) = &opts.tail {
		if tail != "all" && tail.parse::<u64>().is_err() {
			return Err(ComposeError::Unsupported(format!(
				"invalid --tail value {tail:?}: expected a non-negative integer or 'all'"
			)));
		}
	}
	for (flag, value) in [("--since", &opts.since), ("--until", &opts.until)] {
		if let Some(v) = value {
			if !is_valid_log_time(v) {
				return Err(ComposeError::Unsupported(format!(
					"invalid {flag} value {v:?}: expected a duration (e.g. 10m, 1h30m), a Unix \
					 timestamp, or an RFC3339 time"
				)));
			}
		}
	}
	Ok(())
}

/// Whether a `--since`/`--until` value is a plausible duration, Unix timestamp,
/// or timestamp string. Conservative: rejects obvious garbage (`abc`) while
/// accepting the forms podman understands.
fn is_valid_log_time(v: &str) -> bool {
	if v.is_empty() {
		return false;
	}
	// Unix timestamp (optionally fractional).
	if v.parse::<f64>().is_ok() {
		return true;
	}
	// Go-style duration: digit-run + unit, repeated (e.g. 1h30m, 90s, 500ms).
	if is_go_duration(v) {
		return true;
	}
	// Timestamp-ish: starts with a 4-digit year and contains only the characters
	// an RFC3339/date string uses. The server does the precise parse; this just
	// blocks free-form garbage.
	let bytes = v.as_bytes();
	bytes.len() >= 4
		&& bytes[..4].iter().all(u8::is_ascii_digit)
		&& v.chars().all(|c| {
			c.is_ascii_digit() || matches!(c, '-' | ':' | 't' | 'T' | 'z' | 'Z' | '.' | '+' | ' ')
		})
}

/// Match a Go-style duration: one or more `<number><unit>` segments, units one
/// of `ns,us,µs,ms,s,m,h`.
fn is_go_duration(v: &str) -> bool {
	let mut rest = v.strip_prefix('-').unwrap_or(v);
	if rest.is_empty() {
		return false;
	}
	let mut segments = 0;
	while !rest.is_empty() {
		let digits = rest.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.');
		if digits.len() == rest.len() {
			// No digits consumed → not a duration segment.
			return false;
		}
		rest = digits;
		let unit_len = ["ms", "ns", "us", "µs", "s", "m", "h"]
			.into_iter()
			.find(|u| rest.starts_with(u))
			.map(str::len);
		match unit_len {
			Some(n) => rest = &rest[n..],
			None => return false,
		}
		segments += 1;
	}
	segments > 0
}

/// Build the libpod `containers/{}/logs` query string from the options.
fn log_query(opts: &LogsOptions) -> String {
	let mut q = format!(
		"stdout=true&stderr=true&follow={}&timestamps={}",
		opts.follow, opts.timestamps
	);
	if let Some(tail) = &opts.tail {
		q.push_str(&format!("&tail={}", urlencoded(tail)));
	}
	if let Some(since) = &opts.since {
		q.push_str(&format!("&since={}", urlencoded(since)));
	}
	if let Some(until) = &opts.until {
		q.push_str(&format!("&until={}", urlencoded(until)));
	}
	q
}

impl Engine {
	/// Stream logs. When `service_name` is `None`, streams from all services. When `follow` is true, tails indefinitely.
	pub async fn logs(
		&self,
		file: &ComposeFile,
		service_name: Option<&str>,
		follow: bool,
	) -> Result<()> {
		let targets: Vec<String> = service_name
			.map(|s| vec![s.to_string()])
			.unwrap_or_default();
		self.logs_with_options(
			file,
			&targets,
			LogsOptions {
				follow,
				..Default::default()
			},
		)
		.await
	}

	/// Stream logs with `docker compose logs` options (`--tail`, `--since`,
	/// `--until`, `--timestamps`, `--follow`).
	///
	/// When `target_services` is empty, logs from every service are streamed;
	/// otherwise only the named services (an unknown name is an error).
	pub async fn logs_with_options(
		&self,
		file: &ComposeFile,
		target_services: &[String],
		opts: LogsOptions,
	) -> Result<()> {
		validate_log_filters(&opts)?;
		let follow = opts.follow;
		let query = log_query(&opts);
		for svc in target_services {
			if !file.services.contains_key(svc) {
				return Err(ComposeError::ServiceNotFound(svc.into()));
			}
		}
		let selected: std::collections::HashSet<&str> =
			target_services.iter().map(String::as_str).collect();
		// (container_name, is_tty) — TTY containers send raw bytes; non-TTY use
		// multiplexed 8-byte-header framing.
		let targets: Vec<(String, bool)> = file
			.services
			.iter()
			.filter(|(n, _)| selected.is_empty() || selected.contains(n.as_str()))
			.flat_map(|(n, s)| {
				let is_tty = s.tty.unwrap_or(false);
				self.replica_names(n, s)
					.into_iter()
					.map(move |cname| (cname, is_tty))
			})
			.collect();

		// When follow=true, streams never end until containers stop. Run them
		// concurrently so multiple containers don't block each other.
		if follow && targets.len() > 1 {
			let futs: Vec<_> = targets
				.into_iter()
				.map(|(container_name, is_tty)| {
					let client = &self.client;
					let query = query.clone();
					async move {
						let path = format!(
							"{API_PREFIX}/containers/{}/logs?{query}",
							urlencoded(&container_name),
						);
						let resp = match client.get_stream(&path).await {
							Ok(r) => r,
							Err(e) => {
								tracing::warn!("logs {container_name}: {e}");
								return;
							}
						};
						let mut stream = if is_tty {
							crate::libpod::parse_raw(resp.into_body())
						} else {
							crate::libpod::parse_multiplexed(resp.into_body())
						};
						// These futures run concurrently under `join_all` on the
						// same task, so the stdout/stderr lock is taken and
						// released within each frame rather than held across the
						// `.await` above — holding a guard across the await would
						// let a sibling future block the thread on the same lock
						// and deadlock. Each frame still locks once and flushes,
						// keeping interleaved `logs -f` output prompt.
						let mut out_pfx = LinePrefixer::new(&container_name);
						let mut err_pfx = LinePrefixer::new(&container_name);
						while let Some(msg) = stream.next().await {
							match msg {
								Ok(LogOutput::StdOut { message }) => {
									out_pfx.write(&mut std::io::stdout().lock(), &message);
								}
								Ok(LogOutput::StdErr { message }) => {
									err_pfx.write(&mut std::io::stderr().lock(), &message);
								}
								Err(_) => break,
							}
						}
						out_pfx.flush_tail(&mut std::io::stdout().lock());
						err_pfx.flush_tail(&mut std::io::stderr().lock());
					}
				})
				.collect();
			futures_util::future::join_all(futs).await;
		} else {
			for (container_name, is_tty) in targets {
				let path = format!(
					"{API_PREFIX}/containers/{}/logs?{query}",
					urlencoded(&container_name),
				);
				// Tolerate a missing/not-yet-created container the way the
				// multi-follow path does: warn and move on so the logs of the
				// services that *do* exist are still shown, instead of aborting the
				// whole command on the first 404.
				let resp = match self.client.get_stream(&path).await {
					Ok(r) => r,
					Err(e) => {
						tracing::warn!("logs {container_name}: {e}");
						continue;
					}
				};
				let mut stream = if is_tty {
					crate::libpod::parse_raw(resp.into_body())
				} else {
					crate::libpod::parse_multiplexed(resp.into_body())
				};

				// Lock stdout once for the whole stream instead of re-acquiring
				// the lock (and issuing a syscall) per frame; stdout is ours
				// exclusively on this path. stderr is locked per frame because
				// the tracing subscriber also writes there: holding its lock
				// across the await loop would starve concurrent log emissions.
				// Flush after each frame so `logs -f` still streams promptly.
				let mut out = std::io::stdout().lock();
				let mut out_pfx = LinePrefixer::new(&container_name);
				let mut err_pfx = LinePrefixer::new(&container_name);
				while let Some(msg) = stream.next().await {
					match msg {
						Ok(LogOutput::StdOut { message }) => out_pfx.write(&mut out, &message),
						Ok(LogOutput::StdErr { message }) => {
							err_pfx.write(&mut std::io::stderr().lock(), &message)
						}
						Err(_) => break,
					}
				}
				out_pfx.flush_tail(&mut out);
				err_pfx.flush_tail(&mut std::io::stderr().lock());
			}
		}

		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::{is_valid_log_time, log_query, validate_log_filters, LogsOptions};

	#[test]
	fn log_query_defaults_to_stdout_stderr_no_follow() {
		let q = log_query(&LogsOptions::default());
		assert_eq!(q, "stdout=true&stderr=true&follow=false&timestamps=false");
	}

	#[test]
	fn log_query_includes_set_options() {
		let q = log_query(&LogsOptions {
			follow: true,
			tail: Some("20".into()),
			since: Some("10m".into()),
			until: Some("2024-01-01T00:00:00".into()),
			timestamps: true,
		});
		assert!(q.contains("follow=true"));
		assert!(q.contains("timestamps=true"));
		assert!(q.contains("&tail=20"));
		assert!(q.contains("&since=10m"));
		// `:` is percent-encoded in the query value.
		assert!(q.contains("&until=2024-01-01T00%3A00%3A00"));
	}

	#[test]
	fn validate_log_filters_accepts_good_values() {
		assert!(validate_log_filters(&LogsOptions {
			tail: Some("all".into()),
			since: Some("10m".into()),
			until: Some("2024-01-01T00:00:00Z".into()),
			..Default::default()
		})
		.is_ok());
		assert!(validate_log_filters(&LogsOptions {
			tail: Some("100".into()),
			since: Some("1700000000".into()),
			..Default::default()
		})
		.is_ok());
		assert!(validate_log_filters(&LogsOptions::default()).is_ok());
	}

	#[test]
	fn validate_log_filters_rejects_bad_tail_and_time() {
		assert!(validate_log_filters(&LogsOptions {
			tail: Some("abc".into()),
			..Default::default()
		})
		.is_err());
		assert!(validate_log_filters(&LogsOptions {
			since: Some("yesterday".into()),
			..Default::default()
		})
		.is_err());
		assert!(validate_log_filters(&LogsOptions {
			until: Some("not-a-time".into()),
			..Default::default()
		})
		.is_err());
	}

	#[test]
	fn is_valid_log_time_classifies_forms() {
		assert!(is_valid_log_time("10m"));
		assert!(is_valid_log_time("1h30m"));
		assert!(is_valid_log_time("500ms"));
		assert!(is_valid_log_time("1700000000"));
		assert!(is_valid_log_time("2024-01-02T03:04:05Z"));
		assert!(!is_valid_log_time("abc"));
		assert!(!is_valid_log_time(""));
		assert!(!is_valid_log_time("10x"));
	}
}
