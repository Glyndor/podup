//! The project name as a variable of the file being loaded.

use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
	/// The name the project being loaded on this thread runs under, or `None`
	/// outside a [`Scope`] that has one.
	static EXPOSED: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// RAII scope that exposes a project name as `COMPOSE_PROJECT_NAME`.
///
/// While it is open, every variable map built on this thread (the `-f` files,
/// what they `include:` and `extends:`, the service `env_file:` entries and the
/// diagnostic pass) carries the name, over any value the environment has. It
/// puts back what it found when dropped, so nested loads leave the outer one as
/// they found it.
pub(crate) struct Scope {
	previous: Option<String>,
}

impl Scope {
	/// Expose `name`, or nothing when it is `None`.
	pub(crate) fn set(name: Option<String>) -> Self {
		Scope {
			previous: EXPOSED.with(|c| c.replace(name)),
		}
	}
}

impl Drop for Scope {
	fn drop(&mut self) {
		EXPOSED.with(|c| *c.borrow_mut() = self.previous.take());
	}
}

/// Add the exposed name to `vars`, if a scope has one.
pub(super) fn apply(vars: &mut HashMap<String, String>) {
	if let Some(name) = EXPOSED.with(|c| c.borrow().clone()) {
		vars.insert("COMPOSE_PROJECT_NAME".to_string(), name);
	}
}
