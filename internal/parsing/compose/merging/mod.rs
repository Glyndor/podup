//! Combining several sources into one compose file: `include:`, `extends:`,
//! YAML merge keys, the `!override`/`!reset` tags, and anchoring the paths of
//! imported content.

pub(super) mod alias_budget;
pub(super) mod anchor;
pub(super) mod extends;
pub(super) mod include;
pub(super) mod merge;
pub(super) mod tags;
pub(super) mod textual;
