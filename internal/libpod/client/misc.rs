use bytes::Bytes;
use hyper::StatusCode;

use super::{full, meets_minimum, Client, PathStat, Result, READ_TIMEOUT};

impl Client {
	/// `GET /libpod/_ping` returns Ok(()) when Podman is reachable and
	/// speaks a supported libpod API version.
	pub async fn ping(&self) -> Result<()> {
		let req = Self::build_request(
			hyper::Method::GET,
			"/libpod/_ping",
			full(Bytes::new()),
			None,
		)?;
		let resp = self.send(req, Some(READ_TIMEOUT)).await?;
		let reported = resp
			.headers()
			.get("Libpod-API-Version")
			.and_then(|v| v.to_str().ok())
			.unwrap_or_default()
			.to_owned();
		let (status, body) = resp.read_body(Some(READ_TIMEOUT)).await?;
		Self::check_status(status, &body)?;
		if !meets_minimum(&reported) {
			return Err(super::PodmanError::IncompatibleApiVersion { reported });
		}
		Ok(())
	}

	/// `HEAD` a container-archive path and decode its stat header.
	async fn head_container_path_stat(&self, path: &str) -> Result<Option<PathStat>> {
		use base64::Engine as _;

		let req = Self::build_request(hyper::Method::HEAD, path, full(Bytes::new()), None)?;
		let resp = self.send(req, Some(READ_TIMEOUT)).await?;
		// Drain the body on every status (including 404) so the pool guard
		// is released only after the response is fully consumed. Short-
		// circuiting before the read would let the connection return to
		// the idle queue while the daemon is still framing a body, and
		// the next acquirer would see interleaved bytes (#1740). For
		// `HEAD` the body is empty; for a successful 200 it carries the
		// stat the headers already described. Either way, reading it is
		// the connection-cleanup step.
		let stat_header = resp
			.headers()
			.get("X-Docker-Container-Path-Stat")
			.and_then(|v| v.to_str().ok())
			.map(str::to_string);
		let (status, body) = resp.read_body(Some(READ_TIMEOUT)).await?;
		if status == StatusCode::NOT_FOUND {
			return Ok(None);
		}
		Self::check_status(status, &body)?;
		let Some(stat) = stat_header else {
			return Ok(Some(PathStat::default()));
		};
		let json = base64::engine::general_purpose::STANDARD
			.decode(stat.as_bytes())
			.map_err(|e| super::PodmanError::Api {
				status: 0,
				message: format!("malformed container path stat: {e}"),
			})?;
		Ok(Some(
			serde_json::from_slice(&json).map_err(super::PodmanError::Json)?,
		))
	}

	/// `HEAD` a container-archive path, returning whether the path is a directory.
	pub async fn head_path_is_dir(&self, path: &str) -> Result<Option<bool>> {
		Ok(self
			.head_container_path_stat(path)
			.await?
			.map(|s| s.mode & (1 << 31) != 0))
	}

	/// The full decoded stat for a container path, or `None` when it does not exist.
	pub(crate) async fn head_path_stat(&self, path: &str) -> Result<Option<PathStat>> {
		self.head_container_path_stat(path).await
	}

	/// The full decoded stat for a container path, on a 200 or on a 404 that
	/// still carries the `X-Docker-Container-Path-Stat` header.
	///
	/// On Podman 5.7.0 a dangling symbolic link answers the stat HEAD with a
	/// 404 whose body carries the link's own `mode` and `size` in the stat
	/// header, and `head_path_stat` throws that stat away. The link is there;
	/// the 404 is what the runtime returns for "not a regular file or
	/// directory", not "absent". Reading the stat back off the 404 is what
	/// closes the gap: a regular file or directory that answers 404 returns
	/// `None` either way, so the dispatch does not matter for them; a link is
	/// the only kind that benefits.
	pub(crate) async fn head_path_stat_even_if_missing(
		&self,
		path: &str,
	) -> Result<Option<PathStat>> {
		use base64::Engine as _;

		let req = Self::build_request(hyper::Method::HEAD, path, full(Bytes::new()), None)?;
		let resp = self.send(req, Some(READ_TIMEOUT)).await?;
		// Drain the body on every status (including 404) so the pool guard
		// is released only after the response is fully consumed. Short-
		// circuiting before the read would let the connection return to
		// the idle queue while the daemon is still framing a body, and
		// the next acquirer would see interleaved bytes (#1740). For
		// `HEAD` the body is empty; for a successful 200 it carries the
		// stat the headers already described. Either way, reading it is
		// the connection-cleanup step.
		let stat_header = resp
			.headers()
			.get("X-Docker-Container-Path-Stat")
			.and_then(|v| v.to_str().ok())
			.map(str::to_string);
		let (status, body) = resp.read_body(Some(READ_TIMEOUT)).await?;
		if status == StatusCode::NOT_FOUND {
			// A 404 without the stat header means the path is truly absent;
			// the caller sees the same `None` `head_path_stat` returned for
			// the regular-file/directory case it already covered. A 404 WITH
			// the header (the Podman 5.7.0 dangling-link shape) gets the stat
			// back so the verification can read the link's mode off it.
			let Some(stat) = stat_header else {
				return Ok(None);
			};
			let json = base64::engine::general_purpose::STANDARD
				.decode(stat.as_bytes())
				.map_err(|e| super::PodmanError::Api {
					status: 0,
					message: format!("malformed container path stat: {e}"),
				})?;
			return Ok(Some(
				serde_json::from_slice(&json).map_err(super::PodmanError::Json)?,
			));
		}
		Self::check_status(status, &body)?;
		let Some(stat) = stat_header else {
			return Ok(Some(PathStat::default()));
		};
		let json = base64::engine::general_purpose::STANDARD
			.decode(stat.as_bytes())
			.map_err(|e| super::PodmanError::Api {
				status: 0,
				message: format!("malformed container path stat: {e}"),
			})?;
		Ok(Some(
			serde_json::from_slice(&json).map_err(super::PodmanError::Json)?,
		))
	}
}
