//! Reading what the user supplies from disk: dotenv syntax, `env_file:` loading,
//! and the size-bounded file read they and the compose parser share.

pub(crate) mod dotenv;
/// `env_file:` loading: KEY=VALUE pairs from a service's declared files.
pub mod env_file;
pub(crate) mod filesystem;
