use super::{is_valid_object_name, urlencoded};

#[test]
fn valid_object_names_accepted() {
	assert!(is_valid_object_name("web"));
	assert!(is_valid_object_name("proj-web-1"));
	assert!(is_valid_object_name("a.b_c-1"));
	assert!(is_valid_object_name("0abc"));
}

#[test]
fn invalid_object_names_rejected() {
	assert!(!is_valid_object_name(""));
	assert!(!is_valid_object_name("-leading-dash"));
	assert!(!is_valid_object_name(".leading-dot"));
	assert!(!is_valid_object_name("has space"));
	assert!(!is_valid_object_name("has/slash"));
	assert!(!is_valid_object_name("tab\tname"));
	assert!(!is_valid_object_name("emoji😀"));
}

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

/// A name from a compose file ends up in the request line sent to the socket, so
/// whatever bytes it holds, the encoded form may only hold unreserved characters
/// and `%XX` escapes: no CR, LF, NUL, space, `/`, `?`, `#` or `%` of its own.
#[test]
fn every_byte_encodes_to_unreserved_or_an_escape() {
	for b in 0u8..=255 {
		let text = String::from_utf8_lossy(&[b]).into_owned();
		let out = urlencoded(&text);
		let mut chars = out.chars().peekable();
		while let Some(c) = chars.next() {
			if c == '%' {
				let hi = chars.next().expect("escape needs two digits");
				let lo = chars.next().expect("escape needs two digits");
				assert!(hi.is_ascii_hexdigit() && lo.is_ascii_hexdigit(), "{out:?}");
				assert!(
					!hi.is_ascii_lowercase() && !lo.is_ascii_lowercase(),
					"{out:?}"
				);
			} else {
				assert!(
					c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~'),
					"byte {b:#04x} left {c:?} in {out:?}"
				);
			}
		}
	}
}

#[test]
fn request_line_breakers_never_survive() {
	for hostile in [
		"a\r\nHost: evil",
		"a\0b",
		"a b",
		"a?x=1",
		"a#frag",
		"a%2e%2e",
	] {
		let out = urlencoded(hostile);
		assert!(
			!out.contains(['\r', '\n', '\0', ' ', '?', '#', '/']),
			"{out:?}"
		);
	}
	// A percent sign in the input is itself escaped, so it cannot spell `%2e`.
	assert_eq!(urlencoded("%2e"), "%252e");
}

/// `.` and `..` are unreserved and pass through, so a path segment made of
/// nothing but dots is only safe because object names must start with an
/// alphanumeric first; keep that gate in step.
#[test]
fn dot_segments_are_not_valid_object_names() {
	assert_eq!(urlencoded(".."), "..");
	assert!(!is_valid_object_name("."));
	assert!(!is_valid_object_name(".."));
	assert!(!is_valid_object_name("..%2f"));
}
