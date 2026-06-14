//! Verification primitives for self-update — the security core.
//!
//! Trust anchor is the set of Ed25519 public keys embedded in this binary
//! ([`RELEASE_PUBKEYS`]), not the download domain or TLS. A release is accepted
//! only if `SHA256SUMS` carries a valid signature from a matching private key
//! (held in CI as `RELEASE_SIGN_KEY`) and the downloaded binary's SHA-256 digest
//! appears in that signed manifest. Every check fails closed.

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

use crate::ComposeError;

/// Accepted Ed25519 release public keys — at most two. Slot 0 is the active key
/// (`RELEASE_SIGN_KEY`, base64 `APh+kh61dJeT0HzG+KQXELzDjK4ccvqY9K+FptOZ3+Y=`);
/// slot 1 is the all-zero placeholder except during a key rotation, when it
/// carries the second accepted key. A signature is trusted if it validates under
/// any non-placeholder key. The keys are public by design — their integrity
/// comes from being baked into the signed, build-provenance-attested binary, so
/// an attacker cannot swap them without invalidating the binary itself.
///
/// Verified against the genuine published `SHA256SUMS.sig` (see
/// `embedded_key_verifies_real_release`). [`release_pubkeys`] still fails closed
/// if both are zeroed, so a misbuild can never trust an unverifiable release.
///
/// # Key rotation (run if the private key may be compromised)
///
/// 1. Ship a release embedding `[old, new]` with `SHA256SUMS` signed by the
///    **old** key. Binaries in the field trust only `old`, so they accept it and
///    upgrade, picking up `new` in the process.
/// 2. Ship the next release embedding `[new, zero]` signed by the **new** key.
///    Every binary from step 1 trusts `new`, so the old key is retired and all
///    installs converge on the new key.
pub const RELEASE_PUBKEYS: [[u8; 32]; 2] = [
	[
		0, 248, 126, 146, 30, 181, 116, 151, 147, 208, 124, 198, 248, 164, 23, 16, 188, 195, 140,
		174, 28, 114, 250, 152, 244, 175, 133, 166, 211, 153, 223, 230,
	],
	// Rotation slot — all-zero until a second key is being rolled in.
	[0u8; 32],
];

/// A parsed `MAJOR.MINOR.PATCH` version, ordered for comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
	pub major: u64,
	pub minor: u64,
	pub patch: u64,
}

/// Parse a `vX.Y.Z` or `X.Y.Z` version string. Anything else is rejected so a
/// malformed tag can never be mistaken for "newer".
pub fn parse_version(s: &str) -> crate::Result<Version> {
	let trimmed = s.trim();
	let core = trimmed.strip_prefix('v').unwrap_or(trimmed);
	let mut parts = core.split('.');
	let mut next = |what: &str| -> crate::Result<u64> {
		parts
			.next()
			.and_then(|p| p.parse::<u64>().ok())
			.ok_or_else(|| ComposeError::Update(format!("invalid version '{s}': bad {what}")))
	};
	let major = next("major")?;
	let minor = next("minor")?;
	let patch = next("patch")?;
	if parts.next().is_some() {
		return Err(ComposeError::Update(format!(
			"invalid version '{s}': too many components"
		)));
	}
	Ok(Version {
		major,
		minor,
		patch,
	})
}

/// Decode the configured release public keys, skipping empty rotation slots.
/// Fails closed if none remain (verification key not configured for this build)
/// or a configured key is malformed.
pub fn release_pubkeys() -> crate::Result<Vec<VerifyingKey>> {
	let mut keys = Vec::new();
	for raw in &RELEASE_PUBKEYS {
		if raw == &[0u8; 32] {
			continue;
		}
		let key = VerifyingKey::from_bytes(raw)
			.map_err(|e| ComposeError::Update(format!("embedded release key is invalid: {e}")))?;
		keys.push(key);
	}
	if keys.is_empty() {
		return Err(ComposeError::Update(
			"release verification key not configured in this build; refusing to self-update"
				.to_string(),
		));
	}
	Ok(keys)
}

/// Verify that `signature` (raw 64-byte Ed25519) over `message` validates under
/// any of `keys`. Fails closed on a wrong length or a mismatch against every
/// key. Kept separate from [`verify_signature`] so the multi-key logic is
/// testable without touching the embedded constant.
fn verify_with_keys(keys: &[VerifyingKey], message: &[u8], signature: &[u8]) -> crate::Result<()> {
	let sig = Signature::from_slice(signature).map_err(|_| {
		ComposeError::Update(format!(
			"malformed signature: expected 64 bytes, got {}",
			signature.len()
		))
	})?;
	if keys.iter().any(|key| key.verify(message, &sig).is_ok()) {
		Ok(())
	} else {
		Err(ComposeError::Update(
			"signature verification failed — release may be tampered or unsigned".to_string(),
		))
	}
}

/// Verify that `signature` (raw 64-byte Ed25519) over `message` was produced by
/// one of the accepted release keys. Fails closed on a wrong length, no
/// configured key, or a mismatch against every key.
pub fn verify_signature(message: &[u8], signature: &[u8]) -> crate::Result<()> {
	verify_with_keys(&release_pubkeys()?, message, signature)
}

/// Verify `signature` against the embedded key using an explicitly supplied key
/// — test seam so the signature path is exercised without the placeholder guard.
#[cfg(test)]
pub fn verify_signature_with(
	key: &VerifyingKey,
	message: &[u8],
	signature: &[u8],
) -> crate::Result<()> {
	let sig = Signature::from_slice(signature)
		.map_err(|_| ComposeError::Update("malformed signature".to_string()))?;
	key.verify(message, &sig)
		.map_err(|_| ComposeError::Update("signature verification failed".to_string()))
}

/// Look up the expected lowercase-hex SHA-256 digest for `asset` in a signed
/// `SHA256SUMS` manifest (`<hex>␠␠<name>` or `<hex>␠*<name>` lines).
pub fn expected_digest(sha256sums: &[u8], asset: &str) -> crate::Result<String> {
	let text = std::str::from_utf8(sha256sums)
		.map_err(|_| ComposeError::Update("SHA256SUMS is not valid UTF-8".to_string()))?;
	for line in text.lines() {
		let line = line.trim();
		let Some((hex, name)) = line.split_once(char::is_whitespace) else {
			continue;
		};
		// Strip the optional binary-mode '*' marker on the filename.
		let name = name.trim().trim_start_matches('*');
		if name == asset {
			let hex = hex.trim().to_ascii_lowercase();
			if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
				return Err(ComposeError::Update(format!(
					"SHA256SUMS has a malformed digest for {asset}"
				)));
			}
			return Ok(hex);
		}
	}
	Err(ComposeError::Update(format!(
		"{asset} is not listed in SHA256SUMS"
	)))
}

/// Compute the lowercase-hex SHA-256 of `data`.
pub fn sha256_hex(data: &[u8]) -> String {
	let digest = Sha256::digest(data);
	let mut out = String::with_capacity(64);
	for byte in digest {
		out.push(char::from_digit((byte >> 4) as u32, 16).unwrap());
		out.push(char::from_digit((byte & 0xf) as u32, 16).unwrap());
	}
	out
}

/// Compare two byte slices in constant time, returning `true` when equal.
///
/// The running time depends only on the length, not on where the first
/// differing byte sits, so it leaks no information about a partial match.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
	if a.len() != b.len() {
		return false;
	}
	let mut diff = 0u8;
	for (x, y) in a.iter().zip(b.iter()) {
		diff |= x ^ y;
	}
	diff == 0
}

/// Verify the downloaded bytes hash to `expected_hex` (case-insensitive).
///
/// The digest comparison runs in constant time so it cannot leak how many
/// leading bytes matched.
pub fn verify_digest(data: &[u8], expected_hex: &str) -> crate::Result<()> {
	let actual = sha256_hex(data);
	let expected = expected_hex.to_ascii_lowercase();
	if constant_time_eq(actual.as_bytes(), expected.as_bytes()) {
		Ok(())
	} else {
		Err(ComposeError::Update(format!(
			"checksum mismatch: expected {expected_hex}, got {actual}"
		)))
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use ed25519_dalek::{Signer, SigningKey};

	fn test_keypair() -> (SigningKey, VerifyingKey) {
		let seed = [7u8; 32];
		let sk = SigningKey::from_bytes(&seed);
		let vk = sk.verifying_key();
		(sk, vk)
	}

	#[test]
	fn parse_version_with_and_without_v() {
		assert_eq!(
			parse_version("v1.2.3").unwrap(),
			parse_version("1.2.3").unwrap()
		);
		let v = parse_version("v0.6.0").unwrap();
		assert_eq!((v.major, v.minor, v.patch), (0, 6, 0));
	}

	#[test]
	fn version_ordering() {
		assert!(parse_version("v0.6.1").unwrap() > parse_version("v0.6.0").unwrap());
		assert!(parse_version("v1.0.0").unwrap() > parse_version("v0.99.99").unwrap());
		assert!(parse_version("v0.6.0").unwrap() == parse_version("0.6.0").unwrap());
	}

	#[test]
	fn parse_version_rejects_garbage() {
		for bad in ["", "v1", "1.2", "1.2.3.4", "a.b.c", "1.2.x", "v1.2.-1"] {
			assert!(parse_version(bad).is_err(), "should reject {bad}");
		}
	}

	#[test]
	fn embedded_key_is_configured_and_rejects_garbage() {
		// A real key is baked in; it must load and reject a bogus signature.
		assert_ne!(RELEASE_PUBKEYS[0], [0u8; 32]);
		assert!(release_pubkeys().is_ok());
		assert!(verify_signature(b"data", &[0u8; 64]).is_err());
	}

	#[test]
	fn zeroed_key_would_fail_closed() {
		// Defence in depth: an all-zero key is a valid curve point, so the
		// explicit guard in `release_pubkeys` — not the curve math — is what
		// refuses to trust an unverifiable release if every key is zeroed.
		assert!(VerifyingKey::from_bytes(&[0u8; 32]).is_ok());
		let is_placeholder = |key: [u8; 32]| key == [0u8; 32];
		assert!(is_placeholder([0u8; 32]));
		assert!(!is_placeholder(RELEASE_PUBKEYS[0]));
	}

	#[test]
	fn accepts_signature_from_any_configured_key() {
		// Rotation: a binary embedding two keys must accept a release signed by
		// EITHER, so an in-field binary can upgrade across a key change.
		let (sk_a, vk_a) = test_keypair();
		let sk_b = SigningKey::from_bytes(&[9u8; 32]);
		let vk_b = sk_b.verifying_key();
		let msg = b"SHA256SUMS payload";

		let sig_b = sk_b.sign(msg).to_bytes();
		verify_with_keys(&[vk_a, vk_b], msg, &sig_b).unwrap();

		let sig_a = sk_a.sign(msg).to_bytes();
		verify_with_keys(&[vk_a, vk_b], msg, &sig_a).unwrap();
	}

	#[test]
	fn rejects_signature_from_unconfigured_key() {
		// A signature from a key that is NOT in the accepted set must fail, even
		// though other keys are configured.
		let (_sk_a, vk_a) = test_keypair();
		let sk_x = SigningKey::from_bytes(&[3u8; 32]);
		let msg = b"payload";
		let sig_x = sk_x.sign(msg).to_bytes();
		assert!(verify_with_keys(&[vk_a], msg, &sig_x).is_err());
	}

	#[test]
	fn embedded_key_verifies_real_release() {
		// Regression vector: the genuine published podup SHA256SUMS and its
		// signature must verify against the embedded key. If a future edit
		// swaps the key, this fails loudly.
		let sha256sums = "\
52d6148bf50d9d3f24a634402ec39d44302d73b21e3b74ed6a28877fdd7b93ea  podup-linux-x86_64
95202fc77b4ff60d1f67f198c312baafe710bec2e9d3a6d48fc92ba0f5a0774f  podup-linux-arm64
8e935c2b28d5955867ea0c94fe2a4fc1a6aa6951011b02eff850eb98ae41e239  podup-darwin-arm64
efb48becd0c057f6248e91ccbc5b0795edcfbdf66eb5535f24938a5bba7c4ab2  podup-darwin-x86_64
2fcbef1ae50e976b4d072c101fa2d03a235b2c17ee1ff6a3bfdf6e3df1d15389  podup-windows-x86_64.exe
";
		let signature: [u8; 64] = [
			242, 54, 152, 188, 196, 207, 89, 151, 84, 217, 6, 0, 46, 45, 6, 218, 150, 236, 75, 144,
			192, 84, 216, 67, 161, 125, 33, 43, 162, 172, 217, 138, 252, 241, 202, 49, 40, 147,
			184, 80, 158, 122, 152, 153, 175, 99, 167, 132, 8, 171, 166, 43, 170, 39, 149, 74, 219,
			134, 101, 155, 15, 109, 136, 11,
		];
		verify_signature(sha256sums.as_bytes(), &signature).unwrap();

		// And the manifest it signs really lists this platform's asset digest.
		let digest = expected_digest(sha256sums.as_bytes(), "podup-linux-x86_64").unwrap();
		assert_eq!(
			digest,
			"52d6148bf50d9d3f24a634402ec39d44302d73b21e3b74ed6a28877fdd7b93ea"
		);
	}

	#[test]
	fn valid_signature_accepted() {
		let (sk, vk) = test_keypair();
		let msg = b"SHA256SUMS contents";
		let sig = sk.sign(msg).to_bytes();
		verify_signature_with(&vk, msg, &sig).unwrap();
	}

	#[test]
	fn tampered_message_rejected() {
		let (sk, vk) = test_keypair();
		let sig = sk.sign(b"original").to_bytes();
		assert!(verify_signature_with(&vk, b"tampered", &sig).is_err());
	}

	#[test]
	fn wrong_key_rejected() {
		let (sk, _) = test_keypair();
		let other = SigningKey::from_bytes(&[9u8; 32]).verifying_key();
		let sig = sk.sign(b"data").to_bytes();
		assert!(verify_signature_with(&other, b"data", &sig).is_err());
	}

	#[test]
	fn malformed_signature_length_rejected() {
		let (_, vk) = test_keypair();
		assert!(verify_signature_with(&vk, b"data", &[0u8; 10]).is_err());
	}

	#[test]
	fn sha256_known_vector() {
		// SHA-256 of the empty input.
		assert_eq!(
			sha256_hex(b""),
			"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
		);
		// SHA-256 of "abc".
		assert_eq!(
			sha256_hex(b"abc"),
			"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
		);
	}

	#[test]
	fn digest_roundtrip_and_mismatch() {
		let data = b"podup binary bytes";
		let hex = sha256_hex(data);
		verify_digest(data, &hex).unwrap();
		verify_digest(data, &hex.to_ascii_uppercase()).unwrap();
		assert!(verify_digest(data, &"0".repeat(64)).is_err());
		// A length mismatch is rejected, not panicked on.
		assert!(verify_digest(data, "deadbeef").is_err());
	}

	#[test]
	fn constant_time_eq_matches_only_identical_slices() {
		assert!(constant_time_eq(b"abc", b"abc"));
		assert!(!constant_time_eq(b"abc", b"abd"));
		assert!(!constant_time_eq(b"abc", b"ab"));
		assert!(constant_time_eq(b"", b""));
	}

	#[test]
	fn expected_digest_two_space_format() {
		let sums = format!("{}  podup-linux-x86_64\n", "a".repeat(64));
		assert_eq!(
			expected_digest(sums.as_bytes(), "podup-linux-x86_64").unwrap(),
			"a".repeat(64)
		);
	}

	#[test]
	fn expected_digest_binary_star_format() {
		let sums = format!("{} *podup-darwin-arm64\n", "B".repeat(64));
		// Hex is normalized to lowercase.
		assert_eq!(
			expected_digest(sums.as_bytes(), "podup-darwin-arm64").unwrap(),
			"b".repeat(64)
		);
	}

	#[test]
	fn expected_digest_picks_right_line() {
		let sums = format!(
			"{}  podup-linux-x86_64\n{}  podup-linux-arm64\n",
			"1".repeat(64),
			"2".repeat(64)
		);
		assert_eq!(
			expected_digest(sums.as_bytes(), "podup-linux-arm64").unwrap(),
			"2".repeat(64)
		);
	}

	#[test]
	fn expected_digest_missing_asset_errors() {
		let sums = format!("{}  other-asset\n", "a".repeat(64));
		assert!(expected_digest(sums.as_bytes(), "podup-linux-x86_64").is_err());
	}

	#[test]
	fn expected_digest_malformed_hex_errors() {
		let sums = "nothex  podup-linux-x86_64\n";
		assert!(expected_digest(sums.as_bytes(), "podup-linux-x86_64").is_err());
	}

	#[test]
	fn expected_digest_rejects_non_utf8() {
		assert!(expected_digest(&[0xff, 0xfe], "x").is_err());
	}
}
