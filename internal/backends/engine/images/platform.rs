//! The `platform:` of a service, as the parts the libpod endpoints take.
//!
//! Compose writes a platform as `os/arch[/variant]`, and the pull endpoint
//! reads it as three parameters of its own (`os`, `arch`, `variant`): a
//! `platform` parameter there is ignored, so the image of the host's
//! architecture comes down whatever the service asks for. A bare
//! architecture (`arm64`) is accepted too, with no OS.

/// The parts of a platform string. A part the string leaves out is `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Platform {
	pub(crate) os: Option<String>,
	pub(crate) arch: Option<String>,
	pub(crate) variant: Option<String>,
}

impl Platform {
	/// Split `os/arch[/variant]`, or a bare architecture.
	pub(crate) fn parse(platform: &str) -> Self {
		let part = |value: Option<&str>| {
			value
				.map(str::trim)
				.filter(|v| !v.is_empty())
				.map(str::to_string)
		};
		let mut parts = platform.trim().split('/');
		let first = parts.next();
		match parts.next() {
			Some(arch) => Self {
				os: part(first),
				arch: part(Some(arch)),
				variant: part(parts.next()),
			},
			None => Self {
				os: None,
				arch: part(first),
				variant: None,
			},
		}
	}

	/// The query parameters of the pull endpoint that select this platform.
	pub(crate) fn pull_query(&self) -> String {
		let mut out = String::new();
		for (key, value) in [
			("os", &self.os),
			("arch", &self.arch),
			("variant", &self.variant),
		] {
			if let Some(value) = value {
				out.push_str(&format!("&{key}={}", crate::libpod::urlencoded(value)));
			}
		}
		out
	}
}

#[cfg(test)]
#[path = "platform_tests.rs"]
mod tests;
