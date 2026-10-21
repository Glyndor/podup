use crate::ComposeError;

/// The release asset name for the platform this binary was built for. Mirrors
/// the `release.yml` build matrix exactly.
pub fn platform_asset() -> Option<&'static str> {
	super::asset_for(std::env::consts::OS, std::env::consts::ARCH)
}

/// Map an OS/ARCH pair to its release asset name. Split out from
/// [`platform_asset`] so the full matrix is testable without the host's values.
pub(crate) fn asset_for(os: &str, arch: &str) -> Option<&'static str> {
	match (os, arch) {
		("linux", "x86_64") => Some("podup-linux-x86_64"),
		("linux", "aarch64") => Some("podup-linux-arm64"),
		("macos", "aarch64") => Some("podup-darwin-arm64"),
		("macos", "x86_64") => Some("podup-darwin-x86_64"),
		("windows", "x86_64") => Some("podup-windows-x86_64.exe"),
		("windows", "aarch64") => Some("podup-windows-arm64.exe"),
		_ => None,
	}
}

/// Resolve the asset for the current platform or fail with a clear message.
pub fn require_platform_asset() -> crate::Result<&'static str> {
	super::platform_asset().ok_or_else(|| {
		ComposeError::Update(format!(
			"self-update is not supported on {}/{}; reinstall manually from \
			 https://github.com/Glyndor/podup/releases",
			std::env::consts::OS,
			std::env::consts::ARCH
		))
	})
}
