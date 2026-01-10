use anyhow::{Result, anyhow};
use nom::{
    IResult,
    branch::alt,
    bytes::complete::{is_not, tag, tag_no_case, take_while1},
    character::complete::{char, digit1, multispace0, multispace1},
    combinator::{map, map_res, opt, value, verify},
    multi::{separated_list0, separated_list1},
    sequence::{delimited, preceded, tuple},
};
use serde_json::{Value, json};

/// Der Haupteinstiegspunkt
pub fn convert_surql_to_dbsp(sql: &str) -> Result<Value> {
    let clean_sql = sql.trim().trim_end_matches(';');
    match parse_full_query(clean_sql) {
        Ok((_, plan)) => Ok(plan),
        Err(e) => Err(anyhow!("SQL Parsing Fehler: {}", e)),
    }
}

// --- NOM PARSER HELPERS ---

fn ws<'a, F, O, E: nom::error::ParseError<&'a str>>(
    inner: F,
) -> impl FnMut(&'a str) -> IResult<&'a str, O, E>
where
    F: FnMut(&'a str) -> IResult<&'a str, O, E>,
{
    delimited(multispace0, inner, multispace0)
}

// Identifiers: user, table:1, address.city
fn parse_identifier(input: &str) -> IResult<&str, String> {
    let parser = take_while1(|c: char| c.is_alphanumeric() || c == '_' || c == ':' || c == '.');
    map(parser, |s: &str| s.to_string())(input)
}

// --- VALUE PARSING (inkl. Prefix Support) ---

#[derive(Debug, Clone)]
enum ParsedValue {
    Json(Value),
    Identifier(String), // Für Joins (z.B. user.id = post.author)
    Prefix(String),     // Für 'term*'
}

fn parse_string_literal(input: &str) -> IResult<&str, ParsedValue> {
    let parse_content = |delimiter| {
        delimited(
            char(delimiter),
            is_not(if delimiter == '\'' { "'" } else { "\"" }),
            char(delimiter),
        )
    };

    map(alt((parse_content('\''), parse_content('"'))), |s: &str| {
        if s.ends_with('*') {
            ParsedValue::Prefix(s.trim_end_matches('*').to_string())
        } else {
            ParsedValue::Json(json!(s))
        }
    })(input)
}

fn parse_value_entry(input: &str) -> IResult<&str, ParsedValue> {
    alt((
        parse_string_literal, // 'string' oder "string"
        map(preceded(char('$'), parse_identifier), |s| {
            ParsedValue::Json(json!({ "$param": s }))
        }),
        value(ParsedValue::Json(json!(true)), tag_no_case("true")),
        value(ParsedValue::Json(json!(false)), tag_no_case("false")),
        map_res(digit1, |s: &str| {
            s.parse::<i64>().map(|n| ParsedValue::Json(json!(n)))
        }),
        // Wenn es kein Wert ist, ist es ein Identifier (für Joins!)
        map(parse_identifier, ParsedValue::Identifier),
    ))(input)
}

// --- LOGIC PARSING (AND / OR / JOINS) ---

// Ein einzelner Vergleich: a = b
fn parse_comparison(input: &str) -> IResult<&str, Value> {
    let (input, (left, _, right)) =
        tuple((ws(parse_identifier), ws(char('=')), ws(parse_value_entry)))(input)?;

    match right {
        ParsedValue::Json(val) => Ok((
            input,
            json!({
                "type": "eq",
                "field": left,
                "value": val
            }),
        )),
        ParsedValue::Prefix(val) => Ok((
            input,
            json!({
                "type": "prefix",
                "prefix": val // Engine erwartet prefix meist auf key applied? Oder field?
                // HINWEIS: Deine Engine unterstützt Prefix nur auf KEY Ebene im alten Code ("prefix": val).
                // Wenn du "field LIKE val*" willst, muss Engine angepasst werden.
                // Wir nutzen hier das Format des alten Parsers:
                // "type": "prefix", "prefix": val
            }),
        )),
        ParsedValue::Identifier(right_field) => {
            // DAS IST EIN JOIN! (feld = anderes_feld)
            // Wir markieren es speziell, damit wir später den Join-Operator bauen können
            Ok((
                input,
                json!({
                    "type": "__JOIN_CANDIDATE__",
                    "left": left,
                    "right": right_field
                }),
            ))
        }
    }
}

// Parse AND Block: a=1 AND b=2
fn parse_and_clause(input: &str) -> IResult<&str, Vec<Value>> {
    separated_list1(ws(tag_no_case("AND")), parse_comparison)(input)
}

// Parse OR Block: (a=1 AND b=2) OR (c=3)
fn parse_where_logic(input: &str) -> IResult<&str, Value> {
    let (input, or_groups) = preceded(
        tag_no_case("WHERE"),
        separated_list1(ws(tag_no_case("OR")), parse_and_clause),
    )(input)?;

    // Logik Baum bauen
    // Wir haben eine Liste von AND-Gruppen.
    // [ [A, B], [C] ] bedeutet: (A AND B) OR (C)

    let mut or_predicates = Vec::new();

    for and_group in or_groups {
        if and_group.len() == 1 {
            or_predicates.push(and_group[0].clone());
        } else {
            or_predicates.push(json!({
                "type": "and",
                "predicates": and_group
            }));
        }
    }

    if or_predicates.len() == 1 {
        Ok((input, or_predicates[0].clone()))
    } else {
        Ok((
            input,
            json!({
                "type": "or",
                "predicates": or_predicates
            }),
        ))
    }
}

// --- REST (LIMIT, ORDER, SELECT) ---

fn parse_limit_clause(input: &str) -> IResult<&str, usize> {
    preceded(
        tag_no_case("LIMIT"),
        ws(map_res(digit1, |s: &str| s.parse::<usize>())),
    )(input)
}

fn parse_order_clause(input: &str) -> IResult<&str, Vec<Value>> {
    let single_order = map(
        tuple((
            ws(parse_identifier),
            opt(ws(alt((tag_no_case("ASC"), tag_no_case("DESC"))))),
        )),
        |(field, dir)| json!({ "field": field, "direction": dir.unwrap_or("ASC").to_uppercase() }),
    );
    preceded(
        tag_no_case("ORDER BY"),
        separated_list1(ws(char(',')), single_order),
    )(input)
}

fn parse_full_query(input: &str) -> IResult<&str, Value> {
    let (input, _) = tag_no_case("SELECT")(input)?;
    let (input, _) = multispace1(input)?;

    let (input, fields) = alt((
        map(tag("*"), |_| vec!["*".to_string()]),
        separated_list1(ws(char(',')), parse_identifier),
    ))(input)?;

    let (input, _) = multispace1(input)?;
    let (input, _) = tag_no_case("FROM")(input)?;
    let (input, _) = multispace1(input)?;

    let (input, table) = parse_identifier(input)?;

    let (input, where_logic) = opt(ws(parse_where_logic))(input)?;
    let (input, order_by) = opt(ws(parse_order_clause))(input)?;
    let (input, limit) = opt(ws(parse_limit_clause))(input)?;

    // --- OPERATOR TREE BAUEN ---

    let mut current_op = json!({ "op": "scan", "table": table });

    // WHERE Logic verarbeiten (Filter vs Join unterscheiden)
    if let Some(logic) = where_logic {
        // Rekursive Funktion um Joins auszusortieren oder Filter zu wrappen
        current_op = wrap_conditions(current_op, logic);
    }

    // Projections
    if fields.len() > 1 || fields[0] != "*" {
        let projections: Vec<Value> = fields
            .iter()
            .map(|f| json!({ "type": "field", "name": f }))
            .collect();
        current_op = json!({ "op": "project", "projections": projections, "input": current_op });
    }

    // Limit & Order
    if let Some(l) = limit {
        let mut limit_op = json!({ "op": "limit", "limit": l, "input": current_op });
        if let Some(orders) = order_by {
            limit_op
                .as_object_mut()
                .unwrap()
                .insert("order_by".to_string(), json!(orders));
        }
        current_op = limit_op;
    }

    Ok((input, current_op))
}

// Hilfsfunktion: Entscheidet ob Filter oder Join
fn wrap_conditions(input_op: Value, predicate: Value) -> Value {
    // Check if it is a Join Candidate
    if let Some(obj) = predicate.as_object() {
        if let Some(t) = obj.get("type").and_then(|s| s.as_str()) {
            if t == "__JOIN_CANDIDATE__" {
                // JOIN BAUEN
                let left_field = obj.get("left").unwrap().as_str().unwrap();
                let right_full = obj.get("right").unwrap().as_str().unwrap();

                // Wir nehmen an: table.field -> right table ist "table"
                let parts: Vec<&str> = right_full.split('.').collect();
                let (r_table, r_col) = if parts.len() > 1 {
                    (parts[0], parts[1])
                } else {
                    (right_full, "id")
                };

                return json!({
                    "op": "join",
                    "left": input_op,
                    "right": { "op": "scan", "table": r_table },
                    "on": {
                        "left_field": left_field,
                        "right_field": r_col
                    }
                });
            } else if t == "and" {
                // Bei AND können wir sequentiell wrappen (Filter -> Join -> Filter)
                let list = obj.get("predicates").unwrap().as_array().unwrap();
                let mut curr = input_op;
                for p in list {
                    curr = wrap_conditions(curr, p.clone());
                }
                return curr;
            } else if t == "or" {
                // Bei OR gehen wir davon aus, dass es NUR Filter sind (keine Joins im OR!)
                // Das ist eine Einschränkung, aber Joins im OR sind relational sehr komplex.
                return json!({
                    "op": "filter",
                    "predicate": predicate,
                    "input": input_op
                });
            }
        }
    }

    // Standard Filter
    json!({
        "op": "filter",
        "predicate": predicate,
        "input": input_op
    })
}
