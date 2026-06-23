//! Parse tests for features present in the type system but not previously
//! covered. Split into part files to keep each under the source line limit.

#[path = "coverage/deploy_build.rs"]
mod deploy_build;
#[path = "coverage/network_volume.rs"]
mod network_volume;
#[path = "coverage/resources_secrets.rs"]
mod resources_secrets;
