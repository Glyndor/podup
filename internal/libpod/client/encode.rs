//! Percent-encoding for libpod REST path segments.

/// Percent-encode a string for use as a single URL path/query segment, encoding
/// everything outside the RFC 3986 unreserved set so container names, project
/// names, and tags can contain arbitrary bytes without breaking the request.
pub(crate) fn urlencoded(s: &str) -> String {
	let mut out = String::with_capacity(s.len());
	for b in s.bytes() {
		match b {
			b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
				out.push(b as char);
			}
			_ => {
				out.push('%');
				out.push(
					char::from_digit((b >> 4) as u32, 16)
						.unwrap()
						.to_ascii_uppercase(),
				);
				out.push(
					char::from_digit((b & 0xf) as u32, 16)
						.unwrap()
						.to_ascii_uppercase(),
				);
			}
		}
	}
	out
}

#[cfg(test)]
mod tests {
	use super::urlencoded;

	#[test]
	fn unreserved_chars_pass_through() {
		assert_eq!(urlencoded("abc-XYZ_0.9~"), "abc-XYZ_0.9~");
	}

	#[test]
	fn space_encoded() {
		assert_eq!(urlencoded("hello world"), "hello%20world");
	}

	#[test]
	fn slash_encoded() {
		assert_eq!(urlencoded("a/b"), "a%2Fb");
	}

	#[test]
	fn colon_encoded() {
		assert_eq!(urlencoded("myproj:v1"), "myproj%3Av1");
	}

	#[test]
	fn empty_string() {
		assert_eq!(urlencoded(""), "");
	}

	#[test]
	fn unicode_byte_encoded() {
		// '€' = 0xE2 0x82 0xAC in UTF-8
		assert_eq!(urlencoded("€"), "%E2%82%AC");
	}

	#[test]
	fn container_name_typical() {
		assert_eq!(urlencoded("myproject-web"), "myproject-web");
	}

	#[test]
	fn container_name_with_brackets() {
		assert_eq!(urlencoded("a[b]"), "a%5Bb%5D");
	}
}
