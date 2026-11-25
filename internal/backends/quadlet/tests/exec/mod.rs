//! Tests for the ExecStart line a unit gets: escaping of systemd specifiers, the generator, and the podman argv helpers.

pub(in crate::backends::quadlet::tests) mod exec_escaping;
pub(in crate::backends::quadlet::tests) mod exec_generator;
pub(in crate::backends::quadlet::tests) mod hostile_native_values;
pub(in crate::backends::quadlet::tests) mod native_key_exactness;
pub(in crate::backends::quadlet::tests) mod native_key_values;
pub(in crate::backends::quadlet::tests) mod podman_argv;
pub(in crate::backends::quadlet::tests) mod unsetenv;
