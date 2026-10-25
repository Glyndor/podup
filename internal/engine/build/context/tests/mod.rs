//! Unit tests for the build-context tar assembly and `.dockerignore`
//! matching in [`super`] (split out to keep the module under the source
//! line limit).

use super::{build_context_tar, build_context_tar_with_inline, map_additional_context};
use crate::engine::context::ignore_patterns::read_patterns;
use std::fs;
use std::path::Path;
use tempfile::tempdir;

mod additional_context;
mod build_secrets;
mod context_tar;
#[cfg(unix)]
mod hostile;
mod inline_tar;
mod read_patterns;
