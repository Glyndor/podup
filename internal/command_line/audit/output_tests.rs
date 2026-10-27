use std::io::{self, Write};

use super::*;

struct FailingWriter {
	remaining: usize,
	kind: io::ErrorKind,
}

impl Write for FailingWriter {
	fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
		if self.remaining == 0 {
			return Err(io::Error::from(self.kind));
		}
		let count = bytes.len().min(self.remaining);
		self.remaining -= count;
		Ok(count)
	}

	fn flush(&mut self) -> io::Result<()> {
		Ok(())
	}
}

#[test]
fn audit_stdout_renderers_return_io_results() {
	// Pin the stdout boundary too: a writer-aware renderer is insufficient
	// if its stdout wrapper discards the error or panics on it.
	let _: fn(&[AuditedContainer], &AuditReport) -> io::Result<()> = render_table;
	let _: fn(&AuditReport) -> io::Result<()> = render_json;
	let _: fn() -> io::Result<()> = render_list_checks_table;
	let _: fn() -> io::Result<()> = render_list_checks_json;
}

#[test]
fn all_audit_outputs_propagate_write_errors_and_silence_only_broken_pipe() {
	let file = podup::parse_str("services:\n  web:\n    image: alpine\n").unwrap();
	let containers = audited_containers(&file);
	let report = audit_file(&file, &[]);
	for kind in [io::ErrorKind::BrokenPipe, io::ErrorKind::PermissionDenied] {
		for remaining in [0, 7] {
			for output in 0..5 {
				let mut writer = FailingWriter { remaining, kind };
				let result = match output {
					0 => render_table_to(&mut writer, &containers, &report),
					1 => render_table_to(&mut writer, &[], &AuditReport::default()),
					2 => render_json_to(&mut writer, &report),
					3 => render_list_checks_table_to(&mut writer),
					_ => render_list_checks_json_to(&mut writer),
				};
				assert_eq!(result.as_ref().unwrap_err().kind(), kind);
				match output_complete(result) {
					Ok(complete) => assert!(!complete && kind == io::ErrorKind::BrokenPipe),
					Err(podup::ComposeError::Io(e)) => {
						assert_eq!(e.kind(), io::ErrorKind::PermissionDenied)
					}
					other => panic!("unexpected output result: {other:?}"),
				}
			}
		}
	}
	assert!(output_complete(Ok(())).unwrap());
}
