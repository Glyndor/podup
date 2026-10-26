//! Where a service gets its content from: the image build and the
//! `include:`/`extends:` directives, environment variables and files, and
//! volume, secret and config mounts.

pub mod build;
pub mod env;
pub mod volume;
