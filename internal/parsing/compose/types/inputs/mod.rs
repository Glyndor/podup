//! Where a service gets its content from: the image build and the
//! `include:`/`extends:` directives, environment variables and files, and
//! volume, secret and config mounts.

pub mod build;
mod build_ssh;
pub mod env;
pub(crate) mod mode;
pub(crate) mod mode_render;
pub mod volume;
