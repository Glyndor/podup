//! Per-shape coverage for the `port_published_on_all_interfaces` check.
//!
//! The check shares its predicate with the parse-time port-exposure warning
//! in `internal/compose/diagnostics/port_exposure.rs`, so every case below
//! is a mirror of one in `port_exposure_tests.rs`: the audit fires iff the
//! warning fires, and vice versa, so the two surfaces cannot drift on a
//! future compose-shape addition (#1835).

use super::super::tests::report_for;

mod aggregation;
mod long_form;
mod short_form;
