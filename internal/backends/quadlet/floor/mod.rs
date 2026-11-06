//! The minimum Podman release each Quadlet key we can emit first appeared in.
//!
//! The table and its tests live in their own folder because production never
//! reads them: only the test infrastructure does. The module path the rest of
//! the crate reaches them by is unchanged.

pub(crate) mod min_podman;
