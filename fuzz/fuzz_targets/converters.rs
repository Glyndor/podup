#![no_main]

//! Fuzz three small compose converters whose failures are silent: the ulimit
//! pair, the device cgroup rule and the short-form volume split. The properties
//! are the ones a user relies on. A soft limit is never above its hard limit and
//! `-1` is the largest value, so nothing negative wraps into a huge limit. A
//! cgroup rule that parses has only non-negative numbers, and the same rule
//! written back parses to the same thing. An entry with no separator is a
//! container path and never a host bind.

use libfuzzer_sys::fuzz_target;
use podup::fuzz_api::{classify_short_volume, device_cgroup_rule, ulimit_pair};

fuzz_target!(|data: &[u8]| {
	if data.len() >= 16 {
		let soft = i64::from_le_bytes(data[0..8].try_into().unwrap());
		let hard = i64::from_le_bytes(data[8..16].try_into().unwrap());
		let (s, h) = ulimit_pair(soft, hard);
		assert!(s <= h, "soft {s} above hard {h} for ({soft}, {hard})");
		if hard == -1 {
			assert_eq!(h, u64::MAX);
		}
		if soft >= 0 && hard >= soft {
			assert_eq!(
				(s, h),
				(soft as u64, if hard == -1 { u64::MAX } else { hard as u64 })
			);
		}
		if hard < -1 {
			assert_eq!(h, 0, "a negative hard limit other than -1 is zero");
		}
	}

	let text = String::from_utf8_lossy(data);
	if let Some((kind, major, minor, access)) = device_cgroup_rule(&text) {
		for n in [major, minor].into_iter().flatten() {
			assert!(n >= 0, "negative device number in {text:?}");
		}
		let show = |n: Option<i64>| n.map_or("*".to_string(), |v| v.to_string());
		let again = format!(
			"{} {}:{} {}",
			kind.clone().unwrap_or_default(),
			show(major),
			show(minor),
			access.clone().unwrap_or_default()
		);
		assert_eq!(
			device_cgroup_rule(&again),
			Some((kind, major, minor, access)),
			"{text:?} did not survive being written back as {again:?}"
		);
	}

	if let Some((is_bind, _, _)) = classify_short_volume(&text) {
		let drive = text.as_bytes().len() >= 2
			&& text.as_bytes()[0].is_ascii_alphabetic()
			&& text.as_bytes()[1] == b':';
		let skip = if drive { 2 } else { 0 };
		let has_separator = text.as_bytes().iter().skip(skip).any(|&b| b == b':');
		assert!(
			!is_bind || has_separator,
			"{text:?} has no separator and still became a host bind"
		);
	}
});
