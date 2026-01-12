use anyhow::{anyhow, Result};
use nom::{
    branch::alt,
    bytes::complete::{is_not, tag, tag_no_case, take_while},
    character::complete::{alpha1, char, digit1, multispace0},
    combinator::{cut, map, map_res, opt, recognize, value}, 
    multi::separated_list1,
    sequence::{delimited, pair, preceded, tuple},
    IResult,
};
use serde_json::{json, Value};

pub fn convert_surql_to_dbsp(sql: &str) -> Result<Value> {
    let clean_sql = sql.trim().trim_end_matches(';');
    match parse_full_query(clean_sql) {
        Ok((_, plan)) => Ok(plan),
        Err(e) => Err(anyhow!("SQL Parsing Error: {}", e)),
    }
}

// --- HELPERS ---

fn ws<'a, F, O, E: nom::error::ParseError<&'a str>>(
    inner: F,
) -> impl FnMut(&'a str) -> IResult<&'a str, O, E>
where
    F: FnMut(&'a str) -> IResult<&'a str, O, E>,
{
    delimited(multispace0, inner, multispace0)
}

// Identifier: Start with Alpha/_, then Alphanumeric/_/:/.
fn parse_identifier(input: &str) -> IResult<&str, String> {
    let parser = recognize(pair(
        alt((alpha1, tag("_"))),
        take_while(|c: char| c.is_alphanumeric() || c == '_' || c == ':' || c == '.'),
    ));
    map(parser, |s: &str| s.to_string())(input)
}

// --- VALUES ---

#[derive(Debug, Clone)]
enum ParsedValue {
    Json(Value),
    Identifier(String),
    Prefix(String),
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
        parse_string_literal,
        map(preceded(char('$'), parse_identifier), |s| {
            ParsedValue::Json(json!({ "$param": s }))
        }),
        value(ParsedValue::Json(json!(true)), tag_no_case("true")),
        value(ParsedValue::Json(json!(false)), tag_no_case("false")),
        // Numbers before Identifiers!
        map_res(digit1, |s: &str| {
            s.parse::<i64>().map(|n| ParsedValue::Json(json!(n)))
        }),
        map(parse_identifier, ParsedValue::Identifier),
    ))(input)
}

// --- LOGIC ---

fn parse_leaf_predicate(input: &str) -> IResult<&str, Value> {
    let (input, (left, op, right)) = tuple((
        ws(parse_identifier),
        ws(alt((
            tag(">="),
            tag("<="),
            tag("!="),
            tag("="),
            tag(">"),
            tag("<"),
            tag_no_case("CONTAINS"),
            tag_no_case("INSIDE"),
        ))),
        ws(parse_value_entry),
    ))(input)?;

    let type_str = match op.to_uppercase().as_str() {
        "=" => "eq",
        ">" => "gt",
        "<" => "lt",
        ">=" => "gte", 
        "<=" => "lte", 
        "!=" => "neq", 
        _ => "eq",
    };

    match right {
        ParsedValue::Json(val) => Ok((
            input,
            json!({ "type": type_str, "field": left, "value": val }),
        )),
        ParsedValue::Prefix(val) => Ok((
            input,
            json!({ "type": "prefix", "field": left, "prefix": val }),
        )),
        ParsedValue::Identifier(right_field) => Ok((
            input,
            json!({ "type": "__JOIN_CANDIDATE__", "left": left, "right": right_field }),
        )),
    }
}

// Recursive Expression Parser
// Logic: Or -> And -> Term (Leaf or Parens)

fn parse_term(input: &str) -> IResult<&str, Value> {
    alt((
        delimited(ws(char('(')), parse_or_expression, ws(char(')'))),
        parse_leaf_predicate
    ))(input)
}

fn parse_and_expression(input: &str) -> IResult<&str, Value> {
    let (input, terms) = separated_list1(ws(tag_no_case("AND")), parse_term)(input)?;
    if terms.len() == 1 {
        Ok((input, terms[0].clone()))
    } else {
         Ok((input, json!({ "type": "and", "predicates": terms })))
    }
}

fn parse_or_expression(input: &str) -> IResult<&str, Value> {
    let (input, terms) = separated_list1(ws(tag_no_case("OR")), parse_and_expression)(input)?;
    if terms.len() == 1 {
        Ok((input, terms[0].clone()))
    } else {
         Ok((input, json!({ "type": "or", "predicates": terms })))
    }
}

fn parse_where_logic(input: &str) -> IResult<&str, Value> {
    preceded(
        tag_no_case("WHERE"),
        cut(parse_or_expression),
    )(input)
}

// --- MAIN QUERY ---

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

// --- SELECT PROJECTION ---

fn parse_subquery_projection(input: &str) -> IResult<&str, Value> {
    // (SELECT ... ) AS alias
    let (input, sub_plan) = delimited(
        ws(char('(')),
        parse_full_query,
        ws(char(')')),
    )(input)?;

    let (input, _) = ws(tag_no_case("AS"))(input)?;
    let (input, alias) = ws(parse_identifier)(input)?;

    Ok((input, json!({ "type": "subquery", "alias": alias, "plan": sub_plan })))
}

fn parse_field_projection(input: &str) -> IResult<&str, Value> {
     // field OR field AS alias (though we usually just use field name)
     // keeping it simple: just identifier for now, or *
    alt((
        map(tag("*"), |_| json!({ "type": "all" })),
        map(parse_identifier, |f| json!({ "type": "field", "name": f }))
    ))(input)
}

fn parse_projection_item(input: &str) -> IResult<&str, Value> {
    alt((
        parse_subquery_projection,
        parse_field_projection
    ))(input)
}

fn parse_full_query(input: &str) -> IResult<&str, Value> {
    let (input, _) = ws(tag_no_case("SELECT"))(input)?;
    
    let (input, fields) = separated_list1(ws(char(',')), parse_projection_item)(input)?;

    let (input, _) = ws(tag_no_case("FROM"))(input)?;
    let (input, table) = ws(parse_identifier)(input)?;

    let (input, where_logic) = opt(ws(parse_where_logic))(input)?;

    let (input, order_by) = opt(ws(parse_order_clause))(input)?;
    let (input, limit) = opt(ws(parse_limit_clause))(input)?;

    // --- TREE BUILDING ---
    let mut current_op = json!({ "op": "scan", "table": table });

    if let Some(logic) = where_logic {
        current_op = wrap_conditions(current_op, logic);
    }

    // Projections
    // If we have just one "type": "all", and nothing else, we skip projection technically
    // But let's be explicit if desired. 
    // If fields contains any subquery or if fields is not just "*", we project.
    let needs_projection = fields.len() > 1 || fields[0].get("type").and_then(|t| t.as_str()) != Some("all");

    if needs_projection {
        current_op = json!({ "op": "project", "projections": fields, "input": current_op });
    }

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

fn wrap_conditions(input_op: Value, predicate: Value) -> Value {
    // Check if we can extract a Join
    // Simple logic: If top-level is JoinCandidate, convert to JOIN op.
    
    if let Some(obj) = predicate.as_object() {
        if let Some(t) = obj.get("type").and_then(|s| s.as_str()) {
            if t == "__JOIN_CANDIDATE__" {
                let left_field = obj.get("left").and_then(|v| v.as_str()).unwrap_or("id");
                let right_full = obj
                    .get("right")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                
                // Assume right_full is "table.field"
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
                    "on": { "left_field": left_field, "right_field": r_col }
                });
            }
        }
    }
    
    // Default: Filter
    json!({ "op": "filter", "predicate": predicate, "input": input_op })
}
