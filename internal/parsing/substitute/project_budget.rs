use std::cell::Cell;

thread_local! {
	/// Bytes of substituted output spent so far by the project being loaded, or
	/// `None` outside a [`Scope`], where each file counts alone.
	static PROJECT_SPENT: Cell<Option<usize>> = const { Cell::new(None) };
}

/// RAII scope for the interpolation budget of one project.
///
/// A public entry point that loads a project opens one. While it is open, every
/// file that load reads on this thread (the `-f` files, what they `include:` and
/// what they `extends:`) draws on one [`super::MAX_INTERP_DOCUMENT_BYTES`]
/// instead of getting that much each. Those internal steps do not open a scope;
/// they share the one their entry point opened.
///
/// Opening a scope always starts a fresh budget and puts back whatever was there
/// when it is dropped, on `Err` and on unwind too. So a project loaded from a
/// synchronous callback in the middle of another load (a `tracing` subscriber,
/// say) counts for itself and leaves the outer count as it found it.
/// A scope puts back the state it found, so scopes must be dropped in the reverse
/// of the order they were opened; every use here is a local guard that does.
pub(crate) struct Scope {
	previous: Option<usize>,
}

impl Scope {
	/// A fresh budget for a project loaded from here.
	pub(crate) fn open() -> Self {
		Self::replace(Some(0))
	}

	/// No project budget: each file counts alone, as for `parse_str`. Used for
	/// the diagnostic pass that re-reads input files after a load.
	pub(crate) fn none() -> Self {
		Self::replace(None)
	}

	fn replace(next: Option<usize>) -> Self {
		Scope {
			previous: PROJECT_SPENT.with(|c| c.replace(next)),
		}
	}
}

impl Drop for Scope {
	fn drop(&mut self) {
		PROJECT_SPENT.with(|c| c.set(self.previous));
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
