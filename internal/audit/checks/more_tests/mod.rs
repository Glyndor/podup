//! Shared by the per-check tests: the sensitive-bind child module and the
//! `report_for` helper. The positive/negative pairs for each check live next to
//! the check they cover, in `check_fns/` (`limits_tests.rs`, `namespaces_tests.rs`,
//! `privileges_tests.rs`, `root_filesystem_tests.rs`, `secrets_tests.rs`, `image_tests.rs`,
//! `ports_tests.rs`).

use super::tests::report_for;

// The `sensitive_bind_mount` rows live in their own file to keep this one
// under the line limit. A child module, so `report_for` and the imports
// above reach it through `super::`.
mod sensitive_bind;
