use super::{redact_url, redact_urls_in_text};

#[test]
fn the_user_information_and_the_query_go() {
	assert_eq!(
		redact_url("https://user:s3cr3t@host.example/org/repo.git?token=abc123"),
		"https://host.example/org/repo.git"
	);
}

#[test]
fn a_token_alone_in_the_user_information_goes_too() {
	assert_eq!(
		redact_url("https://ghp_token@host.example/repo.git"),
		"https://host.example/repo.git"
	);
}

#[test]
fn the_fragment_that_names_the_reference_stays() {
	assert_eq!(
		redact_url("https://u:p@host.example/repo.git?x=1#main:docker"),
		"https://host.example/repo.git#main:docker"
	);
}

#[test]
fn an_at_sign_in_the_path_or_fragment_is_not_user_information() {
	assert_eq!(
		redact_url("https://host.example/a@b/repo.git#v@1"),
		"https://host.example/a@b/repo.git#v@1"
	);
}

#[test]
fn a_port_and_a_url_with_nothing_to_hide_are_unchanged() {
	assert_eq!(
		redact_url("https://host.example:8443/repo.git"),
		"https://host.example:8443/repo.git"
	);
	assert_eq!(
		redact_url("git://host.example/repo.git"),
		"git://host.example/repo.git"
	);
}

#[test]
fn an_authority_with_no_path_is_handled() {
	assert_eq!(
		redact_url("https://u:p@host.example"),
		"https://host.example"
	);
	assert_eq!(
		redact_url("https://u:p@host.example?token=x"),
		"https://host.example"
	);
}

#[test]
fn the_scp_form_keeps_its_user_and_loses_a_password() {
	assert_eq!(
		redact_url("git@host.example:org/repo.git"),
		"git@host.example:org/repo.git"
	);
	assert_eq!(
		redact_url("user:pw@host.example:org/repo.git"),
		"host.example:org/repo.git"
	);
}

#[test]
fn every_url_in_a_message_is_redacted_and_the_rest_is_kept() {
	let message = "cloning \"https://u:pw@127.0.0.1:1/x.git?token=tk#main\" to \"/var/tmp/b\":\n\
		fatal: unable to access 'https://127.0.0.1:1/x.git?token=tk/': Failed to connect";
	assert_eq!(
		redact_urls_in_text(message),
		"cloning \"https://127.0.0.1:1/x.git#main\" to \"/var/tmp/b\":\n\
		fatal: unable to access 'https://127.0.0.1:1/x.git': Failed to connect"
	);
}

#[test]
fn text_without_a_url_is_unchanged() {
	for text in [
		"",
		"no such container: web",
		"a:b c@d d?e#f",
		"ends with ://",
	] {
		assert_eq!(redact_urls_in_text(text), text);
	}
}

#[test]
fn a_url_at_the_end_of_the_text_and_two_in_a_row_are_handled() {
	assert_eq!(
		redact_urls_in_text("see https://u:p@h.example/x?q=1 and http://a:b@i.example"),
		"see https://h.example/x and http://i.example"
	);
}

// Text that is not ASCII in front of, inside and behind a URL: the scheme of a
// URL is found by walking back over characters, and a multibyte one in front of
// it used to put the cut inside that character.

#[test]
fn text_that_is_not_ascii_in_front_of_a_url_does_not_panic_and_is_kept() {
	for (index, (text, expected)) in [
		(
			"\u{201c}https://u:p@h/x\u{201d}",
			"\u{201c}https://h/x\u{201d}",
		),
		("\u{e9}https://u:p@h/x", "\u{e9}https://h/x"),
		("\u{1f600}https://u:p@h/x", "\u{1f600}https://h/x"),
		(
			"\u{4e2d}\u{6587}https://u:p@h/x",
			"\u{4e2d}\u{6587}https://h/x",
		),
		(
			"https://u:p@h/\u{4e2d}\u{6587}",
			"https://h/\u{4e2d}\u{6587}",
		),
		(
			"url: \u{1f600} https://u:p@h/x \u{1f600}",
			"url: \u{1f600} https://h/x \u{1f600}",
		),
		("\u{e9}://", "\u{e9}://"),
		("\u{1f600}://u:p@h", "\u{1f600}://h"),
	]
	.into_iter()
	.enumerate()
	{
		// The message names the case and never the text: the text holds the
		// credentials the case checks for.
		assert!(redact_urls_in_text(text) == expected, "case {index}");
	}
}

#[test]
fn a_url_held_inside_another_one_is_cleaned_too() {
	assert_eq!(
		redact_urls_in_text("https://proxy.example/https://u:s3cr3t@host.example/x"),
		"https://proxy.example/https://host.example/x"
	);
	assert_eq!(
		redact_urls_in_text("https://a:b@proxy.example/https://u:s3cr3t@host.example/x?t=1"),
		"https://proxy.example/https://host.example/x"
	);
}

#[test]
fn punctuation_that_is_valid_in_a_password_or_a_token_does_not_end_the_url() {
	// The messages name the case and never the value: the values are the
	// credentials the case checks for.
	for (index, password) in ["pa,ss", "pa'ss", "pa(ss", "pa)ss", "pa(s)s,'", "a,b(c)d'e"]
		.into_iter()
		.enumerate()
	{
		let text = format!("cloning https://user:{password}@host.example/x.git failed");
		assert!(
			redact_urls_in_text(&text) == "cloning https://host.example/x.git failed",
			"password case {index}"
		);
	}
	for (index, token) in ["to,k", "to'k", "to(k", "to)k", "(t,o'k)"]
		.into_iter()
		.enumerate()
	{
		let text = format!("cloning https://host.example/x.git?token={token} failed");
		assert!(
			redact_urls_in_text(&text) == "cloning https://host.example/x.git failed",
			"token case {index}"
		);
	}
}

#[test]
fn a_quoted_url_is_cleaned_whatever_quote_it_holds() {
	assert_eq!(
		redact_urls_in_text("'https://u:pa'ss,(x)@h.example/x?t=a'b'"),
		"'https://h.example/x'"
	);
	assert_eq!(
		redact_urls_in_text("\"https://u:pa'ss@h.example/x\" and more"),
		"\"https://h.example/x\" and more"
	);
	assert_eq!(
		redact_urls_in_text("\u{201c}https://u:p,s@h.example/x\u{201d}"),
		"\u{201c}https://h.example/x\u{201d}"
	);
	assert_eq!(
		redact_urls_in_text("cloning 'https://u:pw@h.example/x' to '/var/tmp/b'"),
		"cloning 'https://h.example/x' to '/var/tmp/b'"
	);
}

#[test]
fn the_punctuation_that_closes_a_sentence_is_kept() {
	assert_eq!(
		redact_urls_in_text("see https://h.example/x, or https://h.example/y."),
		"see https://h.example/x, or https://h.example/y."
	);
	assert_eq!(
		redact_urls_in_text("(see https://u:p@h.example/x)"),
		"(see https://h.example/x)"
	);
}

#[test]
fn a_known_url_is_cleaned_wherever_it_is_repeated_and_so_are_its_parts() {
	// A password with a slash in it cannot be told from a path by shape alone.
	let url = "https://ciusr9x:Sl4sh/Pw9@host.example/org/repo.git?access=Qy7TokenZ";
	super::remember_url(url);
	assert_eq!(
		redact_urls_in_text(url),
		"https://host.example/org/repo.git"
	);
	let echoed = "fatal: bad password Sl4sh/Pw9 and token Qy7TokenZ for ciusr9x";
	let shown = redact_urls_in_text(echoed);
	for (index, secret) in ["Sl4sh/Pw9", "Qy7TokenZ", "ciusr9x"]
		.into_iter()
		.enumerate()
	{
		assert!(
			!shown.contains(secret),
			"part {index} of the known URL came out"
		);
	}
	assert!(
		shown.contains("fatal: bad password"),
		"the text around the credentials was lost"
	);
}

#[test]
fn a_known_url_in_its_percent_encoded_form_hides_the_decoded_credential() {
	super::remember_url("https://rbusr7q:p%40ss%2Fw0rd@host.example/x.git");
	let shown = redact_urls_in_text("could not log in with p@ss/w0rd");
	assert!(
		!shown.contains("p@ss/w0rd"),
		"the decoded credential came out"
	);
}

#[test]
fn a_port_is_not_taken_for_user_information() {
	assert_eq!(
		redact_urls_in_text("https://host.example:8080/a@b"),
		"https://host.example:8080/a@b"
	);
}

/// A tiny xorshift, so the property runs on the same inputs every time.
struct Rng(u64);

impl Rng {
	fn next(&mut self) -> u64 {
		self.0 ^= self.0 << 13;
		self.0 ^= self.0 >> 7;
		self.0 ^= self.0 << 17;
		self.0
	}

	fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
		items[(self.next() % items.len() as u64) as usize]
	}
}

const PIECES: &[&str] = &[
	"https",
	"://",
	"@",
	":",
	"/",
	"?",
	"#",
	"=",
	"&",
	"%",
	"%4",
	"%zz",
	"a",
	"Z",
	"9",
	" ",
	"\n",
	"\"",
	"'",
	"`",
	"<",
	">",
	"(",
	")",
	",",
	".",
	"\u{e9}",
	"\u{201c}",
	"\u{201d}",
	"\u{1f600}",
	"\u{4e2d}",
	"\u{0}",
	"\u{7f}",
	"user",
	"git@",
	"x.git",
	"ftp+ssh",
	"..",
	"-",
	"+",
];

#[test]
fn no_text_makes_the_functions_panic() {
	let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
	for _ in 0..30_000 {
		let len = 1 + (rng.next() % 40) as usize;
		let text: String = (0..len).map(|_| rng.pick(PIECES)).collect();
		let _ = redact_urls_in_text(&text);
		let _ = redact_url(&text);
		let _ = super::parts_of(&text);
	}
}

/// Characters a secret can hold and still be one URL to the cleaner: no `/`,
/// `@`, `?`, `#`, blank, `"`, `<` or `>`, which end the user information or the
/// URL.
const SECRET_CHARS: &[&str] = &[
	"a",
	"B",
	"7",
	",",
	"'",
	"(",
	")",
	"!",
	"$",
	"*",
	";",
	":",
	"=",
	"&",
	"%",
	"\u{e9}",
	"\u{4e2d}",
	"\u{1f600}",
	"-",
	"_",
	".",
];

#[test]
fn a_password_and_a_token_put_into_any_text_never_come_out() {
	let mut rng = Rng(0x2545_f491_4f6c_dd1d);
	let around: &[&str] = &[
		"",
		" ",
		"cloning ",
		"\"",
		"'",
		"(",
		"\u{201c}",
		"\u{e9}",
		"\u{1f600}",
		"see ",
		"x\n",
	];
	for round in 0..20_000 {
		let secret = |rng: &mut Rng, tag: &str| -> String {
			let n = 1 + (rng.next() % 6) as usize;
			let body: String = (0..n).map(|_| rng.pick(SECRET_CHARS)).collect();
			format!("{tag}{round:05}{body}")
		};
		let password = secret(&mut rng, "PW");
		let token = secret(&mut rng, "TK");
		let url = format!("https://user:{password}@host.example/org/repo.git?token={token}#main");
		let before = rng.pick(around);
		let after = rng.pick(around);
		let text = format!("{before}{url}{after} and again {url}");
		let shown = redact_urls_in_text(&text);
		// The failure names the iteration of a fixed seed, never the values,
		// so it can be reproduced without printing a credential.
		assert!(
			!shown.contains(&password),
			"the password came out at iteration {round}"
		);
		assert!(
			!shown.contains(&token),
			"the token came out at iteration {round}"
		);
	}
}

#[test]
fn the_end_of_a_token_that_is_punctuation_does_not_come_back_with_the_text() {
	assert_eq!(
		redact_urls_in_text("cloning https://h.example/x?token=ab!. now"),
		"cloning https://h.example/x now"
	);
	assert_eq!(
		redact_urls_in_text("(https://h.example/x?token=ab)"),
		"(https://h.example/x"
	);
}

#[test]
fn a_password_in_an_image_reference_put_into_any_text_never_comes_out() {
	let mut rng = Rng(0x7a3c_91e5_55d2_0b17);
	let around: &[&str] = &[
		"",
		" ",
		"\"",
		"'",
		"(",
		"\u{201c}",
		"\u{e9}",
		"\u{1f600}",
		"parsing reference ",
		"x\n",
	];
	for round in 0..20_000 {
		let n = 1 + (rng.next() % 6) as usize;
		let body: String = (0..n).map(|_| rng.pick(SECRET_CHARS)).collect();
		let password = format!("PW{round:05}{body}");
		let reference = format!("u{round}:{password}@registry.example/team/img:tag");
		let before = rng.pick(around);
		let after = rng.pick(around);
		let shown = redact_urls_in_text(&format!("{before}{reference}{after} and {reference}"));
		assert!(
			!shown.contains(&password),
			"the password came out at iteration {round}"
		);
	}
}
