//! The rule: a serialisation error must be propagated, not turned into a
//! value.
//!
//! `serde_json::to_string(&value).unwrap_or_default()` answers "this value
//! cannot be serialised" with an empty string, and an empty string is
//! indistinguishable from a value that serialised to nothing. Where the
//! serialised text is the whole output of a command, the command prints an
//! empty document and exits zero, so a script reading it sees "no results"
//! where the truth is "the output could not be built" (#1444).
//!
//! The shape is a chain, not a line: the call, then whatever is done with what
//! it returned. That is why this reads tokens and not lines. A chain split
//! over three lines by rustfmt is the same chain, and a chain that was
//! reformatted into one line is the same chain; the rule cannot be evaded by
//! running the formatter.
//!
//! What is a serialisation call: a path call to one of the encoders below, or
//! the same function imported by name. What turns the failure into a value:
//! `unwrap_or_default`, `unwrap_or` with an empty value, `unwrap_or_else` with
//! any closure, and `map_or` with an empty value. Each of them replaces the
//! error with something the caller cannot tell from a result.

use super::lexer::Token;

/// The modules whose functions serialise a value into text or bytes.
///
/// `yaml_serde` is the package name behind the `serde_yaml` alias the crate
/// imports, so both spellings are here.
const SERIALISERS: &[&str] = &["serde_json", "serde_yaml", "yaml_serde", "toml"];

/// The functions that serialise.
///
/// `to_value` is deliberately not here: it answers with a value, not with
/// text, and its default is a null rather than an empty document. That is a
/// different failure with a different symptom, and a rule that reached it
/// would be a rule about something else.
const CALLS: &[&str] = &[
	"to_string",
	"to_string_pretty",
	"to_vec",
	"to_writer",
	"to_writer_pretty",
];

/// One shape the rule forbids, and where it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
	/// The line the serialisation call is on.
	pub line: usize,
	/// The method that threw the failure away.
	pub shape: String,
}

/// Every forbidden shape in `tokens`.
pub fn findings(tokens: &[Token]) -> Vec<Finding> {
	let imported = imported_calls(tokens);
	let mut out = Vec::new();
	for index in 0..tokens.len() {
		let Some(open) = serialisation_call(tokens, index, &imported) else {
			continue;
		};
		let Some(close) = matching_bracket(tokens, open) else {
			continue;
		};
		for (method, args) in chain(tokens, close + 1) {
			if is_swallow(&method, &args) {
				out.push(Finding {
					line: tokens[index].line,
					shape: method,
				});
			}
		}
	}
	out.extend(stored_results(tokens, &imported));
	out.sort_by_key(|f| f.line);
	out.dedup();
	out
}

/// Whether the token at `index` opens a serialisation call, and where its
/// argument list starts.
fn serialisation_call(tokens: &[Token], index: usize, imported: &[String]) -> Option<usize> {
	// A path call: `serde_json::to_string(…)`.
	if index + 3 < tokens.len()
		&& SERIALISERS.contains(&tokens[index].text.as_str())
		&& tokens[index + 1].text == "::"
		&& CALLS.contains(&tokens[index + 2].text.as_str())
		&& tokens[index + 3].text == "("
	{
		return Some(index + 3);
	}
	// An imported call: `use serde_json::to_string;` then `to_string(…)`. The
	// name after a dot is a method of something else, never this.
	if index + 1 < tokens.len()
		&& CALLS.contains(&tokens[index].text.as_str())
		&& imported.iter().any(|name| name == &tokens[index].text)
		&& tokens[index + 1].text == "("
		&& (index == 0 || tokens[index - 1].text != ".")
	{
		return Some(index + 1);
	}
	None
}

/// The serialisation functions this file imported by name.
fn imported_calls(tokens: &[Token]) -> Vec<String> {
	let mut out = Vec::new();
	let mut index = 0;
	while index < tokens.len() {
		if tokens[index].text != "use" {
			index += 1;
			continue;
		}
		let end = tokens[index..]
			.iter()
			.position(|t| t.text == ";")
			.map(|offset| index + offset)
			.unwrap_or(tokens.len());
		let statement = &tokens[index..end];
		let mut cursor = 0;
		while cursor + 2 < statement.len() {
			if SERIALISERS.contains(&statement[cursor].text.as_str())
				&& statement[cursor + 1].text == "::"
			{
				if statement[cursor + 2].text == "{" {
					let mut inner = cursor + 3;
					let mut depth = 1;
					while inner < statement.len() && depth > 0 {
						match statement[inner].text.as_str() {
							"{" => depth += 1,
							"}" => depth -= 1,
							_ => {
								if depth == 1 && CALLS.contains(&statement[inner].text.as_str()) {
									out.push(statement[inner].text.clone());
								}
							}
						}
						inner += 1;
					}
					cursor = inner;
					continue;
				}
				if CALLS.contains(&statement[cursor + 2].text.as_str()) {
					out.push(statement[cursor + 2].text.clone());
				}
			}
			cursor += 1;
		}
		index = end + 1;
	}
	out
}

/// The methods called on the value the serialisation call returned, each with
/// the tokens of its arguments.
fn chain(tokens: &[Token], mut index: usize) -> Vec<(String, Vec<Token>)> {
	let mut out = Vec::new();
	while index + 2 < tokens.len() {
		if tokens[index].text != "." || tokens[index + 2].text != "(" {
			break;
		}
		let method = tokens[index + 1].text.clone();
		let Some(close) = matching_bracket(tokens, index + 2) else {
			break;
		};
		out.push((method, tokens[index + 3..close].to_vec()));
		index = close + 1;
	}
	out
}

/// Whether the method throws the failure away and replaces it with a value.
fn is_swallow(method: &str, args: &[Token]) -> bool {
	match method {
		// The default of a string or a byte vector is the empty one, so this
		// is the empty document with a different spelling.
		"unwrap_or_default" => true,
		// A closure replaces the failure with whatever it returns. Whether
		// that is empty or a message, the caller cannot tell it from a
		// result, which is the whole point of the rule.
		"unwrap_or_else" => true,
		"unwrap_or" => is_empty_value(args),
		// `map_or` takes the default first and the mapping second.
		"map_or" => {
			let first = args
				.iter()
				.position(|t| t.text == ",")
				.map(|at| &args[..at])
				.unwrap_or(args);
			is_empty_value(first)
		}
		_ => false,
	}
}

/// Whether the tokens are a value that is empty.
fn is_empty_value(tokens: &[Token]) -> bool {
	let text: String = tokens.iter().map(|t| t.text.as_str()).collect();
	matches!(
		text.as_str(),
		"None"
			| "String::new()"
			| "String::default()"
			| "Vec::new()"
			| "Vec::default()"
			| "Default::default()"
			| "\"\"" | "\"\".into()"
			| "\"\".to_string()"
			| "\"\".to_owned()"
			| "\"\".clone()"
			| "String::from(\"\")"
			| "vec![]"
	)
}

/// The result of a serialisation kept in a name and thrown away later.
///
/// `let text = to_string(&v);` on one line and `text.unwrap_or_default()` on
/// another is the same defect as the chain that does both at once, and the
/// chain is not the only way to write it.
fn stored_results(tokens: &[Token], imported: &[String]) -> Vec<Finding> {
	let mut out = Vec::new();
	let mut tainted: Vec<(String, usize)> = Vec::new();
	for statement in statements(tokens) {
		// A name that holds the result of a serialisation.
		if let Some((name, line)) = bound_serialisation(&statement, imported) {
			tainted.push((name, line));
		}
		// A tainted name whose failure is thrown away.
		for (index, token) in statement.iter().enumerate() {
			let Some((_, line)) = tainted.iter().find(|(n, _)| n == &token.text) else {
				continue;
			};
			if index + 3 >= statement.len()
				|| statement[index + 1].text != "."
				|| statement[index + 3].text != "("
			{
				continue;
			}
			let method = statement[index + 2].text.as_str();
			let Some(close) = matching_bracket(&statement, index + 3) else {
				continue;
			};
			if is_swallow(method, &statement[index + 4..close]) {
				out.push(Finding {
					line: *line,
					shape: method.to_string(),
				});
			}
		}
	}
	out
}

/// The statements of the token stream, split at the semicolons.
fn statements(tokens: &[Token]) -> Vec<Vec<Token>> {
	let mut out = Vec::new();
	let mut current = Vec::new();
	for token in tokens {
		if token.text == ";" {
			out.push(std::mem::take(&mut current));
			continue;
		}
		current.push(token.clone());
	}
	out.push(current);
	out
}

/// The name a statement binds to the result of a serialisation.
fn bound_serialisation(statement: &[Token], imported: &[String]) -> Option<(String, usize)> {
	let mut index = 0;
	while index < statement.len() {
		if statement[index].text != "let" {
			index += 1;
			continue;
		}
		let mut name_index = index + 1;
		if statement.get(name_index).map(|t| t.text.as_str()) == Some("mut") {
			name_index += 1;
		}
		let name = statement.get(name_index)?;
		if statement.get(name_index + 1).map(|t| t.text.as_str()) != Some("=") {
			index += 1;
			continue;
		}
		let holds = statement[name_index + 2..]
			.iter()
			.enumerate()
			.any(|(offset, _)| {
				serialisation_call(statement, name_index + 2 + offset, imported).is_some()
			});
		if holds {
			return Some((name.text.clone(), name.line));
		}
		index = name_index;
	}
	None
}

/// The index of the bracket that closes the one at `open`.
fn matching_bracket(tokens: &[Token], open: usize) -> Option<usize> {
	let mut depth = 0;
	for (index, token) in tokens.iter().enumerate().skip(open) {
		match token.text.as_str() {
			"(" | "[" | "{" => depth += 1,
			")" | "]" | "}" => {
				depth -= 1;
				if depth == 0 {
					return Some(index);
				}
			}
			_ => {}
		}
	}
	None
}
