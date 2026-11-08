use super::*;

// --- exec_words -----------------------------------------------------------

#[test]
fn exec_words_reads_quotes_and_escapes_like_quadlet() {
	let w = |s: &str| exec_words(s).0;
	assert_eq!(w(r#"a "b c" 'd e'"#), ["a", "b c", "d e"]);
	assert_eq!(w(r#"a"b;c"d"#), ["ab;cd"]);
	assert_eq!(w(r#""\x3b""#), [";"]);
	assert_eq!(w(r"\073"), [";"]);
	assert_eq!(w(r";"), [";"]);
	assert_eq!(w(r#""a\"b""#), ["a\"b"]);
	assert_eq!(w(r"a\sb"), ["a b"]);
	assert!(!exec_words(r#""a ; b"#).1);
	assert!(exec_words(r#""a ; b""#).1);
}
