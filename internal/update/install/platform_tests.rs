use super::*;

#[test]
fn platform_asset_matches_known_targets() {
	// Whatever host runs the tests, the asset (if any) must be one of the
	// release matrix names.
	if let Some(asset) = platform_asset() {
		assert!(asset.starts_with("podup-"));
	}
}

#[test]
fn platform_asset_covers_every_release_target() {
	// Pins the OS/ARCH → asset mapping to the full `release.yml` build
	// matrix so a newly added prebuilt (or a dropped arm) is caught here
	// instead of failing self-update silently in the field.
	let expected = [
		(("linux", "x86_64"), "podup-linux-x86_64"),
		(("linux", "aarch64"), "podup-linux-arm64"),
		(("macos", "aarch64"), "podup-darwin-arm64"),
		(("macos", "x86_64"), "podup-darwin-x86_64"),
		(("windows", "x86_64"), "podup-windows-x86_64.exe"),
		(("windows", "aarch64"), "podup-windows-arm64.exe"),
	];
	for ((os, arch), asset) in expected {
		assert_eq!(
			asset_for(os, arch),
			Some(asset),
			"self-update mapping drifted for {os}/{arch}"
		);
	}
}

#[test]
fn require_platform_asset_is_consistent() {
	match (platform_asset(), require_platform_asset()) {
		(Some(a), Ok(b)) => assert_eq!(a, b),
		(None, Err(_)) => {}
		_ => panic!("platform_asset and require_platform_asset disagree"),
	}
}
