//! Tests for the `extends` / multi-`-f` service merge.
//!
//! Split out of `merge.rs` to keep that file within the source line limit.

use crate::parse_str;

mod block_lists;
mod dedupe;
mod dependencies;
mod directives;
mod identity;
mod long_lists;
mod networks;
mod service_fields;
