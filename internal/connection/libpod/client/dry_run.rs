//! `--dry-run` for the libpod client: a read goes through, anything else is
//! refused before it reaches the socket.
//!
//! The gate sits in the three places a request leaves the client (`send`,
//! `send_streaming` and `post_hijack`), not at the call sites: a new mutating
//! call added later is covered without anyone remembering to guard it, and a
//! request nobody thought about fails instead of changing something.

use hyper::Method;

use super::{Client, PodmanError, Result};

impl Client {
	/// Make every request that is not a read fail before it is sent.
	pub fn with_dry_run(mut self, dry_run: bool) -> Self {
		self.dry_run = dry_run;
		self
	}

	/// Whether this client refuses requests that would change something.
	pub fn is_dry_run(&self) -> bool {
		self.dry_run
	}

	/// `Ok` for a read, or for any request when the client is not in dry-run;
	/// otherwise the refusal that names what was not sent.
	pub(in crate::connection::libpod::client) fn refuse_in_dry_run(
		&self,
		method: &Method,
		path: &str,
	) -> Result<()> {
		if !self.dry_run || matches!(*method, Method::GET | Method::HEAD) {
			return Ok(());
		}
		Err(PodmanError::DryRun {
			method: method.to_string(),
			path: path.to_string(),
		})
	}
}

#[cfg(test)]
mod tests;
