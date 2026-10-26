//! What the container is: its name, the image it runs, and its metadata.

mod identity;
mod image;
mod metadata;

pub(crate) use identity::container_name;
pub(super) use identity::emit_identity;
pub(super) use image::{emit_image, emit_pull};
pub(super) use metadata::{emit_annotations, emit_labels, emit_sysctls};
