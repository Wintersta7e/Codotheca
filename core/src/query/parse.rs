//! The Rust parser, the second consumer of §8.3's one production.

use super::ast::{
    Cmp, HasAttribute, IgnoredReason, IgnoredTerm, IsFlag, QueryAst, QueryTerm, TextField,
};
use super::QUERY_GRAMMAR_VERSION;

/// §8.3: quoting is `"…"` with `\"` as the only escape. Whitespace inside quotes does not split.
fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut escaped = false;
    for ch in input.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_quotes => {
                current.push(ch);
                escaped = true;
            }
            '"' => {
                in_quotes = !in_quotes;
                current.push(ch);
            }
            c if c.is_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn unquote(value: &str) -> (String, bool) {
    let mut chars = value.chars();
    if chars.next() == Some('"') {
        if value.chars().count() >= 2 && value.ends_with('"') {
            let inner: String = chars.clone().take(value.chars().count() - 2).collect();
            return (inner.replace("\\\"", "\""), true);
        }
        // Unterminated: take what is there rather than dropping the term silently.
        return (chars.collect::<String>().replace("\\\"", "\""), true);
    }
    (value.to_owned(), false)
}

/// A prefix that reads as a field name: two or more letters. One letter is a drive letter.
fn is_field_shaped(prefix: &str) -> bool {
    prefix.chars().count() >= 2 && prefix.chars().all(|c| c.is_ascii_alphabetic())
}

fn parse_is(value: &str) -> Option<IsFlag> {
    Some(match value {
        "dirty" => IsFlag::Dirty,
        "unpushed" => IsFlag::Unpushed,
        "behind" => IsFlag::Behind,
        "interrupted" => IsFlag::Interrupted,
        "archived" => IsFlag::Archived,
        "pinned" => IsFlag::Pinned,
        "hidden" => IsFlag::Hidden,
        "reference" => IsFlag::Reference,
        "bare" => IsFlag::Bare,
        "fork" => IsFlag::Fork,
        "empty" => IsFlag::Empty,
        "shallow" => IsFlag::Shallow,
        "local" => IsFlag::Local,
        "wsl" => IsFlag::Wsl,
        "new" => IsFlag::New,
        _ => return None,
    })
}

fn parse_has(value: &str) -> Option<HasAttribute> {
    Some(match value {
        "readme" => HasAttribute::Readme,
        "license" => HasAttribute::License,
        "tests" => HasAttribute::Tests,
        "ci" => HasAttribute::Ci,
        "remote" => HasAttribute::Remote,
        "stash" => HasAttribute::Stash,
        "submodules" => HasAttribute::Submodules,
        _ => return None,
    })
}

/// `>10mb` / `<6mo` becomes (op, digits, unit). Nothing else is a comparison.
fn split_comparison(value: &str) -> Option<(Cmp, u64, String)> {
    let mut chars = value.chars();
    let op = match chars.next()? {
        '>' => Cmp::Gt,
        '<' => Cmp::Lt,
        _ => return None,
    };
    let rest: String = chars.collect();
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let unit: String = rest.chars().skip(digits.chars().count()).collect();
    if digits.is_empty() || unit.is_empty() || !unit.chars().all(|c| c.is_ascii_lowercase()) {
        return None;
    }
    Some((op, digits.parse().ok()?, unit))
}

fn parse_field_term(field: &str, raw: &str, quoted: bool, negated: bool) -> Option<QueryTerm> {
    if raw.is_empty() {
        return None;
    }
    match field {
        "lang" | "owner" | "in" | "collection" => {
            let value = if quoted {
                raw.to_owned()
            } else {
                raw.to_lowercase()
            };
            let text_field = match field {
                "lang" => TextField::Lang,
                "owner" => TextField::Owner,
                "in" => TextField::In,
                _ => TextField::Collection,
            };
            Some(QueryTerm::Text {
                negated,
                field: text_field,
                value,
                quoted,
            })
        }
        "is" => parse_is(&raw.to_lowercase()).map(|flag| QueryTerm::Flag { negated, flag }),
        "has" => {
            parse_has(&raw.to_lowercase()).map(|attribute| QueryTerm::Has { negated, attribute })
        }
        "size" => {
            let (op, n, unit) = split_comparison(&raw.to_lowercase())?;
            let scale: u64 = match unit.as_str() {
                "kb" => 1024,
                "mb" => 1_048_576,
                "gb" => 1_073_741_824,
                _ => return None,
            };
            Some(QueryTerm::Size {
                negated,
                op,
                bytes: n.checked_mul(scale)?,
            })
        }
        "touched" => {
            let lowered = raw.to_lowercase();
            if lowered.chars().count() == 4 && lowered.chars().all(|c| c.is_ascii_digit()) {
                return lowered
                    .parse()
                    .ok()
                    .map(|year| QueryTerm::TouchedYear { negated, year });
            }
            let (op, n, unit) = split_comparison(&lowered)?;
            let scale: u64 = match unit.as_str() {
                "d" => 1,
                "w" => 7,
                "mo" => 30,
                "y" => 365,
                _ => return None,
            };
            let days = u32::try_from(n.checked_mul(scale)?).ok()?;
            Some(QueryTerm::TouchedAge { negated, op, days })
        }
        _ => None,
    }
}

const FIELDS: [&str; 9] = [
    "lang",
    "owner",
    "in",
    "is",
    "has",
    "touched",
    "size",
    "completion",
    "collection",
];

#[must_use]
pub fn parse_query(input: &str) -> QueryAst {
    let mut terms = Vec::new();
    let mut ignored = Vec::new();

    for token in tokenize(input) {
        let negated = token.starts_with('-');
        let body: String = if negated {
            token.chars().skip(1).collect()
        } else {
            token.clone()
        };
        if body.is_empty() {
            continue;
        }

        // `split_once` rather than an index, so no byte offset is ever used to slice a string
        // that may hold multi-byte characters. A leading colon is not a field.
        let split = body
            .split_once(':')
            .filter(|(prefix, _)| !prefix.is_empty());
        let Some((prefix, raw_value)) = split else {
            let (value, _) = unquote(&body);
            terms.push(QueryTerm::Bare {
                negated,
                text: value.to_lowercase(),
            });
            continue;
        };

        let field = prefix.to_lowercase();
        if !FIELDS.contains(&field.as_str()) {
            if is_field_shaped(prefix) && !raw_value.starts_with("//") {
                ignored.push(IgnoredTerm {
                    text: token,
                    reason: IgnoredReason::UnknownField,
                });
            } else {
                let (value, _) = unquote(&body);
                terms.push(QueryTerm::Bare {
                    negated,
                    text: value.to_lowercase(),
                });
            }
            continue;
        }

        if field == "completion" {
            ignored.push(IgnoredTerm {
                text: token,
                reason: IgnoredReason::NotComputed,
            });
            continue;
        }

        let (value, quoted) = unquote(raw_value);
        match parse_field_term(&field, &value, quoted, negated) {
            Some(term) => terms.push(term),
            None => ignored.push(IgnoredTerm {
                text: token,
                reason: IgnoredReason::MalformedValue,
            }),
        }
    }

    QueryAst {
        grammar_version: QUERY_GRAMMAR_VERSION,
        terms,
        ignored,
    }
}
