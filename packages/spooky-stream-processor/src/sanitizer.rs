use nom::{
    IResult,
    branch::alt,
    bytes::complete::{is_not, tag, take_while1},
    character::complete::{char, digit1, multispace1, none_of},
    combinator::{map, opt, recognize, value, verify},
    multi::many0,
    sequence::{delimited, pair, preceded, tuple},
};
use simd_json::{json, OwnedValue as Value, StaticNode as Static}; // Import StaticNode as Static
use simd_json::prelude::*; 

// ... Parsers (Unchanged) ...
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
    let allowed_chars = |c: char| (c.is_alphanumeric() || c == '_' || c == ':' || c == '⟨' || c == '⟩') && c != ';';
    map(take_while1(allowed_chars), |s: &str| {
        if is_keyword(s) { Token::Keyword(s.to_string()) } else { Token::Identifier(s.to_string()) }
    })(input)
}

fn is_keyword(s: &str) -> bool {
    let keywords = ["SELECT", "CREATE", "UPDATE", "DELETE", "RELATE", "FROM", "WHERE", "CONTENT", "SET", "RETURN", "TIMEOUT", "PARALLEL", "LIMIT", "START", "GROUP", "ORDER", "BY", "ASC", "DESC", "INSIDE", "CONTAINS", "NONE", "NULL", "TRUE", "FALSE", "AND", "OR", "NOT", "INFO", "DB", "NS"];
    keywords.iter().any(|k| k.eq_ignore_ascii_case(s))
}

fn parse_number(input: &str) -> IResult<&str, Token> {
    map(recognize(tuple((opt(char('-')), digit1, opt(tuple((char('.'), digit1)))))), |s: &str| Token::Number(s.to_string()))(input)
}

fn parse_symbol(input: &str) -> IResult<&str, Token> {
    let safe_symbols = "=,()[]{}<>!+-*/";
    map(verify(none_of(" \t\r\n;"), |c| safe_symbols.contains(*c)), Token::Symbol)(input)
}

fn parse_backtick_lit(input: &str) -> IResult<&str, Token> {
    delimited(char('`'), recognize(many0(is_not("`"))), char('`'))(input)
    .map(|(rem, s)| (rem, Token::BacktickLit(s.to_string())))
}

fn parse_safe_query(input: &str) -> IResult<&str, Vec<Token>> {
    let (remainder, tokens) = many0(preceded(
        many0(alt((parse_comment, value((), multispace1)))),
        alt((parse_string_lit, parse_backtick_lit, parse_number, parse_word, parse_symbol, parse_ws)),
    ))(input)?;
    Ok((remainder, tokens))
}

fn rebuild_query(tokens: Vec<Token>) -> String {
    let mut out = String::new();
    let mut needs_space = false;
    let mut iter = tokens.iter().peekable();
    while let Some(token) = iter.next() {
        match token {
            Token::Keyword(s) => {
                let mut text = s.as_str(); let mut captured_colon = false;
                if text.ends_with(':') { text = &text[0..text.len() - 1]; captured_colon = true; }
                let next_is_colon = matches!(iter.peek(), Some(Token::Symbol(':')));
                let is_key = captured_colon || next_is_colon;
                if needs_space { out.push(' '); }
                if is_key { out.push('"'); out.push_str(text); out.push('"'); if captured_colon { out.push(':'); needs_space = true; } }
                else { out.push_str(text); }
                if !captured_colon { needs_space = true; }
            }
            Token::Identifier(s) => {
                let mut text = s.as_str(); let mut captured_colon = false;
                if text.ends_with(':') { text = &text[0..text.len() - 1]; captured_colon = true; }
                if captured_colon {
                    if let Some(Token::BacktickLit(inner)) = iter.peek() {
                         iter.next(); if needs_space { out.push(' '); }
                         out.push('"'); out.push_str(s); out.push_str(inner); out.push('"');
                         needs_space = true; continue; 
                    }
                }
                let next_is_colon = matches!(iter.peek(), Some(Token::Symbol(':')));
                let is_key = captured_colon || next_is_colon;
                if needs_space { out.push(' '); }
                if is_key { out.push('"'); out.push_str(text); out.push('"'); if captured_colon { out.push(':'); needs_space = true; } }
                else { if text.contains(':') { out.push('"'); out.push_str(text); out.push('"'); } else { out.push_str(text); } }
                if !captured_colon { needs_space = true; }
            }
            Token::Number(s) => { if needs_space { out.push(' '); } out.push_str(s); needs_space = true; }
            Token::StringLit(s) => { if needs_space { out.push(' '); } let content = &s[1..s.len() - 1]; out.push('"'); out.push_str(content); out.push('"'); needs_space = true; }
            Token::BacktickLit(s) => { if needs_space { out.push(' '); } out.push('"'); out.push_str(s); out.push('"'); needs_space = true; }
            Token::Symbol(c) => { if needs_space && c != &',' && c != &')' && c != &']' && c != &':' { out.push(' '); } out.push(*c); if c == &',' || c == &':' { needs_space = true; } else if c == &'(' || c == &'[' { needs_space = false; } else { needs_space = true; } }
            Token::Whitespace => { needs_space = true; }
        }
    }
    out.trim().to_string()
}

pub fn sanitize_query(raw_input: &str) -> Result<String, String> {
    match parse_safe_query(raw_input) {
        Ok((remainder, tokens)) => { if tokens.is_empty() { if !raw_input.trim().is_empty() && !remainder.contains(";") && !raw_input.trim().starts_with("--") {} } Ok(rebuild_query(tokens)) }
        Err(e) => Err(format!("Parsing Error: {}", e)),
    }
}

pub fn fix_surql_json(s: &str) -> String { sanitize_query(s).unwrap_or_else(|_| String::new()) }

pub fn normalize_record(record: Value) -> Value {
    match record {
        Value::String(s) => {
            if (s.starts_with('{') && s.ends_with('}')) || (s.starts_with('[') && s.ends_with(']')) {
                let mut bytes = s.clone().into_bytes();
                if let Ok(parsed) = simd_json::to_owned_value(&mut bytes) { return normalize_record(parsed); }
            }
            Value::String(s)
        }
        Value::Object(map) => {
            // map is Box<Map>
            if map.len() == 2 && map.contains_key("tb") && map.contains_key("id") {
                let tb = map.get("tb");
                let id = map.get("id");
                let tb_str_opt = match tb { Some(Value::String(s)) => Some(s.clone()), _ => None };
                if let (Some(tb_str), Some(id_val)) = (tb_str_opt, id) {
                    let id_str = match id_val {
                        Value::String(s) => s.clone(),
                        Value::Static(Static::I64(n)) => n.to_string(),
                        Value::Static(Static::U64(n)) => n.to_string(),
                        Value::Static(Static::F64(n)) => n.to_string(),
                        Value::Static(Static::Bool(b)) => b.to_string(),
                        Value::Static(Static::Null) => "null".to_string(),
                         _ => format!("{:?}", id_val) 
                    };
                    return Value::String(format!("{}:{}", tb_str, id_str));
                }
            }
            // Use json!({}) to construct safe base object
            let mut new_val = json!({});
            // Deref map box to iterate
            for (k, v) in *map {
                if let Some(mut_map) = new_val.as_object_mut() {
                    mut_map.insert(k, normalize_record(v));
                }
            }
            new_val
        }
        Value::Array(arr) => Value::Array(arr.into_iter().map(normalize_record).collect::<Vec<_>>().into()),
        _ => record,
    }
}

pub fn parse_params(params: Value) -> Option<Value> {
    let s = match params { Value::String(s) => fix_surql_json(&s), _ => params.to_string() };
    let mut bytes = s.into_bytes();
    if let Ok(val) = simd_json::to_owned_value(&mut bytes) { return Some(val); }
    None
}
