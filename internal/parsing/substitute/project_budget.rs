use std::cell::Cell;

thread_local! {
	/// Bytes of substituted output spent so far by the project being loaded, or
	/// `None` outside a [`Scope`], where each file counts alone.
	static PROJECT_SPENT: Cell<Option<usize>> = const { Cell::new(None) };
}

/// RAII scope for the interpolation budget of one project.
///
/// While it is open, every file the loader reads on this thread (the `-f`
/// files, what they `include:` and what they `extends:`) draws on one
/// [`super::MAX_INTERP_DOCUMENT_BYTES`] instead of getting that much each.
/// A nested scope joins the outer one and leaves it open.
pub(crate) struct Scope {
	owner: bool,
}

impl Scope {
	pub(crate) fn open() -> Self {
		let owner = PROJECT_SPENT.with(|c| {
			let fresh = c.get().is_none();
			if fresh {
				c.set(Some(0));
			}
			fresh
		});
		Scope { owner }
	}
}

impl Drop for Scope {
	fn drop(&mut self) {
		if self.owner {
			PROJECT_SPENT.with(|c| c.set(None));
		}
	}
}

/// What the open scope has spent so far, `0` when none is open.
pub(crate) fn spent() -> usize {
	PROJECT_SPENT.with(|c| c.get().unwrap_or(0))
}

/// Record the running total of a file that finished, if a scope is open.
pub(crate) fn record(total: usize) {
	PROJECT_SPENT.with(|c| {
		if c.get().is_some() {
			c.set(Some(total));
		}
	});
}
