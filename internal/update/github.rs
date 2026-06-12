//! GitHub release source for self-update.
//!
//! Talks to GitHub over HTTPS only (rustls + webpki roots via `ureq`). TLS
//! guards transport, but the downloaded bytes are trusted only after the
//! Ed25519 signature check in [`super::verify`] — a compromised endpoint cannot
//! produce a binary that passes that check.

use std::io::Read;

use crate::ComposeError;

use super::ReleaseSource;

/// `owner/repo` slug of the canonical release repository.
pub const REPO: &str = "Glyndor/podup";

/// Hard cap on any single downloaded asset (defensive against a hostile or
/// broken endpoint streaming unbounded data into memory).
const MAX_ASSET_BYTES: u64 = 128 * 1024 * 1024;

/// Fetches release metadata and assets from GitHub.
pub struct GitHubSource {
	repo: String,
	agent: ureq::Agent,
}

impl GitHubSource {
	/// Source for the given `owner/repo`.
	pub fn new(repo: impl Into<String>) -> Self {
		let agent: ureq::Agent = ureq::Agent::config_builder()
			.timeout_global(Some(std::time::Duration::from_secs(60)))
			.user_agent(concat!("podup/", env!("CARGO_PKG_VERSION")))
			.build()
			.into();
		Self {
			repo: repo.into(),
			agent,
		}
	}
}

impl Default for GitHubSource {
	fn default() -> Self {
		Self::new(REPO)
	}
}

/// Read at most `MAX_ASSET_BYTES` from `reader`, erroring if the stream exceeds
/// the cap rather than truncating silently.
fn read_capped(mut reader: impl Read) -> crate::Result<Vec<u8>> {
	let mut buf = Vec::new();
	let read = reader
		.by_ref()
		.take(MAX_ASSET_BYTES + 1)
		.read_to_end(&mut buf)
		.map_err(ComposeError::Io)?;
	if read as u64 > MAX_ASSET_BYTES {
		return Err(ComposeError::Update(
			"release asset exceeds the maximum allowed size".to_string(),
		));
	}
	Ok(buf)
}

impl ReleaseSource for GitHubSource {
	fn latest_version(&self) -> crate::Result<String> {
		let url = format!("https://api.github.com/repos/{}/releases/latest", self.repo);
		let body = self
			.agent
			.get(&url)
			.header("Accept", "application/vnd.github+json")
			.call()
			.map_err(|e| ComposeError::Update(format!("cannot reach GitHub releases API: {e}")))?
			.body_mut()
			.read_to_string()
			.map_err(|e| ComposeError::Update(format!("failed reading release metadata: {e}")))?;

		#[derive(serde::Deserialize)]
		struct Latest {
			tag_name: String,
		}
		let latest: Latest = serde_json::from_str(&body)
			.map_err(|e| ComposeError::Update(format!("malformed release metadata: {e}")))?;
		Ok(latest.tag_name)
	}

	fn fetch(&self, asset: &str) -> crate::Result<Vec<u8>> {
		// Pinned to the latest release; `ureq` follows GitHub's redirect to the
		// asset host. Always HTTPS — the URL is a compile-time constant scheme.
		let url = format!(
			"https://github.com/{}/releases/latest/download/{asset}",
			self.repo
		);
		let resp = self
			.agent
			.get(&url)
			.call()
			.map_err(|e| ComposeError::Update(format!("download failed for {asset}: {e}")))?;
		read_capped(resp.into_body().into_reader())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn read_capped_accepts_small() {
		let data = b"hello world".to_vec();
		let got = read_capped(&data[..]).unwrap();
		assert_eq!(got, data);
	}

	#[test]
	fn read_capped_rejects_oversize() {
		struct Endless;
		impl Read for Endless {
			fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
				for b in buf.iter_mut() {
					*b = 0;
				}
				Ok(buf.len())
			}
		}
		assert!(read_capped(Endless).is_err());
	}

	#[test]
	fn default_uses_canonical_repo() {
		let src = GitHubSource::default();
		assert_eq!(src.repo, REPO);
	}
}
