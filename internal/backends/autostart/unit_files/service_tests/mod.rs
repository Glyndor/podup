//! The unit tests of `service.rs`, one file per subject.
//!
//! The suite was a single `service_tests.rs` over the 300-line soft limit, so
//! it became a folder: this module keeps the shared fixture and the index,
//! and each leaf holds the scenarios of one subject.

use super::*;
use std::path::PathBuf;

/// The options every scenario starts from.
fn opts_single() -> ServiceUnitOpts {
	ServiceUnitOpts {
		exe: PathBuf::from("/usr/local/bin/podup"),
		compose_files: vec![PathBuf::from("/srv/app/docker-compose.yml")],
		project: "app".to_string(),
		working_dir: PathBuf::from("/srv/app"),
		profiles: Vec::new(),
		env_files: Vec::new(),
		max_stop_grace_secs: None,
		socket: None,
		project_directory: None,
	}
}

mod arguments;
mod auto_update;
mod quoting;
mod stop_timeout;
mod unit;
mod validation;
