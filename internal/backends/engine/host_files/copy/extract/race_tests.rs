//! Deterministic filesystem changes at the archive reader boundary.

use std::io::{self, Read};
use std::os::unix::fs::{symlink, PermissionsExt};

use super::archive::extract_tar_guarded;
use super::hostile_support::{archive, file};

struct SwapReader<'a, F> {
	bytes: &'a [u8],
	served: usize,
	at: usize,
	swap: Option<F>,
}

impl<F: FnOnce()> Read for SwapReader<'_, F> {
	fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
		if self.served >= self.at {
			if let Some(swap) = self.swap.take() {
				swap();
			}
		}
		let n = self.bytes.read(out)?;
		self.served += n;
		Ok(n)
	}
}

#[cfg(unix)]
#[test]
fn destination_swap_before_first_header_cannot_redirect_extraction() {
	let tmp = tempfile::tempdir().unwrap();
	let dst = tmp.path().join("dest");
	let pinned = tmp.path().join("original");
	let outside = tmp.path().join("outside");
	std::fs::create_dir(&dst).unwrap();
	std::fs::create_dir(&outside).unwrap();
	let bytes = archive(&[file("result", b"copied", 0o644)]);
	let reader = SwapReader {
		bytes: &bytes,
		served: 0,
		at: 0,
		swap: Some(|| {
			std::fs::rename(&dst, &pinned).unwrap();
			symlink(&outside, &dst).unwrap();
		}),
	};
	extract_tar_guarded(reader, &dst).unwrap();
	assert!(
		!outside.join("result").exists(),
		"destination was re-resolved"
	);
	assert_eq!(std::fs::read(pinned.join("result")).unwrap(), b"copied");
}

#[cfg(unix)]
#[test]
fn file_swap_during_body_read_cannot_redirect_chmod() {
	let tmp = tempfile::tempdir().unwrap();
	let dst = tmp.path().join("dest");
	std::fs::create_dir(&dst).unwrap();
	let outside = tmp.path().join("outside");
	std::fs::write(&outside, b"private").unwrap();
	std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o600)).unwrap();
	let bytes = archive(&[file("result", b"copied", 0o644)]);
	let reader = SwapReader {
		bytes: &bytes,
		served: 0,
		at: 512,
		swap: Some(|| {
			std::fs::rename(dst.join("result"), dst.join("original")).unwrap();
			symlink(&outside, dst.join("result")).unwrap();
		}),
	};
	extract_tar_guarded(reader, &dst).unwrap();
	assert_eq!(std::fs::read(&outside).unwrap(), b"private");
	assert_eq!(
		std::fs::metadata(&outside).unwrap().permissions().mode() & 0o777,
		0o600
	);
	assert_eq!(std::fs::read(dst.join("original")).unwrap(), b"copied");
}
