//! JSON serialisation helpers.

use crate::error::{ComposeError, Result};

/// Serialise `v` to a compact JSON string for embedding into a libpod query
/// parameter or NDJSON row.
///
/// Six sites flow through this: five in [`build`](super::build) (`cachefrom`,
/// `buildargs`, `labels`, `secrets`, `cacheto`) and one in
/// [`events`](super::events) (the `--format json` event row). Each of them
/// used to call `serde_json::to_string(...).unwrap_or_default()`, which
/// silently emitted `""` on a serialisation failure; the empty string was
/// then placed verbatim into the query parameter, where libpod treats it as
/// "no value provided", so the user's args were ignored and the build ran
/// with image defaults. The events side dropped the row and corrupted the
/// NDJSON stream a parser was reading line-by-line (#1366).
///
/// `what` names the offending field in the error so the operator sees which
/// value was rejected.
pub(crate) fn to_query_json<T: serde::Serialize>(what: &str, v: &T) -> Result<String> {
	serde_json::to_string(v).map_err(|e| ComposeError::Build(format!("invalid {what}: {e}")))
}

/// Serialise `v` to a pretty-printed JSON string for emitting as the final
/// `--format json` output of a list command.
///
/// Five sites flow through this: `ls` ([`projects::list_projects_filtered`]),
/// `images`
/// ([`Engine::images_with_services`](crate::engine::Engine::images_with_services)),
/// `top`
/// ([`Engine::top_with_options`](crate::engine::Engine::top_with_options)),
/// `ps`
/// ([`Engine::ps_filtered_with_display`](crate::engine::Engine::ps_filtered_with_display)),
/// and `volumes`
/// ([`Engine::list_volumes_with_display`](crate::engine::Engine::list_volumes_with_display)).
/// Each used to call `serde_json::to_string_pretty(...).unwrap_or_default()`,
/// which silently emitted an empty string on a serialisation failure; the
/// command then printed the empty string and exited 0, so a script consuming
/// `podup <cmd> --format json` received an empty document indistinguishable
/// from "no results" (#1444). Unlike the NDJSON path in
/// [`to_query_json`](self)/[`events`](super::events), where one row can be
/// dropped and the stream continue, `--format json` is the *whole* output,
/// so a failure must propagate as an error and exit non-zero.
///
/// `what` names the offending field in the error so the operator sees which
/// row or document type was rejected.
pub(crate) fn to_pretty_json<T: serde::Serialize>(what: &str, v: &T) -> Result<String> {
	serde_json::to_string_pretty(v).map_err(|e| ComposeError::Build(format!("invalid {what}: {e}")))
}

#[cfg(test)]
mod to_query_json_tests;

#[cfg(test)]
mod to_pretty_json_tests;
