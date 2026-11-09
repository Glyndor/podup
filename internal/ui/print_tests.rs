use super::*;

use std::io;

/// A sink that fails every write with the error kind it was given, so a test
/// can hand the writer a closed pipe without closing a real one.
struct Failing {
	kind: io::ErrorKind,
}

impl Write for Failing {
	fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
		Err(io::Error::new(self.kind, "closed"))
	}

	fn flush(&mut self) -> io::Result<()> {
		Ok(())
	}
}

/// A sink that keeps what it is given.
#[derive(Default)]
struct Collecting {
	written: Vec<u8>,
}

impl Write for Collecting {
	fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
		self.written.extend_from_slice(buf);
		Ok(buf.len())
	}

	fn flush(&mut self) -> io::Result<()> {
		Ok(())
	}
}

#[test]
fn a_line_reaches_the_sink_with_its_newline() {
	let mut sink = Collecting::default();
	write_to(
		&mut sink,
		format_args!("podup: wrote {}", "unit.service"),
		true,
	)
	.expect("write");
	assert_eq!(
		String::from_utf8_lossy(&sink.written),
		"podup: wrote unit.service\n"
	);
}

#[test]
fn a_text_without_a_newline_reaches_the_sink_unchanged() {
	let mut sink = Collecting::default();
	write_to(&mut sink, format_args!("podup: wrote"), false).expect("write");
	assert_eq!(String::from_utf8_lossy(&sink.written), "podup: wrote");
}

#[test]
fn a_closed_pipe_is_reported_as_a_broken_pipe() {
	// The caller turns this into the quiet end of the process, so the error has
	// to arrive with the kind it will look for.
	let mut sink = Failing {
		kind: io::ErrorKind::BrokenPipe,
	};
	let err = write_to(&mut sink, format_args!("podup"), true).expect_err("write fails");
	assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
}

#[test]
fn any_other_write_failure_keeps_its_kind() {
	let mut sink = Failing {
		kind: io::ErrorKind::StorageFull,
	};
	let err = write_to(&mut sink, format_args!("podup"), true).expect_err("write fails");
	assert_eq!(err.kind(), io::ErrorKind::StorageFull);
}

#[test]
fn a_closed_pipe_is_recognised_as_the_reader_being_gone() {
	assert!(reader_is_gone(&io::Error::new(
		io::ErrorKind::BrokenPipe,
		"closed"
	)));
	assert!(!reader_is_gone(&io::Error::new(
		io::ErrorKind::StorageFull,
		"full"
	)));
}

#[test]
fn any_other_failure_is_handed_back_to_the_caller() {
	let err = settle(Err(io::Error::new(io::ErrorKind::StorageFull, "full")))
		.expect_err("the caller sees the failure");
	assert_eq!(err.kind(), io::ErrorKind::StorageFull);
}
