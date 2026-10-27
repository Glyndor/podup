//! Log frames on an output writer.

/// Write one log frame to the output writer, byte for byte.
///
/// What a command printed is bytes, not text. A `run`/`exec` session that
/// pipes a compressed archive to stdout has to deliver that archive, and a
/// program that writes the two halves of one multi-byte character in two
/// separate writes has to deliver that character: decoding the payload on
/// the way out replaced every byte that is not valid UTF-8 with U+FFFD and
/// turned every character split across two frames into two replacement
/// characters. One shared helper keeps the call sites (`Engine::run` and
/// `run_lifecycle_hook`) honest about it.
pub(crate) fn write_frame<W: std::io::Write>(out: &mut W, bytes: &[u8]) -> std::io::Result<()> {
	out.write_all(bytes)
}

#[cfg(test)]
#[path = "log_frame_tests.rs"]
mod tests;
