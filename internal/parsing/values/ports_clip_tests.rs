//! Error text for a port value is clipped, escaped and cut on character
//! boundaries.

use super::*;
use crate::compose::types::{PortMapping, StringOrU16};

fn short(s: &str) -> PortMapping {
	PortMapping::Short(s.to_string())
}

fn port_error(mapping: PortMapping) -> String {
	parse_ports(&[mapping]).unwrap_err().to_string()
}

#[test]
fn a_megabyte_port_string_gives_a_short_error() {
	let huge = "x".repeat(1_000_000);
	for spec in [
		huge.clone(),
		format!("8080:{huge}"),
		format!("{huge}:80"),
		format!("127.0.0.1:8080:{huge}"),
		format!("[::1]:{huge}"),
		format!("[{huge}"),
		format!("1-{huge}"),
		format!("{huge}-1"),
		format!("80/{huge}"),
		format!("{}-1", "9".repeat(1_000_000)),
	] {
		let text = port_error(short(&spec));
		assert!(text.len() < 300, "{} bytes: {text}", text.len());
		assert!(text.contains("more characters)"), "{text}");
	}
}

#[test]
fn a_megabyte_long_form_published_value_gives_a_short_error() {
	let huge = "7".repeat(1_000_000);
	for published in [huge.clone(), format!("{huge}-{huge}"), format!("1-{huge}")] {
		let text = port_error(PortMapping::Long {
			target: 80,
			published: Some(StringOrU16::String(published)),
			protocol: None,
			host_ip: None,
			mode: None,
			name: None,
			app_protocol: None,
		});
		assert!(text.len() < 300, "{} bytes: {text}", text.len());
	}
	let text = port_error(PortMapping::Long {
		target: 80,
		published: None,
		protocol: Some("p".repeat(1_000_000)),
		host_ip: None,
		mode: None,
		name: None,
		app_protocol: None,
	});
	assert!(text.len() < 300, "{} bytes: {text}", text.len());
}

#[test]
fn a_cut_port_string_reports_how_many_characters_it_dropped() {
	let text = port_error(short(&format!("{}x", "1234567890".repeat(8))));
	assert!(
		text.ends_with(&format!(
			"bad port: {}...(1 more characters)",
			"1234567890".repeat(8)
		)),
		"{text}"
	);
	let text = port_error(short(&"a".repeat(100_080)));
	assert!(text.contains("...(100000 more characters)"), "{text}");
}

#[test]
fn a_short_invalid_port_shows_its_full_text() {
	assert!(port_error(short("abc")).ends_with("bad port: abc"));
	assert!(port_error(short("99999:80")).contains("out of range (1-65535): 99999"));
	assert!(port_error(short("85-80")).contains("start > end in range: 85-80"));
	assert!(port_error(short("80/banana")).contains("unsupported protocol 'banana' in '80/banana'"));
	let exact = "z".repeat(80);
	let text = port_error(short(&exact));
	assert!(
		text.ends_with(&exact) && !text.contains("more characters"),
		"{text}"
	);
}

#[test]
fn a_control_character_in_a_port_string_does_not_reach_the_message() {
	for spec in [
		"a\u{1b}[31mb",
		"80/\u{1b}[31mb",
		"8080:\u{1b}[2J",
		"[\u{1b}[31m",
		"\u{9b}31m-1",
	] {
		let text = port_error(short(spec));
		assert!(!text.chars().any(char::is_control), "{text:?}");
	}
	let huge = format!("\u{1b}{}", "1".repeat(1_000_000));
	assert!(!port_error(short(&huge)).chars().any(char::is_control));
}

#[test]
fn the_cut_never_splits_a_multibyte_character() {
	// 80 characters of 2, 3 and 4 bytes each, then more: the cut falls inside
	// what would be the middle of a character if it counted bytes.
	for ch in ['é', '€', '😀'] {
		for extra in [1usize, 2, 1000] {
			let spec: String = std::iter::repeat_n(ch, 80 + extra).collect();
			let text = port_error(short(&spec));
			let kept: String = std::iter::repeat_n(ch, 80).collect();
			assert!(
				text.ends_with(&format!("bad port: {kept}...({extra} more characters)")),
				"{ch}: {text}"
			);
		}
	}
}
