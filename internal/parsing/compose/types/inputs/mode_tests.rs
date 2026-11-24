use super::*;

fn bits(text: &str) -> u32 {
	parse_mode_text(text)
		.unwrap_or_else(|_| panic!("a mode that should be read was refused"))
		.bits
}

/// Every spelling of the same mode, and what each is read as.
#[test]
fn the_forms_are_read_as_the_spec_and_docker_read_them() {
	for (text, want, bare) in [
		("0o644", 0o644, false),
		("0O644", 0o644, false),
		("0o404", 0o404, false),
		("0o400", 0o400, false),
		("0o440", 0o440, false),
		("0o600", 0o600, false),
		("0o755", 0o755, false),
		("0o7777", 0o7777, false),
		("0644", 0o644, false),
		("0400", 0o400, false),
		("00", 0, false),
		("0x100", 0o400, false),
		("0x1a4", 0o644, false),
		("0X1A4", 0o644, false),
		("644", 0o644, true),
		("400", 0o400, true),
		("420", 0o420, true),
		("256", 0o256, true),
	] {
		let read = parse_mode_text(text)
			.unwrap_or_else(|_| panic!("a mode that should be read was refused"));
		assert_eq!((read.bits, read.bare), (want, bare));
	}
	// A digit that is 8 or 9 cannot be octal: the decimal value.
	let read = parse_mode_text("384").unwrap();
	assert_eq!((read.bits, read.bare), (384, true));
	assert_eq!(bits("0"), 0);
	assert_eq!(bits("  0644  "), 0o644);
}

#[test]
fn what_is_not_a_mode_is_refused() {
	for text in [
		"",
		" ",
		"-1",
		"+644",
		"1.5",
		"0o",
		"0o8",
		"0o789",
		"0x",
		"0xg",
		"0888",
		"0o17777",
		"0x1000",
		"12345678901",
		"abc",
		"0644x",
		"true",
		"0o-7",
		"0x-1",
	] {
		assert!(parse_mode_text(text).is_err(), "was accepted");
	}
}

#[test]
fn control_characters_in_a_refused_value_are_not_printed() {
	let why = parse_mode_text("0o7\u{1b}[31m").unwrap_err();
	assert!(!why.contains('\u{1b}'));
}

#[test]
fn the_notice_says_what_was_applied_and_how_to_write_it() {
	let notice = bare_notice("644", 0o644);
	assert!(notice.contains("without a leading 0"));
	assert!(notice.contains("0644 was applied"));
	assert!(notice.contains("\"0644\"") && notice.contains("0o644"));
	assert!(!notice.contains("docker"));
}

/// A number whose spelling is lost is read only when its two readings agree.
#[test]
fn a_number_without_a_spelling_is_applied_only_when_it_is_not_a_guess() {
	// 0o600 decodes to 384: its digits are not octal, so the number is the only reading.
	assert_eq!(read_unknown_number(384), Ok(0o600));
	// 0o755 decodes to 493, the same.
	assert_eq!(read_unknown_number(493), Ok(0o755));
	// 0 and the single octal digits read the same either way.
	for n in 0..8 {
		assert_eq!(read_unknown_number(n), Ok(n));
	}
	// 420 is 0o644 decoded, or the digits 0420 typed: refused.
	for n in [420, 644, 400, 256, 404, 640, 660, 700, 755, 10, 77] {
		let why = read_unknown_number(n).unwrap_err();
		assert!(why.contains(&format!("mode {n}")));
		assert!(why.contains("quoted octal string") && why.contains("0o"));
	}
	// Above 0o7777 is not a mode.
	assert!(read_unknown_number(99999).is_err());
}

fn yaml(text: &str) -> serde_yaml::Value {
	serde_yaml::from_str(text).unwrap()
}

#[test]
fn the_modes_of_secrets_configs_and_tmpfs_are_written_as_text() {
	let mut doc = yaml(
		"services:\n  web:\n    secrets:\n      - source: s\n        mode: 420\n    configs:\n      - source: c\n        mode: 256\n    volumes:\n      - type: tmpfs\n        tmpfs:\n          mode: 1023\n    labels:\n      mode: 420\n    ports:\n      - mode: host\n",
	);
	super::super::mode_render::modes_as_text_yaml(&mut doc);
	let out = serde_yaml::to_string(&doc).unwrap();
	assert!(out.contains("mode: '0644'") || out.contains("mode: \"0644\""));
	assert!(out.contains("0400") && out.contains("01777"));
	// A label or a port that has a key `mode` is not a mode place.
	assert!(out.contains("labels:\n      mode: 420"));
}

#[test]
fn the_json_form_is_the_same() {
	let mut doc: serde_json::Value = serde_json::from_str(
		r#"{"services":{"web":{"secrets":[{"source":"s","mode":420}],"volumes":[{"tmpfs":{"mode":448}}],"labels":{"mode":420}}}}"#,
	)
	.unwrap();
	super::super::mode_render::modes_as_text_json(&mut doc);
	assert_eq!(doc["services"]["web"]["secrets"][0]["mode"], "0644");
	assert_eq!(
		doc["services"]["web"]["volumes"][0]["tmpfs"]["mode"],
		"0700"
	);
	assert_eq!(doc["services"]["web"]["labels"]["mode"], 420);
}
