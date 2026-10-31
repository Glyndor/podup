//! Entry point for the fuzz harness: the self-update parsers that read bytes
//! nobody controls (a release manifest, a GitHub response, a version tag),
//! forwarded unchanged and without behaviour of their own.

use super::github;
use super::verify;

pub use super::verify::Version;

/// `verify::parse_version`, unchanged.
pub fn parse_version(s: &str) -> crate::Result<Version> {
	verify::parse_version(s)
}

/// `verify::expected_digest`, unchanged.
pub fn expected_digest(sha256sums: &[u8], asset: &str) -> crate::Result<String> {
	verify::expected_digest(sha256sums, asset)
}

/// `github::parse_latest_tag`, unchanged.
pub fn parse_latest_tag(body: &[u8]) -> crate::Result<String> {
	github::parse_latest_tag(body)
}
