//! The Quadlet keys the floor's Podman reads.
//!
//! The allowlists and their tests live in their own folder because production never
//! reads them: only the test infrastructure does. The module path the rest of
//! the crate reaches them by is unchanged.

pub(crate) mod min_podman;
