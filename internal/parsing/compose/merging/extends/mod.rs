//! `extends:` directive: inheritance and field merging between service definitions.
//!
//! Services can extend another service within the same file or from an external
//! compose file referenced by path. Resolution is recursive (chains are supported)
//! and cycle detection uses a visited set to error early.
//!
//! Merge semantics: scalar fields from the child win; collection fields
//! (env vars, labels, vectors) are merged with the child taking precedence on
//! overlapping keys. See [`merge_service`] for full field-by-field rules.

mod merge;
mod resolution;

pub(in crate::parsing::compose) use merge::{merge_service, merge_service_tagged};
#[cfg(test)]
use resolution::{parse_file_inner_call_count, reset_parse_file_inner_counter};
pub(in super::super) use resolution::{resolve_all_extends, resolve_extends_same_file};

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "parse_count_tests.rs"]
mod parse_count_tests;
#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
