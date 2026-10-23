//! Log frames on an output writer.

/// Write one log frame without creating a lossy `Cow` for valid UTF-8.
///
/// `String::from_utf8_lossy` allocates a `Cow<String>` even on the happy path;
/// `std::str::from_utf8` returns `&str` directly with no allocation. The
/// lossy path is the rare one (an arbitrary byte stream from `run`/`exec`/
/// hook output), so a single shared helper keeps the two call sites
/// (`Engine::run` and `run_lifecycle_hook`) honest and matches the previous
/// observable output (#1369).
pub(crate) fn write_frame<W: std::io::Write>(out: &mut W, bytes: &[u8]) -> std::io::Result<()> {
	match std::str::from_utf8(bytes) {
		Ok(text) => out.write_all(text.as_bytes()),
		Err(_) => out.write_all(String::from_utf8_lossy(bytes).as_bytes()),
	}
}
