//! Recover the owning service when YAML's generic value cannot hold an integer.

use std::collections::HashMap;

use serde::de::{IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};

use crate::error::ComposeError;
use crate::libpod::validate::spec_field_error;

#[derive(Deserialize)]
struct Document {
	services: HashMap<String, Mask>,
}

#[derive(Deserialize)]
struct Mask {
	#[serde(default, deserialize_with = "large_integer")]
	umask: Option<String>,
}

/// Only used after the ordinary YAML reader fails, without changing its types.
pub(crate) fn number_error(content: &str) -> Option<ComposeError> {
	let document: Document = serde_yaml::from_str(content).ok()?;
	for (name, mask) in document.services {
		if let Some(value) = mask.umask {
			return Some(spec_field_error(
				&name,
				"umask",
				value,
				"umask must be a quoted octal string; quote the value (only integer 0 is accepted)",
			).into());
		}
	}
	None
}

fn large_integer<'de, D: Deserializer<'de>>(de: D) -> Result<Option<String>, D::Error> {
	struct Integer;
	impl<'de> Visitor<'de> for Integer {
		type Value = Option<String>;
		fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
			f.write_str("a permission mask")
		}
		fn visit_u128<E>(self, value: u128) -> Result<Self::Value, E> {
			Ok(Some(value.to_string()))
		}
		fn visit_i128<E>(self, value: i128) -> Result<Self::Value, E> {
			Ok(Some(value.to_string()))
		}
		fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E> {
			Ok(None)
		}
		fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E> {
			Ok(None)
		}
		fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E> {
			Ok(None)
		}
		fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E> {
			Ok(None)
		}
		fn visit_str<E>(self, _: &str) -> Result<Self::Value, E> {
			Ok(None)
		}
		fn visit_unit<E>(self) -> Result<Self::Value, E> {
			Ok(None)
		}
		fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
			while seq.next_element::<IgnoredAny>()?.is_some() {}
			Ok(None)
		}
		fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
			while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
			Ok(None)
		}
	}
	de.deserialize_any(Integer)
}
