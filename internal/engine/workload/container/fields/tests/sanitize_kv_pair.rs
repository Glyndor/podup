use super::*;

// --- sanitize_kv_pair ---

#[test]
fn sanitize_kv_pair_accepts_normal_input() {
	let mut labels = HashMap::new();
	let (k, v) = sanitize_kv_pair(&mut labels, "com.example.team", "blue").unwrap();
	assert_eq!(k, "com.example.team");
	assert_eq!(v, "blue");
	assert_eq!(
		labels.get("com.example.team").map(String::as_str),
		Some("blue")
	);
}

#[test]
fn sanitize_kv_pair_rejects_empty_key() {
	let mut labels = HashMap::new();
	// An empty key would collide on every subsequent empty entry; reject at
	// the boundary rather than silently deduplicating.
	assert_eq!(
		sanitize_kv_pair(&mut labels, "", "v"),
		Err(SanitizeError::InvalidKey)
	);
	assert!(labels.is_empty());
}

#[test]
fn sanitize_kv_pair_rejects_control_char_in_key() {
	let mut labels = HashMap::new();
	// \t in the middle of the key: a downstream consumer that splits on
	// whitespace would see two keys.
	assert_eq!(
		sanitize_kv_pair(&mut labels, "bad\tkey", "v"),
		Err(SanitizeError::InvalidKey)
	);
	assert!(labels.is_empty());
}

#[test]
fn sanitize_kv_pair_rejects_control_char_in_value() {
	let mut labels = HashMap::new();
	// NUL in the value: meaningless in a label and would corrupt any
	// downstream parser that treats the value as a C string.
	assert_eq!(
		sanitize_kv_pair(&mut labels, "k", "bad\0val"),
		Err(SanitizeError::InvalidValue)
	);
	assert!(labels.is_empty());
}

#[test]
fn sanitize_kv_pair_rejects_oversize_key() {
	let mut labels = HashMap::new();
	// 254 bytes, one past the podman cap.
	let big = "a".repeat(MAX_LABEL_KEY_LEN + 1);
	assert_eq!(
		sanitize_kv_pair(&mut labels, &big, "v"),
		Err(SanitizeError::InvalidKey)
	);
	assert!(labels.is_empty());
}

#[test]
fn sanitize_kv_pair_accepts_key_at_cap() {
	let mut labels = HashMap::new();
	// Exactly the cap is the inclusive boundary.
	let at_cap = "a".repeat(MAX_LABEL_KEY_LEN);
	let (k, _) = sanitize_kv_pair(&mut labels, &at_cap, "v").unwrap();
	assert_eq!(k.len(), MAX_LABEL_KEY_LEN);
}

#[test]
fn sanitize_kv_pair_rejects_oversize_value() {
	let mut labels = HashMap::new();
	// 4 KiB + 1, past the value cap.
	let big = "a".repeat(MAX_LABEL_VALUE_LEN + 1);
	assert_eq!(
		sanitize_kv_pair(&mut labels, "k", &big),
		Err(SanitizeError::InvalidValue)
	);
	assert!(labels.is_empty());
}

#[test]
fn sanitize_kv_pair_rejects_when_map_is_full() {
	let mut labels = HashMap::new();
	for i in 0..MAX_LABEL_FILE_ENTRIES {
		labels.insert(format!("k{i}"), "v".into());
	}
	// A new key at the cap is rejected.
	assert_eq!(
		sanitize_kv_pair(&mut labels, "new-key", "v"),
		Err(SanitizeError::TooManyEntries)
	);
	// ...but overwriting an existing key at the cap is allowed: the cap
	// bounds the number of distinct keys, not total insertions.
	assert!(sanitize_kv_pair(&mut labels, "k0", "new-v").is_ok());
	assert_eq!(labels.get("k0").map(String::as_str), Some("new-v"));
}
