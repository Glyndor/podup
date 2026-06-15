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

/// Hard cap on the release-metadata JSON response. A `releases/latest` payload
/// is a few KiB; 1 MiB is generous headroom while still bounding memory if a
/// hostile or broken endpoint streams an oversized body.
const MAX_METADATA_BYTES: u64 = 1024 * 1024;

/// Fetches release metadata and assets from GitHub.
pub struct GitHubSource {
	repo: String,
	agent: ureq::Agent,
	/// Base for the releases API (`https://api.github.com`). Overridable in
	/// tests so the transport-error path can be exercised offline.
	api_base: String,
	/// Base for asset downloads (`https://github.com`). Overridable in tests.
	dl_base: String,
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
			api_base: "https://api.github.com".to_string(),
			dl_base: "https://github.com".to_string(),
		}
	}

	/// Construct with overridden host bases — test seam for the transport-error
	/// path (point at a closed local port to force a connection failure).
	#[cfg(test)]
	fn with_bases(repo: impl Into<String>, api_base: &str, dl_base: &str) -> Self {
		let mut s = Self::new(repo);
		s.api_base = api_base.to_string();
		s.dl_base = dl_base.to_string();
		s
	}
}

impl Default for GitHubSource {
	fn default() -> Self {
		Self::new(REPO)
	}
}

/// Read at most `cap` bytes from `reader`, erroring if the stream exceeds the
/// cap rather than truncating silently.
fn read_capped(mut reader: impl Read, cap: u64) -> crate::Result<Vec<u8>> {
	let mut buf = Vec::new();
	let read = reader
		.by_ref()
		.take(cap + 1)
		.read_to_end(&mut buf)
		.map_err(ComposeError::Io)?;
	if read as u64 > cap {
		return Err(ComposeError::Update(
			"release data exceeds the maximum allowed size".to_string(),
		));
	}
	Ok(buf)
}

/// Parse the `tag_name` out of a GitHub "latest release" JSON body. Split from
/// the HTTP call so the malformed-metadata failure paths are unit-testable
/// without a network seam.
fn parse_latest_tag(body: &[u8]) -> crate::Result<String> {
	#[derive(serde::Deserialize)]
	struct Latest {
		tag_name: String,
	}
	let latest: Latest = serde_json::from_slice(body)
		.map_err(|e| ComposeError::Update(format!("malformed release metadata: {e}")))?;
	Ok(latest.tag_name)
}

impl ReleaseSource for GitHubSource {
	fn latest_version(&self) -> crate::Result<String> {
		let url = format!("{}/repos/{}/releases/latest", self.api_base, self.repo);
		let resp = self
			.agent
			.get(&url)
			.header("Accept", "application/vnd.github+json")
			.call()
			.map_err(|e| ComposeError::Update(format!("cannot reach GitHub releases API: {e}")))?;
		let body = read_capped(resp.into_body().into_reader(), MAX_METADATA_BYTES)?;
		parse_latest_tag(&body)
	}

	fn fetch(&self, asset: &str) -> crate::Result<Vec<u8>> {
		// Pinned to the latest release; `ureq` follows GitHub's redirect to the
		// asset host. Always HTTPS — the URL is a compile-time constant scheme.
		let url = format!(
			"{}/{}/releases/latest/download/{asset}",
			self.dl_base, self.repo
		);
		let resp = self
			.agent
			.get(&url)
			.call()
			.map_err(|e| ComposeError::Update(format!("download failed for {asset}: {e}")))?;
		read_capped(resp.into_body().into_reader(), MAX_ASSET_BYTES)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A reader that yields zero bytes forever — used to exercise the cap.
	struct Endless;
	impl Read for Endless {
		fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
			for b in buf.iter_mut() {
				*b = 0;
			}
			Ok(buf.len())
		}
	}

	#[test]
	fn read_capped_accepts_small() {
		let data = b"hello world".to_vec();
		let got = read_capped(&data[..], MAX_ASSET_BYTES).unwrap();
		assert_eq!(got, data);
	}

	#[test]
	fn read_capped_rejects_oversize() {
		assert!(read_capped(Endless, MAX_ASSET_BYTES).is_err());
	}

	#[test]
	fn read_capped_enforces_metadata_cap() {
		// The metadata cap is far smaller than the asset cap; an endless stream
		// must be rejected once it crosses the 1 MiB metadata bound.
		assert!(read_capped(Endless, MAX_METADATA_BYTES).is_err());
	}

	#[test]
	fn read_capped_accepts_up_to_cap() {
		// Exactly `cap` bytes is allowed; cap+1 is rejected.
		let exactly = [0u8; 8];
		assert!(read_capped(&exactly[..], 8).is_ok());
		let over = [0u8; 9];
		assert!(read_capped(&over[..], 8).is_err());
	}

	#[test]
	fn default_uses_canonical_repo() {
		let src = GitHubSource::default();
		assert_eq!(src.repo, REPO);
	}

	#[test]
	fn parse_latest_tag_extracts_tag() {
		let tag = parse_latest_tag(br#"{"tag_name":"v1.2.3","name":"r"}"#).unwrap();
		assert_eq!(tag, "v1.2.3");
	}

	#[test]
	fn parse_latest_tag_rejects_malformed_json() {
		assert!(parse_latest_tag(b"not json at all").is_err());
		assert!(parse_latest_tag(b"").is_err());
	}

	#[test]
	fn parse_latest_tag_rejects_missing_field() {
		// Well-formed JSON object without `tag_name` must fail, not default.
		let err = parse_latest_tag(br#"{"name":"release"}"#).unwrap_err();
		assert!(err.to_string().contains("malformed release metadata"));
	}

	#[test]
	fn latest_version_maps_transport_error() {
		// Port 1 is closed → connection refused, offline and deterministic. The
		// transport failure must map to the friendly "cannot reach" error.
		use crate::update::ReleaseSource;
		let src = GitHubSource::with_bases(REPO, "http://127.0.0.1:1", "http://127.0.0.1:1");
		let err = src.latest_version().unwrap_err();
		assert!(
			err.to_string().contains("cannot reach GitHub releases API"),
			"got: {err}"
		);
	}

	#[test]
	fn fetch_maps_transport_error() {
		use crate::update::ReleaseSource;
		let src = GitHubSource::with_bases(REPO, "http://127.0.0.1:1", "http://127.0.0.1:1");
		let err = src.fetch("podup-linux-x86_64").unwrap_err();
		assert!(err.to_string().contains("download failed"), "got: {err}");
	}
}
