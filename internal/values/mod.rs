//! Scalar value formats: port mappings, memory and CPU sizes, RFC 3339
//! timestamps, and the human-readable rendering of sizes and durations.

/// Port-mapping parser for the docker-compose `ports:` format variants.
pub mod ports;
/// Memory and CPU value parsers shared by the engine and tests.
pub mod size;
pub(crate) mod timestamp;
pub(crate) mod units;
