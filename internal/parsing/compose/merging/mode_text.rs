//! A `mode:` keeps the text it was written as until it is typed.
//!
//! The YAML reader decodes `0o644` to the number 420 before anything else sees
//! it, and the typed reader cannot tell it from a `420` somebody typed. Each
//! spelling has its own rule, so the document is read again by a reader that
//! visits only the places a mode can be, and the number the first reading made is
//! replaced by the text of the scalar, which the YAML parser gives as it was
//! written (`deserialize_str`). The rules that read the text are in
//! `types::inputs::mode`.
//!
//! The places are the `secrets:` and `configs:` entries of a service or a job, and
//! the `tmpfs:` of its `volumes:` entries, reached through merge keys (`<<`, a
//! single mapping or a list of them), tags and aliases. Everything else in the
//! document is skipped without being read.
//!
//! Two passes, so that no node is ever read in a way it cannot be: the first finds
//! the `mode:` scalars that are numbers and where they are; the second asks for the
//! text of exactly those, which are known to be scalars. Neither pass fails on a
//! node it does not expect: it skips it. If a reading fails anyway, nothing is
//! replaced, and a number that still stands is not guessed at: it is read only
//! when its two possible readings are the same mode, and refused otherwise.

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

use serde::de::{
	DeserializeSeed, EnumAccess, IgnoredAny, MapAccess, SeqAccess, VariantAccess, Visitor,
};
use serde::Deserializer;
use serde_yaml::Value;

use crate::error::{ComposeError, Result};

/// The most `mode:` values one document may hold. A project that has more is
/// refused with a message, rather than read at a cost that grows with it.
const MAX_MODES: usize = 100_000;

/// One step of the way to a node.
enum Seg {
	Key(Cow<'static, str>),
	Idx(usize),
}

/// The step for a key. The keys the walk follows are known, and cost nothing; a
/// service's name is the one that is kept.
fn key_step(key: Option<String>) -> Seg {
	let Some(key) = key else {
		return Seg::Key(Cow::Borrowed(""));
	};
	let known = [
		"services", "jobs", "secrets", "configs", "volumes", "tmpfs", "mode", "<<",
	]
	.into_iter()
	.find(|known| *known == key);
	Seg::Key(known.map_or(Cow::Owned(key), Cow::Borrowed))
}

/// The way to a node, as a list linked to its parent: a step is one allocation
/// shared by everything below it, and the path of a number is one pointer. The
/// steps are laid out as a list only when a number is put back.
#[derive(Clone, Default)]
struct Path(Option<Rc<Link>>);

struct Link {
	seg: Seg,
	parent: Path,
}

impl Path {
	fn push(&self, seg: Seg) -> Path {
		#[cfg(test)]
		LINKS.with(|c| c.set(c.get() + 1));
		Path(Some(Rc::new(Link {
			seg,
			parent: self.clone(),
		})))
	}

	/// The steps from the root down to the node.
	fn steps(&self) -> Vec<&Seg> {
		let mut steps = Vec::new();
		let mut at = &self.0;
		while let Some(link) = at {
			steps.push(&link.seg);
			at = &link.parent.0;
		}
		steps.reverse();
		steps
	}
}

/// Where a visit is, in the shape of a compose document.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
	Root,
	/// The `services:` or `jobs:` mapping.
	Services,
	/// One service or job.
	Service,
	/// A `secrets:` or `configs:` list.
	Refs,
	/// One entry of it.
	Entry,
	/// A `volumes:` list.
	Volumes,
	/// One entry of it.
	Volume,
	/// A `tmpfs:` block.
	Tmpfs,
}

/// What a key leads to: a place to read, a `mode:` to look at, or nothing to read.
enum Next {
	Place(Place),
	Mode,
	Skip,
}

fn next(place: Place, key: &str) -> Next {
	match (place, key) {
		(Place::Root, "services" | "jobs") => Next::Place(Place::Services),
		(Place::Services, _) => Next::Place(Place::Service),
		(Place::Service, "secrets" | "configs") => Next::Place(Place::Refs),
		(Place::Service, "volumes") => Next::Place(Place::Volumes),
		(Place::Service, "<<") => Next::Place(Place::Service),
		(Place::Entry, "mode") | (Place::Tmpfs, "mode") => Next::Mode,
		(Place::Entry, "<<") => Next::Place(Place::Entry),
		(Place::Volume, "tmpfs") => Next::Place(Place::Tmpfs),
		(Place::Volume, "<<") => Next::Place(Place::Volume),
		(Place::Tmpfs, "<<") => Next::Place(Place::Tmpfs),
		_ => Next::Skip,
	}
}

/// What the items of a list at `place` are.
fn item(place: Place) -> Option<Place> {
	match place {
		Place::Refs | Place::Entry => Some(Place::Entry),
		Place::Volumes | Place::Volume => Some(Place::Volume),
		Place::Service => Some(Place::Service),
		Place::Tmpfs => Some(Place::Tmpfs),
		Place::Root | Place::Services => None,
	}
}

#[derive(Clone, Copy)]
enum Phase<'a> {
	/// Find the `mode:` scalars that are numbers.
	Find,
	/// Take the text of the scalars numbered in the set.
	Text(&'a HashSet<usize>),
}

/// One walk of the document. A number is noted with its path, which costs a
/// pointer; both lists are in the order of the walk, so of the ordinal.
struct Walk<'a> {
	phase: Phase<'a>,
	/// How many `mode:` scalars the walk has met; the same in both passes, since
	/// they walk the same places in the same order.
	ordinal: Cell<usize>,
	limit: usize,
	/// Whether the walk was stopped for having met more than `limit`.
	over: Cell<bool>,
	numbers: RefCell<Vec<(usize, Path)>>,
	texts: RefCell<Vec<(usize, String)>>,
}

impl<'a> Walk<'a> {
	fn new(phase: Phase<'a>, limit: usize) -> Self {
		Self {
			phase,
			ordinal: Cell::new(0),
			limit,
			over: Cell::new(false),
			numbers: RefCell::new(Vec::new()),
			texts: RefCell::new(Vec::new()),
		}
	}
}

struct Node<'w, 'a> {
	walk: &'w Walk<'a>,
	place: Place,
	path: Path,
}

impl<'de, 'w, 'a> DeserializeSeed<'de> for Node<'w, 'a> {
	type Value = ();
	fn deserialize<D: Deserializer<'de>>(self, d: D) -> std::result::Result<(), D::Error> {
		d.deserialize_any(self)
	}
}

/// The key of a mapping entry as text, when it is text.
struct Key;

impl<'de> DeserializeSeed<'de> for Key {
	type Value = Option<String>;
	fn deserialize<D: Deserializer<'de>>(
		self,
		d: D,
	) -> std::result::Result<Option<String>, D::Error> {
		struct V;
		impl<'de> Visitor<'de> for V {
			type Value = Option<String>;
			fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
				f.write_str("a mapping key")
			}
			fn visit_str<E>(self, s: &str) -> std::result::Result<Option<String>, E> {
				Ok(Some(s.to_string()))
			}
			fn visit_bool<E>(self, _: bool) -> std::result::Result<Option<String>, E> {
				Ok(None)
			}
			fn visit_i64<E>(self, _: i64) -> std::result::Result<Option<String>, E> {
				Ok(None)
			}
			fn visit_u64<E>(self, _: u64) -> std::result::Result<Option<String>, E> {
				Ok(None)
			}
			fn visit_f64<E>(self, _: f64) -> std::result::Result<Option<String>, E> {
				Ok(None)
			}
			fn visit_unit<E>(self) -> std::result::Result<Option<String>, E> {
				Ok(None)
			}
			fn visit_seq<A: SeqAccess<'de>>(
				self,
				mut s: A,
			) -> std::result::Result<Option<String>, A::Error> {
				while s.next_element::<IgnoredAny>()?.is_some() {}
				Ok(None)
			}
			fn visit_map<A: MapAccess<'de>>(
				self,
				mut m: A,
			) -> std::result::Result<Option<String>, A::Error> {
				while m.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
				Ok(None)
			}
			fn visit_enum<A: EnumAccess<'de>>(
				self,
				data: A,
			) -> std::result::Result<Option<String>, A::Error> {
				let (_tag, v): (IgnoredAny, _) = data.variant()?;
				v.newtype_variant::<IgnoredAny>()?;
				Ok(None)
			}
		}
		d.deserialize_any(V)
	}
}

impl<'de, 'w, 'a> Visitor<'de> for Node<'w, 'a> {
	type Value = ();

	fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		f.write_str("any YAML value")
	}

	// A scalar where a place is expected holds nothing to find.
	fn visit_bool<E>(self, _: bool) -> std::result::Result<(), E> {
		Ok(())
	}
	fn visit_i64<E>(self, _: i64) -> std::result::Result<(), E> {
		Ok(())
	}
	fn visit_u64<E>(self, _: u64) -> std::result::Result<(), E> {
		Ok(())
	}
	fn visit_f64<E>(self, _: f64) -> std::result::Result<(), E> {
		Ok(())
	}
	fn visit_str<E>(self, _: &str) -> std::result::Result<(), E> {
		Ok(())
	}
	fn visit_unit<E>(self) -> std::result::Result<(), E> {
		Ok(())
	}
	fn visit_none<E>(self) -> std::result::Result<(), E> {
		Ok(())
	}
	fn visit_some<D: Deserializer<'de>>(self, d: D) -> std::result::Result<(), D::Error> {
		self.deserialize(d)
	}

	fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<(), A::Error> {
		let Some(child) = item(self.place) else {
			while seq.next_element::<IgnoredAny>()?.is_some() {}
			return Ok(());
		};
		let mut index = 0;
		loop {
			let more = seq
				.next_element_seed(Node {
					walk: self.walk,
					place: child,
					path: self.path.push(Seg::Idx(index)),
				})?
				.is_some();
			if !more {
				return Ok(());
			}
			index += 1;
		}
	}

	fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<(), A::Error> {
		while let Some(key) = map.next_key_seed(Key)? {
			let step = key.as_deref().map_or(Next::Skip, |k| next(self.place, k));
			match step {
				Next::Skip => {
					map.next_value::<IgnoredAny>()?;
				}
				Next::Place(place) => {
					map.next_value_seed(Node {
						walk: self.walk,
						place,
						path: self.path.push(key_step(key)),
					})?;
				}
				Next::Mode => {
					map.next_value_seed(Leaf {
						walk: self.walk,
						path: self.path.push(key_step(key)),
					})?;
				}
			}
		}
		Ok(())
	}

	// A tagged node is its content, at the same place.
	fn visit_enum<A: EnumAccess<'de>>(self, data: A) -> std::result::Result<(), A::Error> {
		let (_tag, content): (IgnoredAny, _) = data.variant()?;
		content.newtype_variant_seed(self)
	}
}

/// A `mode:` value. In the first pass a number is noted with its path; in the
/// second the text of the numbers noted is taken. Whatever else stands there is
/// skipped.
struct Leaf<'w, 'a> {
	walk: &'w Walk<'a>,
	path: Path,
}

impl<'de, 'w, 'a> DeserializeSeed<'de> for Leaf<'w, 'a> {
	type Value = ();
	fn deserialize<D: Deserializer<'de>>(self, d: D) -> std::result::Result<(), D::Error> {
		let ordinal = self.walk.ordinal.get();
		if ordinal >= self.walk.limit {
			self.walk.over.set(true);
			return Err(serde::de::Error::custom("too many modes"));
		}
		self.walk.ordinal.set(ordinal + 1);
		match self.walk.phase {
			Phase::Find => d.deserialize_any(Find {
				walk: self.walk,
				ordinal,
				path: self.path,
			}),
			Phase::Text(wanted) if wanted.contains(&ordinal) => d.deserialize_str(Text {
				walk: self.walk,
				ordinal,
			}),
			Phase::Text(_) => d.deserialize_ignored_any(IgnoredAny).map(|_| ()),
		}
	}
}

struct Find<'w, 'a> {
	walk: &'w Walk<'a>,
	ordinal: usize,
	path: Path,
}

impl<'w, 'a> Find<'w, 'a> {
	fn number<E>(self) -> std::result::Result<(), E> {
		#[cfg(test)]
		PATH_NOTED.with(|c| c.set(c.get() + 1));
		self.walk
			.numbers
			.borrow_mut()
			.push((self.ordinal, self.path));
		Ok(())
	}
}

impl<'de, 'w, 'a> Visitor<'de> for Find<'w, 'a> {
	type Value = ();

	fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		f.write_str("any YAML value")
	}
	fn visit_u64<E>(self, _: u64) -> std::result::Result<(), E> {
		self.number()
	}
	fn visit_i64<E>(self, _: i64) -> std::result::Result<(), E> {
		self.number()
	}
	fn visit_i128<E>(self, _: i128) -> std::result::Result<(), E> {
		self.number()
	}
	fn visit_u128<E>(self, _: u128) -> std::result::Result<(), E> {
		self.number()
	}
	fn visit_f64<E>(self, _: f64) -> std::result::Result<(), E> {
		self.number()
	}
	fn visit_bool<E>(self, _: bool) -> std::result::Result<(), E> {
		Ok(())
	}
	fn visit_str<E>(self, _: &str) -> std::result::Result<(), E> {
		Ok(())
	}
	fn visit_unit<E>(self) -> std::result::Result<(), E> {
		Ok(())
	}
	fn visit_none<E>(self) -> std::result::Result<(), E> {
		Ok(())
	}
	fn visit_some<D: Deserializer<'de>>(self, d: D) -> std::result::Result<(), D::Error> {
		d.deserialize_any(self)
	}
	fn visit_seq<A: SeqAccess<'de>>(self, mut s: A) -> std::result::Result<(), A::Error> {
		while s.next_element::<IgnoredAny>()?.is_some() {}
		Ok(())
	}
	fn visit_map<A: MapAccess<'de>>(self, mut m: A) -> std::result::Result<(), A::Error> {
		while m.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
		Ok(())
	}
	// A tagged scalar is the scalar.
	fn visit_enum<A: EnumAccess<'de>>(self, data: A) -> std::result::Result<(), A::Error> {
		let (_tag, v): (IgnoredAny, _) = data.variant()?;
		v.newtype_variant_seed(FindAgain {
			walk: self.walk,
			ordinal: self.ordinal,
			path: self.path,
		})
	}
}

/// The content of a tag, read as the `mode:` value it is.
struct FindAgain<'w, 'a> {
	walk: &'w Walk<'a>,
	ordinal: usize,
	path: Path,
}

impl<'de, 'w, 'a> DeserializeSeed<'de> for FindAgain<'w, 'a> {
	type Value = ();
	fn deserialize<D: Deserializer<'de>>(self, d: D) -> std::result::Result<(), D::Error> {
		d.deserialize_any(Find {
			walk: self.walk,
			ordinal: self.ordinal,
			path: self.path,
		})
	}
}

/// The text of a scalar known to be one.
struct Text<'w, 'a> {
	walk: &'w Walk<'a>,
	ordinal: usize,
}

impl<'de, 'w, 'a> Visitor<'de> for Text<'w, 'a> {
	type Value = ();

	fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		f.write_str("a scalar")
	}

	fn visit_str<E: serde::de::Error>(self, text: &str) -> std::result::Result<(), E> {
		self.walk
			.texts
			.borrow_mut()
			.push((self.ordinal, text.to_string()));
		Ok(())
	}
}

/// Read `content` again and put the text of every `mode:` that the first reading
/// made a number of where the number stood. Returns how many were put back; a
/// document with more than [`MAX_MODES`] modes is refused.
pub(super) fn restore(value: &mut Value, content: &str) -> Result<usize> {
	restore_up_to(value, content, MAX_MODES)
}

fn restore_up_to(value: &mut Value, content: &str, limit: usize) -> Result<usize> {
	#[cfg(test)]
	if FORCE_FAILURE.with(Cell::get) {
		return Ok(0);
	}
	let too_many = || {
		ComposeError::Unsupported(format!(
			"the project has more than {limit} `mode:` values in its secrets, configs and tmpfs mounts; split it or write fewer"
		))
	};
	let find = Walk::new(Phase::Find, limit);
	let start = Node {
		walk: &find,
		place: Place::Root,
		path: Path::default(),
	};
	let read = start.deserialize(serde_yaml::Deserializer::from_str(content));
	if find.over.get() {
		return Err(too_many());
	}
	if read.is_err() {
		return Ok(0);
	}
	let numbers = find.numbers.into_inner();
	if numbers.is_empty() {
		return Ok(0);
	}
	let wanted: HashSet<usize> = numbers.iter().map(|(ordinal, _)| *ordinal).collect();
	let take = Walk::new(Phase::Text(&wanted), limit);
	let start = Node {
		walk: &take,
		place: Place::Root,
		path: Path::default(),
	};
	if start
		.deserialize(serde_yaml::Deserializer::from_str(content))
		.is_err()
	{
		return Ok(0);
	}
	let texts = take.texts.into_inner();
	let mut restored = 0;
	for (path, text) in pair(numbers, texts) {
		if let Some(slot @ Value::Number(_)) = at(value, &path.steps()) {
			*slot = Value::String(text);
			restored += 1;
		}
	}
	Ok(restored)
}

// For the tests: make the reading fail, and count how many paths were noted.
#[cfg(test)]
thread_local! {
	pub(super) static FORCE_FAILURE: Cell<bool> = const { Cell::new(false) };
	pub(super) static LINKS: Cell<usize> = const { Cell::new(0) };
	pub(super) static PATH_NOTED: Cell<usize> = const { Cell::new(0) };
}

/// The text that belongs to each number. Both lists are in the order of the walk,
/// so one sweep pairs them by ordinal; a number with no text, or a text with no
/// number, is left out.
fn pair<T>(numbers: Vec<(usize, T)>, texts: Vec<(usize, String)>) -> Vec<(T, String)> {
	let mut texts = texts.into_iter().peekable();
	let mut pairs = Vec::new();
	for (ordinal, item) in numbers {
		while texts.next_if(|(o, _)| *o < ordinal).is_some() {}
		if let Some((_, text)) = texts.next_if(|(o, _)| *o == ordinal) {
			pairs.push((item, text));
		}
	}
	pairs
}

/// The node at `path`, looking through tags, also at the end.
fn at<'v>(value: &'v mut Value, path: &[&Seg]) -> Option<&'v mut Value> {
	let mut node = value;
	for seg in path {
		while let Value::Tagged(tagged) = node {
			node = &mut tagged.value;
		}
		node = match (*seg, node) {
			(Seg::Key(key), Value::Mapping(map)) => map.get_mut(&**key)?,
			(Seg::Idx(i), Value::Sequence(seq)) => seq.get_mut(*i)?,
			_ => return None,
		};
	}
	while let Value::Tagged(tagged) = node {
		node = &mut tagged.value;
	}
	Some(node)
}

#[cfg(test)]
#[path = "mode_text_tests.rs"]
mod tests;
