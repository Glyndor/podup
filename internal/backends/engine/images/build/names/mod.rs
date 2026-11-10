//! Image names: how a built image is named, normalised and retagged.
//!
//! [`tags`] picks the primary tag and classifies the context,
//! [`normalize`] restores the docker.io canonical form the docker-compat build
//! handler used to apply, and [`extra_tags`] attaches every remaining
//! `build.tags` alias to the image the build produced.

mod extra_tags;
mod normalize;
mod remote;
mod service_contexts;
mod tags;

pub(in crate::backends::engine::images::build) use remote::fold_dockerfile_folder;
pub(in crate::backends::engine::images::build) use service_contexts::resolve_additional_contexts;
pub(crate) use service_contexts::with_context_builds;
pub(in crate::backends::engine) use tags::is_remote_context;
pub(in crate::backends::engine::images::build) use tags::looks_like_secret;
pub(crate) use tags::primary_build_tag;
