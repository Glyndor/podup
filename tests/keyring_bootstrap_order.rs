//! `docs/debian-packaging.md` must not teach a manual archive-key install.
//!
//! The page used to walk the reader through `dpkg-deb -x`, `gpg --show-keys`
//! and `sudo dpkg -i glyndor-archive-keyring.deb`, and an earlier version had
//! that order backwards, so `dpkg -i` ran the keyring package's maintainer
//! scripts as root before its key was checked. The manual path is gone now:
//! the archive bootstrap performs the fingerprint check and aborts on a
//! mismatch, and the apt repository documents the fingerprint and tests its
//! own bootstrap. A manual check is the step readers skip, so documenting it
//! here would only reintroduce a way to install without it.

use std::path::Path;

const DOC: &str = "docs/debian-packaging.md";

#[test]
fn the_packaging_guide_has_no_manual_keyring_install() {
	let p = Path::new(env!("CARGO_MANIFEST_DIR")).join(DOC);
	let text =
		std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()));
	for needle in [
		"dpkg -i glyndor-archive-keyring",
		"dpkg-deb -x glyndor-archive-keyring",
		"gpg --show-keys",
	] {
		assert!(
			!text.contains(needle),
			"{DOC} teaches a manual archive-key step ({needle:?}). Point the \
			 reader at the README's bootstrap instead, which checks the key and \
			 aborts on a mismatch."
		);
	}
}
