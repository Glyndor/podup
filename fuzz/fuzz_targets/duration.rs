#![no_main]

//! Fuzz the duration and quantity parsers. The pull-policy interval parser is
//! checked against a wider-integer reference, so a wrapped sum or product shows
//! up as a difference (a debug build also panics on the overflow). The float
//! based parsers in `size` must not hand back a value a saturating cast
//! produced: `i64::MAX` / `u64::MAX` mean the input was out of range, and the
//! seconds and nanoseconds forms must agree with each other.

use libfuzzer_sys::fuzz_target;

/// The interval syntax, summed in `u128` so nothing can wrap: one or more
/// `<digits><w|d|h|m|s>` groups. `None` for bad syntax, `Some(total)` otherwise.
fn reference_interval(text: &str) -> Option<u128> {
	let bytes = text.as_bytes();
	if bytes.is_empty() {
		return None;
	}
	let mut total: u128 = 0;
	let mut i = 0;
	while i < bytes.len() {
		let start = i;
		while i < bytes.len() && bytes[i].is_ascii_digit() {
			i += 1;
		}
		if i == start {
			return None;
		}
		let digits = text[start..i].trim_start_matches('0');
		// More than 30 significant digits cannot fit u128 with the unit applied;
		// any such count is far beyond u32 anyway.
		let count: u128 = if digits.is_empty() {
			0
		} else if digits.len() > 30 {
			u128::MAX / 1_000_000
		} else {
			digits.parse().ok()?
		};
		let unit: u128 = match bytes.get(i)? {
			b'w' => 604_800,
			b'd' => 86_400,
			b'h' => 3_600,
			b'm' => 60,
			b's' => 1,
			_ => return None,
		};
		total = total.saturating_add(count.saturating_mul(unit));
		i += 1;
	}
	Some(total)
}

fuzz_target!(|data: &[u8]| {
	let Ok(text) = std::str::from_utf8(data) else {
		return;
	};

	let expected = reference_interval(text).and_then(|t| u32::try_from(t).ok());
	assert_eq!(
		podup::fuzz_api::parse_interval(text),
		expected,
		"interval {text:?}"
	);

	if let Some(bytes) = podup::size::parse_memory(text) {
		assert!(
			bytes == -1 || (0..i64::MAX).contains(&bytes),
			"memory {text:?} gave {bytes}, a saturated or negative cast"
		);
	}
	if let Some(nanos) = podup::size::parse_cpus(text) {
		assert!(
			(0..i64::MAX).contains(&nanos),
			"cpus {text:?} gave {nanos}, a saturated or negative cast"
		);
	}
	let secs = podup::size::parse_duration_secs(text);
	let nanos = podup::size::parse_duration_nanos(text);
	if let Some(secs) = secs {
		assert!(secs < u64::MAX, "seconds {text:?} saturated to u64::MAX");
	}
	if let Some(nanos) = nanos {
		assert!(
			(0..i64::MAX).contains(&nanos),
			"nanoseconds {text:?} gave {nanos}, a saturated or negative cast"
		);
		// The range of seconds is the wider, so a value that fits in
		// nanoseconds fits in seconds, and the two agree to the rounding step.
		let secs = secs.unwrap_or_else(|| panic!("{text:?} has nanoseconds but no seconds"));
		let whole = (nanos / 1_000_000_000) as i128;
		assert!(
			(secs as i128 - whole).abs() <= 1,
			"{text:?}: {secs} s against {nanos} ns"
		);
	}
});
