//! What runs as a container: the container spec, its Podman configuration, the pod around it, readiness checks and the facts a compose file declares about it.

pub(crate) mod container;
pub(in crate::engine) mod container_config;
pub(in crate::engine) mod declared;
pub(in crate::engine) mod pod;
pub(in crate::engine) mod readiness;
