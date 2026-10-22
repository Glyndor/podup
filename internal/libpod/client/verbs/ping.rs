use bytes::Bytes;

use super::{full, meets_minimum, Client, Result, READ_TIMEOUT};

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
}
