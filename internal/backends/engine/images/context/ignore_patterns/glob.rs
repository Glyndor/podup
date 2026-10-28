//! Character-aware glob syntax for classes, escapes and Unicode filenames.
//! Uses the same bounded state-set algorithm as the ASCII wildcard matcher.

enum Token {
	Literal(char),
	Any,
	Star,
	DoubleStar,
	Class {
		negated: bool,
		ranges: Vec<(char, char)>,
	},
}

/// Match UTF-8 glob tokens without recursive wildcard backtracking.
pub(super) fn matches(pattern: &[u8], path: &[u8]) -> bool {
	let (Ok(pattern), Ok(path)) = (std::str::from_utf8(pattern), std::str::from_utf8(path)) else {
		return false;
	};
	let Some(tokens) = tokens(pattern) else {
		return false;
	};
	let mut current = vec![false; tokens.len() + 1];
	let mut stay = vec![false; tokens.len() + 1];
	current[0] = true;
	close(&tokens, &mut current, &stay);
	for ch in path.chars() {
		let mut next = vec![false; tokens.len() + 1];
		stay.fill(false);
		for (i, token) in tokens.iter().enumerate() {
			if !current[i] {
				continue;
			}
			match token {
				Token::Star => stay[i] = ch != '/',
				Token::DoubleStar => stay[i] = true,
				Token::Any => next[i + 1] |= ch != '/',
				Token::Literal(lit) => next[i + 1] |= *lit == ch,
				Token::Class { negated, ranges } => {
					let in_class = ranges.iter().any(|(lo, hi)| *lo <= ch && ch <= *hi);
					next[i + 1] |= ch != '/' && in_class != *negated;
				}
			}
		}
		close(&tokens, &mut next, &stay);
		current = next;
	}
	current[tokens.len()]
}

fn close(tokens: &[Token], positions: &mut [bool], stay: &[bool]) {
	for (i, token) in tokens.iter().enumerate() {
		let entered = positions[i];
		if !entered && !stay[i] {
			continue;
		}
		positions[i] = true;
		match token {
			Token::Star => positions[i + 1] = true,
			Token::DoubleStar => {
				positions[i + 1] = true;
				if entered && matches!(tokens.get(i + 1), Some(Token::Literal('/'))) {
					positions[i + 2] = true;
				}
			}
			_ => {}
		}
	}
}

fn tokens(pattern: &str) -> Option<Vec<Token>> {
	let chars: Vec<char> = pattern.chars().collect();
	let mut tokens = Vec::new();
	let mut i = 0;
	while i < chars.len() {
		let token = match chars[i] {
			'\\' => {
				i += 1;
				Token::Literal(*chars.get(i)?)
			}
			'*' if chars.get(i + 1) == Some(&'*') => {
				i += 1;
				Token::DoubleStar
			}
			'*' => Token::Star,
			'?' => Token::Any,
			'[' => class(&chars, &mut i)?,
			lit => Token::Literal(lit),
		};
		tokens.push(token);
		i += 1;
	}
	Some(tokens)
}

fn class(chars: &[char], i: &mut usize) -> Option<Token> {
	*i += 1;
	let negated = chars.get(*i) == Some(&'^');
	if negated {
		*i += 1;
	}
	let mut ranges = Vec::new();
	while *chars.get(*i)? != ']' {
		let lo = class_char(chars, i)?;
		let hi = if chars.get(*i) == Some(&'-') {
			*i += 1;
			class_char(chars, i)?
		} else {
			lo
		};
		if hi < lo {
			return None;
		}
		ranges.push((lo, hi));
	}
	if ranges.is_empty() {
		return None;
	}
	Some(Token::Class { negated, ranges })
}

fn class_char(chars: &[char], i: &mut usize) -> Option<char> {
	let mut ch = *chars.get(*i)?;
	if ch == ']' {
		return None;
	}
	if ch == '\\' {
		*i += 1;
		ch = *chars.get(*i)?;
	}
	*i += 1;
	Some(ch)
}
