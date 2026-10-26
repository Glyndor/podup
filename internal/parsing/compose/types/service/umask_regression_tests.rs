//! Named diagnostics and unchanged serialization outside the service mask.

use crate::compose::merge::deserialize_with_merge_interp;
use crate::compose::parse_str_raw;

#[test]
fn umask_large_integer_keeps_service_and_quote_hint() {
	for value in ["18446744073709551616", "-18446744073709551617"] {
		let error = parse_str_raw(&format!(
			"services:\n  first:\n    umask: '0022'\n  worker:\n    umask: {value}\n"
		))
		.unwrap_err()
		.to_string();
		assert!(error.contains("worker"), "{error}");
		assert!(
			error.contains("umask") && error.contains("quote"),
			"{error}"
		);
	}
}

#[test]
fn umask_nested_keys_preserve_previous_bytes_and_hash() {
	use sha2::{Digest, Sha256};
	let vars = std::collections::HashMap::new();
	for (text, literal) in [("22", "22"), ("true", "true")] {
		let input = format!("services:\n  web:\n    image: alpine\n    environment:\n      umask: '${{VALUE:-{text}}}'\n    x-metadata:\n      umask: '${{VALUE:-{text}}}'\n");
		let file = deserialize_with_merge_interp(&input, Some(&vars)).unwrap();
		let previous =
			parse_str_raw(&input.replace(&format!("'${{VALUE:-{text}}}'"), literal)).unwrap();
		let bytes = |file: &crate::compose::types::ComposeFile| {
			serde_json::to_vec(&serde_json::to_value(&file.services["web"]).unwrap()).unwrap()
		};
		let actual = bytes(&file);
		let expected = bytes(&previous);
		let frozen = format!(
			r#"{{"annotations":null,"depends_on":null,"dns":null,"dns_opt":null,"dns_search":null,"env_file":null,"environment":{{"umask":{literal}}},"image":"alpine","label_file":null,"labels":null,"networks":null,"sysctls":null,"tmpfs":null,"x-metadata":{{"umask":{literal}}}}}"#
		);
		assert_eq!(expected, frozen.as_bytes());
		assert_eq!(actual, expected);
		assert_eq!(Sha256::digest(&actual), Sha256::digest(&expected));
		assert!(file.services["web"].umask.is_none());
		assert!(!String::from_utf8(actual)
			.unwrap()
			.contains("umask_merge_tag"));
	}
}

#[test]
fn umask_tagged_placeholder_fails_loudly() {
	let error = deserialize_with_merge_interp(
		"services:\n  worker:\n    umask: !override '$${MASK:-0022}'\n",
		Some(&std::collections::HashMap::new()),
	)
	.unwrap_err()
	.to_string();
	assert!(
		error.contains("worker") && error.contains("variable did not resolve"),
		"{error}"
	);
}
