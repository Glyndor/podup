//! Whole-second interval syntax shared by time policies and the refresh extension.

/// Parse one or more ASCII number/unit pairs with checked arithmetic.
/// No image age comparison uses the result: only syntax and range matter.
pub(crate) fn parse_duration(value: &str) -> Option<u32> {
	let bytes = value.as_bytes();
	let mut index = 0;
	let mut total = 0u32;
	if bytes.is_empty() {
		return None;
	}
	while index < bytes.len() {
		let start = index;
		while index < bytes.len() && bytes[index].is_ascii_digit() {
			index += 1;
		}
		if index == start {
			return None;
		}
		let count: u32 = value.get(start..index)?.parse().ok()?;
		let seconds = match bytes.get(index)? {
			b'w' => 604_800,
			b'd' => 86_400,
			b'h' => 3_600,
			b'm' => 60,
			b's' => 1,
			_ => return None,
		};
		total = total.checked_add(count.checked_mul(seconds)?)?;
		index += 1;
	}
	Some(total)
}
