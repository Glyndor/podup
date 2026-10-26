//! What runs as a container: the container spec, its Podman configuration, the pod around it, readiness checks and the facts a compose file declares about it.

pub(crate) mod container;
pub(in crate::backends::engine) mod container_config;
pub(in crate::backends::engine) mod declared;
pub(in crate::backends::engine) mod pod;
pub(in crate::backends::engine) mod readiness;
