//! Hostile inputs for the release verification primitives: tags, signatures and
//! manifests written to get past a check rather than to be read.
//!
//! `verify_tests.rs` pins the honest cases and the plain failures; this file
//! is for the inputs that sit next to them.

use super::*;
use ed25519_dalek::{Signer, SigningKey};

const ASSET: &str = "podup-x86_64-unknown-linux-gnu";

fn signed(message: &[u8]) -> (VerifyingKey, Vec<u8>) {
	let sk = SigningKey::from_bytes(&[7u8; 32]);
	(sk.verifying_key(), sk.sign(message).to_bytes().to_vec())
}

#[test]
fn version_strings_written_to_confuse_the_parser_are_rejected() {
	let huge = format!("v{}.0.0", "9".repeat(40));
	for bad in [
		"",
		"v",
		"latest",
		"1.2",
		"v1.2.3.4",
		"vv1.2.3",
		"v1.2.3-rc1",
		"v1.2.3+build5",
		"1.2.3 beta",
		"v1..3",
		"v1.2.-3",
		"v-1.0.0",
		"v+1.+2.+3",
		"+1.2.3",
		"1.2.+3",
		"0x1.0.0",
		"v1.2.3\n\u{1b}[2J",
		"v1.2.3\0",
		"\u{ff11}.\u{ff12}.\u{ff13}",
		huge.as_str(),
	] {
		assert!(parse_version(bad).is_err(), "{bad:?} was accepted");
	}
}

#[test]
fn padding_around_a_good_version_is_tolerated_and_nothing_else() {
	let v = parse_version(" v1.2.3\r\n").unwrap();
	assert_eq!((v.major, v.minor, v.patch), (1, 2, 3));
}

#[test]
fn a_signature_of_any_wrong_length_is_refused_by_the_embedded_keys() {
	for len in [0usize, 1, 32, 63, 65, 128, 4096] {
		let err = verify_signature(b"manifest", &vec![7u8; len]).unwrap_err();
		assert!(
			err.to_string().contains("malformed signature"),
			"{len}: {err}"
		);
	}
}

#[test]
fn degenerate_64_byte_signatures_are_refused_by_the_embedded_keys() {
	for fill in [0x00u8, 0x01, 0xff] {
		let err = verify_signature(b"manifest", &[fill; 64]).unwrap_err();
		assert!(
			err.to_string().contains("verification failed"),
			"{fill:#x}: {err}"
		);
	}
}

#[test]
fn a_signature_made_for_other_bytes_does_not_carry_over_to_these() {
	let (key, sig) = signed(b"SHA256SUMS of another product or version\n");
	assert!(verify_with_keys(&[key], b"the real manifest\n", &sig).is_err());
	// One flipped bit anywhere in the signature, both halves.
	let (key, sig) = signed(b"manifest");
	for index in [0usize, 31, 32, 63] {
		let mut bad = sig.clone();
		bad[index] ^= 1;
		assert!(
			verify_with_keys(&[key], b"manifest", &bad).is_err(),
			"bit in byte {index}"
		);
	}
}

#[test]
fn an_all_zero_key_with_an_identity_signature_does_not_verify_anything() {
	// The classic forgery for a small-order key: the identity point as the
	// key and as R, S = 0, which a lax verifier accepts for every message.
	// The embedded key is not weak, but the check that refuses this is the
	// one that also protects a rotation to a key that is.
	let mut identity = [0u8; 32];
	identity[0] = 1;
	let weak = VerifyingKey::from_bytes(&identity).unwrap();
	let mut forged = [0u8; 64];
	forged[0] = 1;
	assert!(verify_with_keys(&[weak], b"any message at all", &forged).is_err());
}

#[test]
fn a_manifest_cannot_point_the_asset_somewhere_else() {
	let digest = "a".repeat(64);
	for line in [
		format!("{digest}  ../{ASSET}"),
		format!("{digest}  /tmp/{ASSET}"),
		format!("{digest}  {ASSET}/../x"),
		format!("{digest}  {ASSET}\0"),
		format!("{digest}  x{ASSET}"),
		format!("{digest}  {}", ASSET.to_uppercase()),
	] {
		assert!(
			expected_digest(line.as_bytes(), ASSET).is_err(),
			"{line:?} matched the asset"
		);
	}
}

#[test]
fn a_manifest_digest_that_is_not_exactly_64_hex_digits_is_refused() {
	for hex in [
		"a".repeat(63),
		"a".repeat(65),
		format!("0x{}", "a".repeat(62)),
		format!("{}g", "a".repeat(63)),
		String::new(),
	] {
		let manifest = format!("{hex}  {ASSET}");
		let result = expected_digest(manifest.as_bytes(), ASSET);
		assert!(result.is_err(), "{hex:?} was accepted: {result:?}");
	}
}

#[test]
fn an_empty_expected_digest_never_matches() {
	assert!(verify_digest(b"", "").is_err());
	assert!(verify_digest(b"anything", "").is_err());
	assert!(verify_digest(b"anything", &"0".repeat(64)).is_err());
}
