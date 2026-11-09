#![no_main]

//! Fuzz the `SHA256SUMS` manifest lookup that decides which digest a
//! downloaded asset must match. It must never panic, and the digest it hands
//! back must be a real SHA-256: 64 hex characters, nothing else.

use std::borrow::Cow;

use libfuzzer_sys::fuzz_target;

/// How long the asset name may be before it is cut.
const MAX_ASSET_NAME: usize = 256;

fuzz_target!(|data: &[u8]| {
	// The input splits in two: the name of the asset, then the manifest it
	// has to appear in, separated by a NUL byte.
	let (name, sums) = match data.iter().position(|b| *b == 0) {
		Some(sep) => (
			String::from_utf8_lossy(&data[..sep.min(MAX_ASSET_NAME)]),
			&data[sep + 1..],
		),
		None => (Cow::Borrowed(""), data),
	};

	let Ok(digest) = podup::fuzz_api::expected_digest(sums, &name) else {
		return;
	};
	assert_eq!(digest.len(), 64, "digest {digest:?} is not 64 characters");
	assert!(
		digest.bytes().all(|b| b.is_ascii_hexdigit()),
		"digest {digest:?} holds a character that is not a hex digit"
	);
});
