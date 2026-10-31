//! Image-level operations: commit/export of a service container to an image or
//! tar archive, and resolving compose `image:` references to pinned digests.

mod digests;
mod export;

pub use digests::resolve_image_digests;
pub use export::CommitOptions;
