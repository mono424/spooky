use nom::{
    IResult,
    branch::alt,
    bytes::complete::{is_not, tag, take_while1},
    character::complete::{char, digit1, multispace1, none_of},
    combinator::{map, opt, recognize, value, verify},
    multi::many0,
    sequence::{delimited, pair, preceded, tuple},
};
use serde_json::Value;

// =============================================================================
// TEIL 1: DIE INTERNE NOM LOGIK
// =============================================================================

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Keyword(String),
    Identifier(String),
    StringLit(String),
    BacktickLit(String),
    Number(String),
    Symbol(char),
    Whitespace,
}

fn parse_ws(input: &str) -> IResult<&str, Token> {
    value(Token::Whitespace, multispace1)(input)
}

fn parse_comment(input: &str) -> IResult<&str, ()> {
    alt((
        value((), pair(tag("--"), is_not("\n\r"))),
        value((), pair(tag("//"), is_not("\n\r"))),
        value((), tuple((tag("/*"), take_while1(|c| c != '*'), tag("*/")))),
    ))(input)
}

fn parse_string_lit(input: &str) -> IResult<&str, Token> {
    let parse_single = delimited(
        char('\''),
        recognize(many0(alt((tag("\\'"), is_not("'\\"))))),
        char('\''),
    );
    let parse_double = delimited(
        char('"'),
        recognize(many0(alt((tag("\\\""), is_not("\"\\"))))),
        char('"'),
    );
    map(alt((parse_single, parse_double)), |s: &str| {
        Token::StringLit(format!("'{}'", s))
    })(input)
}

fn parse_word(input: &str) -> IResult<&str, Token> {
    // FIX: Semikolon darf NIEMALS Teil eines Wortes sein!
    let allowed_chars =
        |c: char| (c.is_alphanumeric() || c == '_' || c == ':' || c == '⟨' || c == '⟩') && c != ';';

    map(take_while1(allowed_chars), |s: &str| {
        if is_keyword(s) {
            Token::Keyword(s.to_string())
        } else {
            Token::Identifier(s.to_string())
        }
    })(input)
}

fn is_keyword(s: &str) -> bool {
    let keywords = [
        "SELECT", "CREATE", "UPDATE", "DELETE", "RELATE", "FROM", "WHERE", "CONTENT", "SET",
        "RETURN", "TIMEOUT", "PARALLEL", "LIMIT", "START", "GROUP", "ORDER", "BY", "ASC", "DESC",
        "INSIDE", "CONTAINS", "NONE", "NULL", "TRUE", "FALSE", "AND", "OR", "NOT", "INFO", "DB",
        "NS",
    ];
    keywords.iter().any(|k| k.eq_ignore_ascii_case(s))
}

fn parse_number(input: &str) -> IResult<&str, Token> {
    map(
        recognize(tuple((
            opt(char('-')),
            digit1,
            opt(tuple((char('.'), digit1))),
        ))),
        |s: &str| Token::Number(s.to_string()),
    )(input)
}

fn parse_symbol(input: &str) -> IResult<&str, Token> {
    let safe_symbols = "=,()[]{}<>!+-*/";
    map(
        verify(none_of(" \t\r\n;"), |c| safe_symbols.contains(*c)),
        Token::Symbol,
    )(input)
}


fn parse_backtick_lit(input: &str) -> IResult<&str, Token> {
    delimited(
        char('`'),
        recognize(many0(is_not("`"))),
        char('`'),
    )(input)
    .map(|(rem, s)| (rem, Token::BacktickLit(s.to_string())))
}

fn parse_safe_query(input: &str) -> IResult<&str, Vec<Token>> {
    // Die Logik hier ist: Wir parsen Tokens, solange wir KEIN Semikolon sehen.
    let (remainder, tokens) = many0(preceded(
        many0(alt((parse_comment, value((), multispace1)))),
        alt((
            parse_string_lit,
            parse_backtick_lit, // Add support for `...`
            parse_number,
            parse_word,
            parse_symbol,
            parse_ws,
        )),
    ))(input)?;

    Ok((remainder, tokens))
}

// In src/sanitizer.rs

fn rebuild_query(tokens: Vec<Token>) -> String {
    let mut out = String::new();
    let mut needs_space = false;

    let mut iter = tokens.iter().peekable();

    while let Some(token) = iter.next() {
        match token {
            Token::Keyword(s) => {
                let mut text = s.as_str();
                let mut captured_colon = false;
                if text.ends_with(':') {
                    text = &text[0..text.len() - 1];
                    captured_colon = true;
                }

                let next_is_colon = matches!(iter.peek(), Some(Token::Symbol(':')));
                let is_key = captured_colon || next_is_colon;

                if needs_space {
                    out.push(' ');
                }

                if is_key {
                    out.push('"');
                    out.push_str(text);
                    out.push('"');
                    if captured_colon {
                        out.push(':');
                        needs_space = true;
                    }
                } else {
                    out.push_str(text);
                }

                if !captured_colon {
                    needs_space = true;
                }
            }

            Token::Identifier(s) => {
                let mut text = s.as_str();
                let mut captured_colon = false;

                if text.ends_with(':') {
                    text = &text[0..text.len() - 1];
                    captured_colon = true;
                }
                
                // MERGE LOGIC for Complex IDs: thread:`foo`
                // If Ident ends with colon AND next is BacktickLit -> Merge to quoted string "thread:foo"
                if captured_colon {
                    if let Some(Token::BacktickLit(inner)) = iter.peek() {
                         iter.next(); // Consume backtick token
                         
                         if needs_space { out.push(' '); }
                         out.push('"');
                         out.push_str(s); // "thread:"
                         out.push_str(inner); // "foo"
                         out.push('"');
                         needs_space = true;
                         continue; 
                    }
                }

                let next_is_colon = matches!(iter.peek(), Some(Token::Symbol(':')));
                let is_key = captured_colon || next_is_colon;

                if needs_space {
                    out.push(' ');
                }

                if is_key {
                    out.push('"');
                    out.push_str(text);
                    out.push('"');
                    if captured_colon {
                        out.push(':');
                        needs_space = true;
                    }
                } else {
                    if text.contains(':') {
                        out.push('"');
                        out.push_str(text);
                        out.push('"');
                    } else {
                        out.push_str(text);
                    }
                }

                if !captured_colon {
                    needs_space = true;
                }
            }

            Token::Number(s) => {
                if needs_space { out.push(' '); }
                out.push_str(s);
                needs_space = true;
            }
            Token::StringLit(s) => {
                if needs_space { out.push(' '); }
                let content = &s[1..s.len() - 1];
                out.push('"');
                out.push_str(content);
                out.push('"');
                needs_space = true;
            }
            Token::BacktickLit(s) => {
                // Standalone backtick -> quoted string
                if needs_space { out.push(' '); }
                out.push('"');
                out.push_str(s);
                out.push('"');
                needs_space = true;
            }
            Token::Symbol(c) => {
                if needs_space && c != &',' && c != &')' && c != &']' && c != &':' {
                    out.push(' ');
                }
                out.push(*c);
                if c == &',' || c == &':' {
                    needs_space = true;
                } else if c == &'(' || c == &'[' {
                    needs_space = false;
                } else {
                    needs_space = true;
                }
            }
            Token::Whitespace => {
                needs_space = true;
            }
        }
    }
    out.trim().to_string()
}

// =============================================================================
// TEIL 2: DIE PUBLIC API
// =============================================================================

pub fn sanitize_query(raw_input: &str) -> Result<String, String> {
    match parse_safe_query(raw_input) {
        Ok((remainder, tokens)) => {
            if tokens.is_empty() {
                // Leerer Input oder nur Kommentare/Müll
                if !raw_input.trim().is_empty()
                    && !remainder.contains(";")
                    && !raw_input.trim().starts_with("--")
                {
                    // Strict Mode könnte hier Fehler werfen
                }
            }
            // Wir ignorieren den remainder (alles ab dem Semikolon)
            Ok(rebuild_query(tokens))
        }
        Err(e) => Err(format!("Parsing Error: {}", e)),
    }
}

// -----------------------------------------------------------------------------
// LEGACY BRIDGE
// -----------------------------------------------------------------------------

pub fn fix_surql_json(s: &str) -> String {
    sanitize_query(s).unwrap_or_else(|_| String::new())
}

pub fn normalize_record(record: Value) -> Value {
    match record {
        Value::String(s) => {
            if (s.starts_with('{') && s.ends_with('}')) || (s.starts_with('[') && s.ends_with(']'))
            {
                if let Ok(parsed) = serde_json::from_str::<Value>(&s) {
                    return normalize_record(parsed);
                }
            }
            Value::String(s)
        }
        Value::Object(map) => {
            if map.len() == 2 && map.contains_key("tb") && map.contains_key("id") {
                let tb = map.get("tb").and_then(|v| v.as_str());
                let id = map.get("id");
                if let (Some(tb_str), Some(id_val)) = (tb, id) {
                    let id_str = match id_val {
                        Value::String(s) => s.clone(),
                        Value::Number(n) => n.to_string(),
                        _ => id_val.to_string(),
                    };
                    return Value::String(format!("{}:{}", tb_str, id_str));
                }
            }
            let mut new_map = serde_json::Map::new();
            for (k, v) in map {
                new_map.insert(k, normalize_record(v));
            }
            Value::Object(new_map)
        }
        Value::Array(arr) => Value::Array(arr.into_iter().map(normalize_record).collect()),
        _ => record,
    }
}

pub fn parse_params(params: Value) -> Option<Value> {
    let s = match params {
        Value::String(s) => fix_surql_json(&s),
        _ => params.to_string(),
    };
    if let Ok(val) = serde_json::from_str(&s) {
        return Some(val);
    }
    None
}
