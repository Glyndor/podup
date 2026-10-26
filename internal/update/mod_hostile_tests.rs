//! The update flow against a release source that lies, and what it asks for
//! before it gives up.
//!
//! The order is the defence: the signed manifest is fetched and verified
//! before the binary is, so a forged release is refused without buffering a
//! payload the attacker chose. Each case records every asset requested.

use super::*;
use std::cell::RefCell;

/// A source that answers `latest_version` with `tag` and `fetch` from `serve`,
/// recording the name of every asset asked for.
struct Hostile<F: Fn(&str) -> crate::Result<Vec<u8>>> {
	tag: String,
	serve: F,
	asked: RefCell<Vec<String>>,
}

impl<F: Fn(&str) -> crate::Result<Vec<u8>>> ReleaseSource for Hostile<F> {
	fn latest_version(&self) -> crate::Result<String> {
		Ok(self.tag.clone())
	}
	fn fetch(&self, asset: &str) -> crate::Result<Vec<u8>> {
		self.asked.borrow_mut().push(asset.to_string());
		(self.serve)(asset)
	}
}

fn source<F: Fn(&str) -> crate::Result<Vec<u8>>>(tag: &str, serve: F) -> Hostile<F> {
	Hostile {
		tag: tag.to_string(),
		serve,
		asked: RefCell::new(Vec::new()),
	}
}

fn update(src: &dyn ReleaseSource, current: &str, force: bool) -> crate::Result<()> {
	let opts = UpdateOptions {
		check_only: false,
		force,
	};
	run_with_guard(src, current, opts, None)
}

#[test]
fn a_signature_of_the_wrong_length_is_refused_before_the_binary_is_requested() {
	if install::platform_asset().is_none() {
		return;
	}
	for len in [0usize, 1, 63, 65, 4096] {
		let src = source("v9.9.9", move |asset| match asset {
			"SHA256SUMS" => Ok(b"a  podup\n".to_vec()),
			"SHA256SUMS.sig" => Ok(vec![0u8; len]),
			other => panic!("{other} must not be requested after a bad signature"),
		});
		let err = update(&src, "0.6.0", false).unwrap_err();
		assert!(
			err.to_string().contains("malformed signature"),
			"{len}: {err}"
		);
		assert_eq!(*src.asked.borrow(), ["SHA256SUMS", "SHA256SUMS.sig"]);
	}
}

#[test]
fn a_well_formed_signature_that_is_nobodys_is_refused_before_the_binary_is_requested() {
	if install::platform_asset().is_none() {
		return;
	}
	let src = source("v9.9.9", |asset| match asset {
		"SHA256SUMS" => Ok(b"a  podup\n".to_vec()),
		"SHA256SUMS.sig" => Ok(vec![0u8; 64]),
		other => panic!("{other} must not be requested after a bad signature"),
	});
	let err = update(&src, "0.6.0", false).unwrap_err();
	assert!(err.to_string().contains("verification failed"), "{err}");
	assert_eq!(src.asked.borrow().len(), 2);
}

#[test]
fn a_manifest_download_that_fails_stops_the_update_there() {
	if install::platform_asset().is_none() {
		return;
	}
	let src = source("v9.9.9", |_| {
		Err(ComposeError::Update("connection reset".into()))
	});
	let err = update(&src, "0.6.0", false).unwrap_err();
	assert!(err.to_string().contains("connection reset"), "{err}");
	assert_eq!(*src.asked.borrow(), ["SHA256SUMS"]);
}

#[test]
fn a_release_older_than_the_installed_one_is_not_fetched() {
	let src = source("v0.1.0", |asset| panic!("{asset} must not be requested"));
	update(&src, "9.0.0", false).unwrap();
	assert!(src.asked.borrow().is_empty());
}

#[test]
fn an_older_release_under_force_is_still_signature_gated() {
	if install::platform_asset().is_none() {
		return;
	}
	let src = source("v0.1.0", |asset| match asset {
		"SHA256SUMS" => Ok(b"a  podup\n".to_vec()),
		"SHA256SUMS.sig" => Ok(vec![0u8; 64]),
		other => panic!("{other} must not be requested after a bad signature"),
	});
	let err = update(&src, "9.0.0", true).unwrap_err();
	assert!(err.to_string().contains("verification failed"), "{err}");
}

#[test]
fn tags_that_are_not_versions_stop_the_update_before_any_download() {
	for tag in [
		"",
		"latest",
		"v9.9.9-rc1",
		"v+9.+9.+9",
		"v9.9.9\n\u{1b}]0;pwned\u{7}",
		"v99999999999999999999999.0.0",
		"../../etc/passwd",
	] {
		let src = source(tag, |asset| panic!("{asset} must not be requested"));
		assert!(
			update(&src, "0.6.0", false).is_err(),
			"{tag:?} was accepted"
		);
		assert!(src.asked.borrow().is_empty(), "{tag:?}");
	}
}
