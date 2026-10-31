//! The linear-time glob matcher must decide exactly like the recursive one it
//! replaced. The old matcher is kept here, verbatim, as the reference.

use super::ignore_patterns::glob_rec;

fn old_glob_rec(pat: &[u8], s: &[u8]) -> bool {
	if pat.is_empty() {
		return s.is_empty();
	}
	if pat.starts_with(b"**") {
		let rest = &pat[2..];
		if rest.first() == Some(&b'/') && old_glob_rec(&rest[1..], s) {
			return true;
		}
		for i in 0..=s.len() {
			if old_glob_rec(rest, &s[i..]) {
				return true;
			}
		}
		return false;
	}
	match pat[0] {
		b'*' => {
			let mut i = 0;
			loop {
				if old_glob_rec(&pat[1..], &s[i..]) {
					return true;
				}
				if i >= s.len() || s[i] == b'/' {
					return false;
				}
				i += 1;
			}
		}
		b'?' => !s.is_empty() && s[0] != b'/' && old_glob_rec(&pat[1..], &s[1..]),
		c => !s.is_empty() && s[0] == c && old_glob_rec(&pat[1..], &s[1..]),
	}
}

struct Lcg(u64);

impl Lcg {
	fn next(&mut self, bound: usize) -> usize {
		self.0 = self
			.0
			.wrapping_mul(6364136223846793005)
			.wrapping_add(1442695040888963407);
		((self.0 >> 33) as usize) % bound
	}

	fn string(&mut self, alphabet: &[u8], max_len: usize) -> Vec<u8> {
		let len = self.next(max_len + 1);
		(0..len)
			.map(|_| alphabet[self.next(alphabet.len())])
			.collect()
	}
}

fn agree(pat: &[u8], s: &[u8]) {
	assert_eq!(
		glob_rec(pat, s),
		old_glob_rec(pat, s),
		"pattern {:?} against {:?}",
		String::from_utf8_lossy(pat),
		String::from_utf8_lossy(s)
	);
}

#[test]
fn a_hand_written_table_agrees_with_the_old_matcher() {
	let patterns = [
		"",
		"*",
		"**",
		"***",
		"****",
		"?",
		"??",
		"a",
		"a*",
		"*a",
		"*a*",
		"a*b",
		"a?b",
		"**/a",
		"a/**",
		"a/**/b",
		"**/a/**",
		"**/**/a",
		"**/",
		"/**",
		"a/**/",
		"*/",
		"*/a",
		"a/*",
		"a/*/b",
		"**a",
		"a**",
		"a**b",
		"a**/b",
		"**/*.log",
		"*.log",
		"a/*.log",
		"?/?",
		"**?",
		"**/?",
		"*/**",
		"**/*",
		"a/**/**/b",
		"***/a",
		"**/**",
	];
	let paths = [
		"",
		"a",
		"b",
		"/",
		"a/",
		"/a",
		"a/b",
		"a/a",
		"a/b/c",
		"a/b/b",
		"a/a/b",
		"ab",
		"aab",
		"abb",
		"a/b/c/d",
		"x.log",
		"a/x.log",
		"a/b/x.log",
		"a//b",
		"//",
		"aaaa",
		"a/a/a/a",
	];
	for p in patterns {
		for s in paths {
			agree(p.as_bytes(), s.as_bytes());
		}
	}
}

#[test]
fn random_patterns_and_paths_agree_with_the_old_matcher() {
	let mut rng = Lcg(0x9e37_79b9_7f4a_7c15);
	let mut matches = 0;
	let total = 60_000;
	for _ in 0..total {
		let pat = rng.string(b"ab/*?*/", 9);
		let s = rng.string(b"ab/", 10);
		agree(&pat, &s);
		if glob_rec(&pat, &s) {
			matches += 1;
		}
	}
	// The sample must exercise both outcomes or the comparison proves little.
	assert!(
		matches > total / 50 && matches < total * 9 / 10,
		"{matches} of {total} matched"
	);
}

#[test]
fn random_wildcard_heavy_patterns_agree_with_the_old_matcher() {
	let mut rng = Lcg(42);
	for _ in 0..30_000 {
		let pat = rng.string(b"a**/*?", 10);
		let s = rng.string(b"a/", 12);
		agree(&pat, &s);
	}
}
