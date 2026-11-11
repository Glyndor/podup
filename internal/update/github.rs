//! GitHub release source for self-update.
//!
//! Talks to GitHub over HTTPS only (rustls + webpki roots via `ureq`). TLS
//! guards transport, but the downloaded bytes are trusted only after the
//! Ed25519 signature check in [`super::verify`]: a compromised endpoint cannot
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

/// Connection-establishment timeout per request.
const CONNECT_TIMEOUT_SECS: u64 = 30;

/// Whole-request timeout (headers + body).
const TOTAL_TIMEOUT_SECS: u64 = 300;

/// Transport failures are retried this many times in total, with exponential
/// backoff (1s, 2s) between attempts. HTTP 4xx responses are not retried;
/// they are deterministic, not transient.
const ATTEMPTS: u32 = 3;

/// Fetches release metadata and assets from GitHub.
pub struct GitHubSource {
	repo: String,
	agent: ureq::Agent,
	/// Base for the releases API (`https://api.github.com`). Overridable in
	/// tests so the transport-error path can be exercised offline.
	api_base: String,
	/// Base for asset downloads (`https://github.com`). Overridable in tests.
	dl_base: String,
	/// Cap on the release-metadata body: [`MAX_METADATA_BYTES`], lowered in tests
	/// so the refusal is reached without a megabyte of traffic.
	metadata_cap: u64,
	/// Cap on a downloaded asset: [`MAX_ASSET_BYTES`], lowered in tests.
	asset_cap: u64,
	/// Wait before the first retry; doubled before each next one. Zero in tests,
	/// which must not sleep.
	first_delay: std::time::Duration,
	/// The tag the metadata call resolved. Assets are downloaded from this
	/// tag and not from "latest", which moves when a release is published
	/// between the two calls and would pair a new manifest with an old tag.
	resolved_tag: std::sync::OnceLock<String>,
}

impl GitHubSource {
	/// Source for the given `owner/repo`.
	pub fn new(repo: impl Into<String>) -> Self {
		let agent: ureq::Agent = ureq::Agent::config_builder()
			.timeout_connect(Some(std::time::Duration::from_secs(CONNECT_TIMEOUT_SECS)))
			.timeout_global(Some(std::time::Duration::from_secs(TOTAL_TIMEOUT_SECS)))
			.user_agent(concat!("podup/", env!("CARGO_PKG_VERSION")))
			// Reject any non-HTTPS URL, including a redirect target: GitHub's
			// release download redirects to a CDN, and this prevents that hop (or a
			// hostile one) from being downgraded to plaintext http. The Ed25519
			// signature remains the authenticity gate; this hardens transport.
			.https_only(true)
			.build()
			.into();
		Self {
			repo: repo.into(),
			agent,
			api_base: "https://api.github.com".to_string(),
			dl_base: "https://github.com".to_string(),
			metadata_cap: MAX_METADATA_BYTES,
			asset_cap: MAX_ASSET_BYTES,
			first_delay: std::time::Duration::from_secs(1),
			resolved_tag: std::sync::OnceLock::new(),
		}
	}

	/// Source that talks plain http to `base` on the loopback, with small caps
	/// and no backoff. Tests only. The real source cannot be pointed at a local
	/// server because it refuses anything but HTTPS and none can speak TLS to a
	/// client that trusts only the public roots; this one lifts that refusal, so
	/// what it cannot show is the refusal itself, including the one for a
	/// redirect that leaves HTTPS.
	#[cfg(test)]
	fn loopback(base: &str, metadata_cap: u64, asset_cap: u64) -> Self {
		let mut s = Self::new(REPO);
		s.agent = ureq::Agent::config_builder()
			.timeout_global(Some(std::time::Duration::from_secs(20)))
			.https_only(false)
			.proxy(None)
			// A status code is an error in the real source, and the retry logic
			// reads it as one.
			.http_status_as_error(true)
			.build()
			.into();
		s.api_base = base.to_string();
		s.dl_base = base.to_string();
		s.metadata_cap = metadata_cap;
		s.asset_cap = asset_cap;
		s.first_delay = std::time::Duration::ZERO;
		s
	}

	/// Construct with overridden host bases: test seam for the transport-error
	/// path (point at a closed local port to force a connection failure).
	#[cfg(test)]
	fn with_bases(repo: impl Into<String>, api_base: &str, dl_base: &str) -> Self {
		let mut s = Self::new(repo);
		s.api_base = api_base.to_string();
		s.dl_base = dl_base.to_string();
		s
	}

	/// Construct with overridden host bases and a transport that ignores the
	/// process proxy environment. Tests only: refusing a TCP connect must
	/// come from the socket, not from a transparent proxy at `HTTPS_PROXY`
	/// that the default `Config::default()` would otherwise honour.
	#[cfg(test)]
	fn with_bases_no_proxy(repo: impl Into<String>, api_base: &str, dl_base: &str) -> Self {
		let mut s = Self::new(repo);
		s.agent = ureq::Agent::config_builder()
			.timeout_connect(Some(std::time::Duration::from_secs(CONNECT_TIMEOUT_SECS)))
			.timeout_global(Some(std::time::Duration::from_secs(TOTAL_TIMEOUT_SECS)))
			.user_agent(concat!("podup/", env!("CARGO_PKG_VERSION")))
			.https_only(true)
			.proxy(None)
			.build()
			.into();
		s.api_base = api_base.to_string();
		s.dl_base = dl_base.to_string();
		s
	}

	/// GET `url`, retrying transient transport failures with exponential
	/// backoff (up to [`ATTEMPTS`] tries). Deterministic HTTP 4xx responses are
	/// returned immediately.
	fn get_with_retry(
		&self,
		url: &str,
		accept: Option<&str>,
	) -> Result<ureq::http::Response<ureq::Body>, ureq::Error> {
		let mut delay = self.first_delay;
		let mut attempt = 1;
		loop {
			let mut req = self.agent.get(url);
			if let Some(a) = accept {
				req = req.header("Accept", a);
			}
			match req.call() {
				Ok(resp) => return Ok(resp),
				Err(e) => {
					let deterministic = matches!(e, ureq::Error::StatusCode(code) if code < 500);
					if deterministic || attempt >= ATTEMPTS {
						return Err(e);
					}
					attempt += 1;
					std::thread::sleep(delay);
					delay *= 2;
				}
			}
		}
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
pub(crate) fn parse_latest_tag(body: &[u8]) -> crate::Result<String> {
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
			.get_with_retry(&url, Some("application/vnd.github+json"))
			.map_err(|e| ComposeError::Update(format!("cannot reach GitHub releases API: {e}")))?;
		let body = read_capped(resp.into_body().into_reader(), self.metadata_cap)?;
		let tag = parse_latest_tag(&body)?;
		// Only a tag that parses as a version names a download path; anything
		// else is left for the caller to refuse and never reaches a URL.
		if crate::update::verify::parse_version(&tag).is_ok() {
			let _ = self.resolved_tag.set(tag.clone());
		}
		Ok(tag)
	}

	fn fetch(&self, asset: &str) -> crate::Result<Vec<u8>> {
		// Pinned to the tag the metadata call resolved, or to "latest" when none
		// was; `ureq` follows GitHub's redirect to the asset host. Always HTTPS;
		// the URL is a compile-time constant scheme.
		let release = match self.resolved_tag.get() {
			Some(tag) => format!("download/{tag}"),
			None => "latest/download".to_string(),
		};
		let url = format!("{}/{}/releases/{release}/{asset}", self.dl_base, self.repo);
		let resp = self
			.get_with_retry(&url, None)
			.map_err(|e| ComposeError::Update(format!("download failed for {asset}: {e}")))?;
		read_capped(resp.into_body().into_reader(), self.asset_cap)
	}
}

#[cfg(test)]
#[path = "github_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "github_hostile_tests.rs"]
mod hostile_tests;

#[cfg(test)]
#[path = "github_wire_tests.rs"]
mod wire_tests;
