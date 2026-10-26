//! Reading an archive that came OUT of a container: tar extraction, the
//! host-side destination guard it asks first, and the streaming bridge that
//! feeds it from a response body.
//!
//! The writing half, what builds the archive that goes IN, lives in
//! [`super::pack`].

pub(crate) mod archive;
pub(in crate::engine::copy) mod destination;
pub(in crate::engine::copy) mod stream;

#[cfg(test)]
mod destination_tests;

#[cfg(all(test, unix))]
mod hostile_support;

#[cfg(all(test, unix))]
mod hostile_tests;

#[cfg(all(test, unix))]
mod hostile_header_tests;

#[cfg(test)]
mod destination_trusted_tests;
