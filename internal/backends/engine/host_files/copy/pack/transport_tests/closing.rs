// --- closing the archive --------------------------------------------------

fn refused(text: &str) -> crate::error::ComposeError {
	crate::error::ComposeError::Build(text.to_string())
}

/// The error that names the refused entry is kept when closing the archive then
/// fails because the receiver has gone: closing reports the receiver, which
/// would replace it, and a copy with an entry missing would be confirmed.
#[test]
fn a_pack_error_is_kept_when_closing_the_archive_fails_too() {
	let dropped = std::sync::atomic::AtomicBool::new(false);
	let (_, outcome) = super::conclude(
		Vec::new(),
		Err(refused("cp: source entry z.fifo has unverified kind")),
		false,
		Err(refused("cp: flush: receiver dropped")),
		&dropped,
	);
	let error = outcome.unwrap_err().to_string();
	assert!(error.contains("z.fifo"), "{error}");
	assert!(
		!dropped.load(std::sync::atomic::Ordering::Relaxed),
		"a refused entry was marked as only the receiver having gone"
	);
}

/// A failure to close an archive that packed fine is the receiver, and is the
/// timing of a response that came first.
#[test]
fn a_failure_to_close_after_a_clean_pack_is_only_the_receiver() {
	let dropped = std::sync::atomic::AtomicBool::new(false);
	let (_, outcome) = super::conclude(
		Vec::new(),
		Ok(()),
		false,
		Err(refused("cp: flush: receiver dropped")),
		&dropped,
	);
	assert!(outcome.is_err());
	assert!(dropped.load(std::sync::atomic::Ordering::Relaxed));
}

/// An error of the pack that came after the writer lost its receiver is its
/// consequence; one that came before is not.
#[test]
fn a_pack_error_is_only_the_receiver_when_the_writer_lost_it_first() {
	for (lost_first, expected) in [(true, true), (false, false)] {
		let dropped = std::sync::atomic::AtomicBool::new(false);
		let (_, outcome) = super::conclude(
			Vec::new(),
			Err(refused("cp: some pack error")),
			lost_first,
			Ok(()),
			&dropped,
		);
		assert!(outcome.is_err());
		assert_eq!(
			dropped.load(std::sync::atomic::Ordering::Relaxed),
			expected,
			"lost_first={lost_first}"
		);
	}
	let dropped = std::sync::atomic::AtomicBool::new(false);
	let (_, outcome) = super::conclude(Vec::new(), Ok(()), false, Ok(()), &dropped);
	assert!(outcome.is_ok());
}
