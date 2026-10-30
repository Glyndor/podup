//! A bound on what a document with aliases expands to.
//!
//! `guard_alias_expansion` bounds it from the text: the number of references
//! times the size of the document. That holds for flat aliases, and misses an
//! anchor that holds aliases to another anchor, which multiplies: forty
//! references in thirteen kilobytes can replay to forty million nodes.
//!
//! Here the nodes, and the bytes of the scalars among them, are counted as the
//! parser replays them, and the count stops the parse the moment it passes
//! either limit, so the expansion is never built. Both are needed: forty
//! references to a large anchored scalar are forty nodes and still hundreds of
//! megabytes.
//! The count is only a bound: a failure that is not the limit (a tag in a shape
//! the counter does not follow, a syntax error) is left to the real parse to
//! report.

use std::cell::Cell;
use std::marker::PhantomData;

use serde::de::{
	self, DeserializeSeed, EnumAccess, IgnoredAny, MapAccess, SeqAccess, VariantAccess, Visitor,
};

use crate::error::{ComposeError, Result};

/// The most nodes (scalars, sequences, mappings) a document may expand to. A
/// large compose file is thousands, not millions.
pub(super) const MAX_EXPANDED_NODES: usize = 2_000_000;

/// The most bytes of scalars (values and keys) a document may expand to. The
/// same bound the interpolation of a whole document is held to.
pub(super) const MAX_EXPANDED_BYTES: usize = 16 * 1024 * 1024;

/// What the counter's own error says when a limit is passed.
const OVERRUN: &str = "podup: alias expansion limit";

/// Refuse a document whose aliases expand to more than `limit` nodes, or to more
/// than [`MAX_EXPANDED_BYTES`] bytes of scalars.
pub(super) fn within_limit(content: &str, limit: usize) -> Result<()> {
	within_limits(content, limit, MAX_EXPANDED_BYTES)
}

fn within_limits(content: &str, limit: usize, byte_limit: usize) -> Result<()> {
	let nodes = Cell::new(0usize);
	let bytes = Cell::new(0usize);
	for document in serde_yaml::Deserializer::from_str(content) {
		let counted = Count {
			nodes: &nodes,
			limit,
			bytes: &bytes,
			byte_limit,
		}
		.deserialize(document);
		if let Err(e) = counted {
			if e.to_string().contains(OVERRUN) {
				return Err(ComposeError::Unsupported(format!(
					"compose document uses YAML aliases that expand to more than {limit} nodes \
					 or {byte_limit} bytes; inline the repeated content instead"
				)));
			}
			// Anything else is for the real parse to report.
			return Ok(());
		}
	}
	Ok(())
}

#[derive(Clone, Copy)]
struct Count<'a> {
	nodes: &'a Cell<usize>,
	limit: usize,
	bytes: &'a Cell<usize>,
	byte_limit: usize,
}

impl Count<'_> {
	fn tick<E: de::Error>(&self) -> std::result::Result<(), E> {
		let nodes = self.nodes.get() + 1;
		self.nodes.set(nodes);
		if nodes > self.limit {
			Err(E::custom(OVERRUN))
		} else {
			Ok(())
		}
	}
}

impl<'de> DeserializeSeed<'de> for Count<'_> {
	type Value = ();

	fn deserialize<D: de::Deserializer<'de>>(
		self,
		deserializer: D,
	) -> std::result::Result<(), D::Error> {
		deserializer.deserialize_any(self)
	}
}

impl<'de> Visitor<'de> for Count<'_> {
	type Value = ();

	fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str("any YAML value")
	}

	fn visit_bool<E: de::Error>(self, _: bool) -> std::result::Result<(), E> {
		self.tick()
	}

	fn visit_i64<E: de::Error>(self, _: i64) -> std::result::Result<(), E> {
		self.tick()
	}

	fn visit_u64<E: de::Error>(self, _: u64) -> std::result::Result<(), E> {
		self.tick()
	}

	fn visit_f64<E: de::Error>(self, _: f64) -> std::result::Result<(), E> {
		self.tick()
	}

	fn visit_str<E: de::Error>(self, text: &str) -> std::result::Result<(), E> {
		let bytes = self.bytes.get().saturating_add(text.len());
		self.bytes.set(bytes);
		if bytes > self.byte_limit {
			return Err(E::custom(OVERRUN));
		}
		self.tick()
	}

	fn visit_unit<E: de::Error>(self) -> std::result::Result<(), E> {
		self.tick()
	}

	fn visit_none<E: de::Error>(self) -> std::result::Result<(), E> {
		self.tick()
	}

	fn visit_some<D: de::Deserializer<'de>>(
		self,
		deserializer: D,
	) -> std::result::Result<(), D::Error> {
		self.tick()?;
		deserializer.deserialize_any(self)
	}

	fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<(), A::Error> {
		self.tick()?;
		while seq.next_element_seed(self)?.is_some() {}
		Ok(())
	}

	fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<(), A::Error> {
		self.tick()?;
		while map.next_key_seed(self)?.is_some() {
			map.next_value_seed(self)?;
		}
		Ok(())
	}

	fn visit_enum<A: EnumAccess<'de>>(self, data: A) -> std::result::Result<(), A::Error> {
		self.tick()?;
		let (_, variant) = data.variant_seed(PhantomData::<IgnoredAny>)?;
		variant.newtype_variant_seed(self)
	}
}

#[cfg(test)]
#[path = "alias_budget_tests.rs"]
mod tests;
