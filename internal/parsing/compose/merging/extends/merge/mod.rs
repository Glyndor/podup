//! Field-by-field merge of a base service into an overriding service.
//!
//! Scalar fields take the override when present, else the base. Collection
//! fields (env vars, labels, maps) are merged with the override winning on
//! overlapping keys. Sequence fields are combined per the Compose
//! Specification's `extends` rules: referenced (base) items first, then the
//! extending service's items, with duplicates removed, not replaced wholesale.

mod collections;
mod service;
mod tagged;

pub(in crate::parsing::compose) use service::merge_service;
pub(in crate::parsing::compose) use tagged::merge_service_tagged;

#[cfg(test)]
mod tests;
