use super::super::capture::{BuildCapture, MAX_CAPTURE_BYTES, MAX_CAPTURE_LINES};

#[test]
fn capture_keeps_the_tail_with_bounded_bytes_and_records() {
	let mut capture = BuildCapture::new(true);
	for _ in 0..MAX_CAPTURE_LINES * 4 {
		capture.push("x".repeat(4096));
	}
	capture.push("last diagnostic".to_string());
	let lines = capture.into_lines();
	assert!(lines.len() <= MAX_CAPTURE_LINES);
	assert!(lines.iter().map(String::len).sum::<usize>() <= MAX_CAPTURE_BYTES);
	assert_eq!(lines.last().map(String::as_str), Some("last diagnostic"));
}

#[test]
fn capture_limits_tiny_records_too() {
	let mut capture = BuildCapture::new(true);
	for _ in 0..MAX_CAPTURE_LINES * 4 {
		capture.push(String::new());
	}
	assert!(capture.into_lines().len() <= MAX_CAPTURE_LINES);
}

#[test]
fn oversized_record_keeps_its_utf8_tail_without_retaining_the_allocation() {
	let mut capture = BuildCapture::new(true);
	let line = format!("{}final error", "€".repeat(MAX_CAPTURE_BYTES));
	capture.push(line);
	let lines = capture.into_lines();
	assert_eq!(lines.len(), 1);
	assert!(lines[0].len() <= MAX_CAPTURE_BYTES);
	assert!(lines[0].capacity() <= MAX_CAPTURE_BYTES);
	assert!(lines[0].ends_with("final error"));
}

#[test]
fn disabled_capture_retains_nothing() {
	let mut capture = BuildCapture::new(false);
	capture.push("diagnostic already printed or quiet".to_string());
	assert!(capture.into_lines().is_empty());
}
