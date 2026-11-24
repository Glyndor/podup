//! How a compose `mode:` is read from the text it was written as.
//!
//! The Compose Specification says a mode is octal. YAML decodes a number before
//! the typed reader sees it (`0o644` arrives as 420, `0644` as 644), so the
//! spelling has to be read from the text. The rules, for a secret, a config and a
//! tmpfs alike:
//!
//! - `0o` and octal digits (`0o644`) are that octal number;
//! - a leading `0` and octal digits (`0644`, `0400`) are that octal number;
//! - `0x` and hex digits (`0x1a4`) are that number, as docker compose reads them;
//! - an integer with no leading zero (`644`, `420`, `256`) is read as it always
//!   has been, so a file that deploys keeps deploying: its digits are octal when
//!   all are below 8, and the decimal value when one is 8 or 9. It is also the one
//!   spelling docker compose reads differently, so it is reported;
//! - anything else (a sign, a fraction, an octal with an 8 or a 9 after a `0` or
//!   `0o`, text, a value above `0o7777`) is refused.

use serde_yaml::Value;

use crate::error::{sanitize_name, ComposeError};

// How many numbers stood as a mode in the document the last check read, for the
// tests that follow a number through the whole parse.
#[cfg(test)]
thread_local! {
	pub(crate) static FINAL_NUMBERS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A mode read from text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReadMode {
	/// The permission bits.
	pub(crate) bits: u32,
	/// The spelling was an integer with no leading zero, which docker compose reads
	/// as a decimal number.
	pub(crate) bare: bool,
}

const MAX_MODE: u32 = 0o7777;

fn in_range(bits: u32, text: &str) -> Result<u32, String> {
	if bits > MAX_MODE {
		Err(format!(
			"{text:?} is above 0o7777; a mode is at most four octal digits"
		))
	} else {
		Ok(bits)
	}
}

/// Read `text` as a mode, or say why it is not one.
pub(crate) fn parse_mode_text(text: &str) -> Result<ReadMode, String> {
	let t = text.trim();
	if t.is_empty() {
		return Err("it is empty".to_string());
	}
	let shown = |t: &str| crate::error::sanitize_name(t).into_owned();
	if let Some(digits) = t.strip_prefix("0o").or_else(|| t.strip_prefix("0O")) {
		return u32::from_str_radix(digits, 8)
			.ok()
			.filter(|_| !digits.is_empty() && !digits.starts_with(['+', '-']))
			.ok_or_else(|| format!("{:?} has a digit that is not octal after `0o`", shown(t)))
			.and_then(|bits| in_range(bits, &shown(t)))
			.map(|bits| ReadMode { bits, bare: false });
	}
	if let Some(digits) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
		return u32::from_str_radix(digits, 16)
			.ok()
			.filter(|_| !digits.is_empty() && !digits.starts_with(['+', '-']))
			.ok_or_else(|| {
				format!(
					"{:?} has a digit that is not hexadecimal after `0x`",
					shown(t)
				)
			})
			.and_then(|bits| in_range(bits, &shown(t)))
			.map(|bits| ReadMode { bits, bare: false });
	}
	if !t.chars().all(|c| c.is_ascii_digit()) {
		return Err(format!(
			"{:?} is not a number: write an octal such as 0644, \"0644\" or 0o644",
			shown(t)
		));
	}
	if t.len() > 1 && t.starts_with('0') {
		return u32::from_str_radix(t, 8)
			.map_err(|_| {
				format!(
					"{:?} has a digit that is not octal after the leading 0",
					shown(t)
				)
			})
			.and_then(|bits| in_range(bits, &shown(t)))
			.map(|bits| ReadMode { bits, bare: false });
	}
	let bits = u32::from_str_radix(t, 8)
		.or_else(|_| t.parse::<u32>())
		.map_err(|_| format!("{:?} is too large for a mode", shown(t)))?;
	Ok(ReadMode {
		bits: in_range(bits, &shown(t))?,
		bare: true,
	})
}

// The notices a reading has given, each once. A reading only notes them: the
// commands that apply a mode or check the file say them (`emit_pending`), and the
// others (`ps`, `logs`, `down`…) never do, so a project that writes a mode as an
// integer is not told about it by every command.
static PENDING: std::sync::Mutex<std::collections::BTreeSet<String>> =
	std::sync::Mutex::new(std::collections::BTreeSet::new());

/// Note a notice to be said by the command, once however many times it is given.
pub(crate) fn note(message: String) {
	PENDING
		.lock()
		.unwrap_or_else(std::sync::PoisonError::into_inner)
		.insert(message);
}

/// The notices noted since the last call, in order, each once.
pub(crate) fn take_pending() -> Vec<String> {
	std::mem::take(
		&mut *PENDING
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner),
	)
	.into_iter()
	.collect()
}

// Set while a typed value that podup itself wrote is read back (the merge of an
// `extends` and the removal of a nested reset write a service out and read it
// again): there a number is the resolved permission bits, not a spelling that was
// lost, and is read as such.
thread_local! {
	static RESOLVED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Run `read`, which reads back what podup serialised, with its numbers taken as
/// permission bits.
pub(crate) fn reading_resolved<R>(read: impl FnOnce() -> R) -> R {
	let before = RESOLVED.with(|c| c.replace(true));
	let out = read();
	RESOLVED.with(|c| c.set(before));
	out
}

/// A resolved number is the bits it says, when they are a mode.
pub(crate) fn read_resolved_number(n: u32) -> Result<u32, String> {
	in_range(n, &n.to_string())
}

pub(crate) fn is_resolved() -> bool {
	RESOLVED.with(std::cell::Cell::get)
}

/// A mode that reached the typed reader as a number whose spelling is not known
/// (the YAML reader decoded `0o644` to 420 and the text was not recovered). It has
/// two readings: its digits as octal, as an integer typed with no leading zero is
/// read, and the number itself, as a decoded `0o` or `0x` literal is. When they
/// are the same mode it is that mode; when they differ it is refused, because
/// applying either would be a guess.
pub(crate) fn read_unknown_number(n: u32) -> Result<u32, String> {
	let as_digits = u32::from_str_radix(&n.to_string(), 8).ok();
	let bits = match as_digits {
		Some(octal) if octal != n => {
			return Err(format!(
				"mode {n} reached the reader as a number whose spelling is not known: its digits as \
				 octal are 0{octal:o}, and as the number it is 0{n:o}. Write the mode you mean as a \
				 quoted octal string such as \"0{octal:o}\" or \"0{n:o}\", or as 0o{octal:o} or 0o{n:o}"
			));
		}
		_ => n,
	};
	in_range(bits, &n.to_string())
}

/// The sentence for a mode written as an integer with no leading zero: what was
/// applied, and how to write the mode so that it does not depend on the rule.
pub(crate) fn bare_notice(text: &str, applied: u32) -> String {
	format!(
		"mode {text} is written without a leading 0 and was read as octal digits: 0{applied:o} was \
		 applied. Write the mode as a quoted octal string such as \"0{applied:o}\", or as 0o{applied:o}, \
		 so that it does not depend on that rule"
	)
}

// How many numbers without a spelling were read because both readings agree, and
// how many integers with no leading zero were noticed, for the tests that check
// that no number is read without one of the two.
#[cfg(test)]
thread_local! {
	static NOTICES: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

#[cfg(test)]
pub(crate) fn count_agreed_number() {
	NOTICES.with(|c| c.set((c.get().0 + 1, c.get().1)));
}

#[cfg(test)]
pub(crate) fn count_bare_notice() {
	NOTICES.with(|c| c.set((c.get().0, c.get().1 + 1)));
}

/// `(agreed numbers, bare strings)` since the last call.
#[cfg(test)]
pub(crate) fn take_notices() -> (usize, usize) {
	NOTICES.with(|c| c.replace((0, 0)))
}

/// Refuse a `mode:` that is not one, naming the service, the secret or config and
/// the value; a number that is still a number, whose spelling is not known, is read
/// only when both of its readings are the same mode. Runs on the document before
/// it is typed, wherever the document came from.
pub(crate) fn check_modes(value: &Value) -> crate::error::Result<()> {
	#[cfg(test)]
	FINAL_NUMBERS.with(|c| c.set(0));
	for section in ["services", "jobs"] {
		let Some(entries) = value.get(section).and_then(Value::as_mapping) else {
			continue;
		};
		for (name, service) in entries {
			let service_name = name.as_str().unwrap_or("?");
			check_service(service, service_name)?;
		}
	}
	Ok(())
}

fn check_service(service: &Value, service_name: &str) -> crate::error::Result<()> {
	let who = sanitize_name(service_name);
	for kind in ["secrets", "configs"] {
		let Some(list) = service.get(kind).and_then(Value::as_sequence) else {
			continue;
		};
		for entry in list {
			if let Some(mode) = entry.get("mode") {
				let source = entry.get("source").and_then(Value::as_str).unwrap_or("?");
				check_one(
					mode,
					&format!("service '{who}': {kind} '{}'", sanitize_name(source)),
				)?;
			}
		}
	}
	if let Some(list) = service.get("volumes").and_then(Value::as_sequence) {
		for entry in list {
			if let Some(mode) = entry.get("tmpfs").and_then(|t| t.get("mode")) {
				let target = entry.get("target").and_then(Value::as_str).unwrap_or("?");
				check_one(
					mode,
					&format!("service '{who}': tmpfs '{}'", sanitize_name(target)),
				)?;
			}
		}
	}
	Ok(())
}

fn check_one(mode: &Value, place: &str) -> crate::error::Result<()> {
	match mode {
		Value::String(text) => {
			if let Err(why) = parse_mode_text(text) {
				return Err(ComposeError::Unsupported(format!(
					"{place}: invalid mode {:?}: {why}",
					sanitize_name(text)
				)));
			}
		}
		Value::Number(n) => {
			#[cfg(test)]
			FINAL_NUMBERS.with(|c| c.set(c.get() + 1));
			let text = n.to_string();
			let numeric = text.parse::<u32>().map_err(|_| text.clone());
			let verdict = match numeric {
				Ok(n) => read_unknown_number(n),
				Err(shown) => Err(format!("{shown:?} is not a mode")),
			};
			if let Err(why) = verdict {
				return Err(ComposeError::Unsupported(format!("{place}: {why}")));
			}
		}
		_ => {}
	}
	Ok(())
}

#[cfg(test)]
#[path = "mode_tests.rs"]
mod tests;
