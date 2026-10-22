use super::*;

// --- encode_path_for_label ---

#[test]
fn encode_path_for_label_passes_plain_path() {
	assert_eq!(
		encode_path_for_label("/home/user/compose.yaml"),
		"/home/user/compose.yaml"
	);
}

#[test]
fn encode_path_for_label_encodes_comma() {
	// A `,` in the path would visually merge with the next entry when the
	// joined `podup.config-files` label is split back on `,`.
	assert_eq!(encode_path_for_label("a,b/c.yaml"), "a%2Cb/c.yaml");
}

#[test]
fn encode_path_for_label_encodes_multiple_commas() {
	assert_eq!(encode_path_for_label("a,b,c.yaml"), "a%2Cb%2Cc.yaml");
}
