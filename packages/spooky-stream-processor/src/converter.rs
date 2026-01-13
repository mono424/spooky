use anyhow::{anyhow, Result};
use nom::{
    branch::alt,
    bytes::complete::{is_not, tag, tag_no_case, take_while},
    character::complete::{alpha1, char, digit1, multispace0, multispace1},
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

#[derive(Debug, Clone)]
enum SelectField {
    Wildcard,
    Field(String),
    Subquery {
        query: String,
        index: Option<usize>,
        alias: String,
    },
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
        parse_leaf_predicate,
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
    preceded(tag_no_case("WHERE"), cut(parse_or_expression))(input)
}

// --- MAIN QUERY ---

fn parse_limit_clause(input: &str) -> IResult<&str, usize> {
    preceded(
        tag_no_case("LIMIT"),
        ws(map_res(digit1, |s: &str| s.parse::<usize>())),
    )(input)
}

// Parse a subquery field like: (SELECT ...) [0] AS alias
fn parse_subquery_field(input: &str) -> IResult<&str, SelectField> {
    let (input, _) = ws(char('('))(input)?;

    // Find the matching closing parenthesis
    // This is a simplified approach - we'll capture everything until we find )
    let mut depth = 1;
    let mut end_pos = 0;
    for (i, c) in input.chars().enumerate() {
        if c == '(' {
            depth += 1;
        } else if c == ')' {
            depth -= 1;
            if depth == 0 {
                end_pos = i;
                break;
            }
        }
    }

    if depth != 0 {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Tag,
        )));
    }

    let query = &input[..end_pos];
    let input = &input[end_pos..];

    let (input, _) = char(')')(input)?;

    // Parse optional array index like [0]
    let (input, index) = opt(ws(delimited(
        char('['),
        map_res(digit1, |s: &str| s.parse::<usize>()),
        char(']'),
    )))(input)?;

    // Parse AS alias
    let (input, _) = ws(tag_no_case("AS"))(input)?;
    let (input, alias) = ws(parse_identifier)(input)?;

    Ok((
        input,
        SelectField::Subquery {
            query: query.to_string(),
            index,
            alias,
        },
    ))
}

// Parse a single select field (wildcard, identifier, or subquery)
fn parse_select_field(input: &str) -> IResult<&str, SelectField> {
    alt((
        map(char('*'), |_| SelectField::Wildcard),
        parse_subquery_field,
        map(parse_identifier, SelectField::Field),
    ))(input)
}

// Parse all select fields
fn parse_select_fields(input: &str) -> IResult<&str, Vec<SelectField>> {
    separated_list1(ws(char(',')), parse_select_field)(input)
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

    let (input, fields) = parse_select_fields(input)?;

    let (input, _) = ws(tag_no_case("FROM"))(input)?;
    let (input, _) = multispace0(input)?;

    let (input, table) = parse_identifier(input)?;

    let (input, where_logic) = opt(ws(parse_where_logic))(input)?;

    let (input, order_by) = opt(ws(parse_order_clause))(input)?;
    let (input, limit) = opt(ws(parse_limit_clause))(input)?;

    // --- TREE BUILDING ---
    let mut current_op = json!({ "op": "scan", "table": table });

    if let Some(logic) = where_logic {
        current_op = wrap_conditions(current_op, logic);
    }

    // Check if we have any non-wildcard fields or subqueries
    let has_wildcard = fields.iter().any(|f| matches!(f, SelectField::Wildcard));
    let non_wildcard_fields: Vec<&SelectField> = fields
        .iter()
        .filter(|f| !matches!(f, SelectField::Wildcard))
        .collect();

    if !non_wildcard_fields.is_empty() || (fields.len() == 1 && !has_wildcard) {
        let mut projections: Vec<Value> = Vec::new();

        for field in &fields {
            match field {
                SelectField::Wildcard => {
                    // Wildcard doesn't add a specific projection
                }
                SelectField::Field(name) => {
                    projections.push(json!({ "type": "field", "name": name }));
                }
                SelectField::Subquery { query, index: _, alias } => {
                    // Parse the subquery recursively
                    // Note: index is ignored for now as it's typically [0] for one-to-one relationships
                    // which is already handled by the Subquery projection variant
                    match parse_full_query(query) {
                        Ok((_, subquery_plan)) => {
                            projections.push(json!({
                                "type": "subquery",
                                "alias": alias,
                                "plan": subquery_plan
                            }));
                        }
                        Err(_) => {
                            // If subquery parsing fails, skip this projection
                            // This allows graceful degradation for unsupported subquery syntax
                        }
                    }
                }
            }
        }

        if !projections.is_empty() {
            current_op = json!({ "op": "project", "projections": projections, "input": current_op });
        }
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
    // TODO: This logic is very simple and only works if the top-level is a Join Candidate or strictly nested ANDs?
    // With simplified recursion, we might find JoinCandidate inside nested structures.
    // For now, let's keep it simple: Top Level or straightforward AND.

    if let Some(obj) = predicate.as_object() {
        if let Some(t) = obj.get("type").and_then(|s| s.as_str()) {
            if t == "__JOIN_CANDIDATE__" {
                let left_field = obj.get("left").and_then(|v| v.as_str()).unwrap_or("id");
                let right_full = obj
                    .get("right")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
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
            } else if t == "and" {
                // If AND, we wrap normally, View logic handles AND.
                // But optimization: if AND contains JoinCandidate, we might want to hoist it?
                // For this refactor, we stick to Filter-based evaluation unless it is explicitly a JOIN op in circuit.
                // JoinCandidate transformation usually happens here.
                // If multiple predicates, we just return filter for now.
                // Real Query Optimizer would pull out joins.
            }
        }
    }
    json!({ "op": "filter", "predicate": predicate, "input": input_op })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::view::Operator;

    #[test]
    fn test_parse_failing_subquery() {
        let sql = "SELECT *, (SELECT * FROM user WHERE id=$parent.author LIMIT 1)[0] AS author FROM thread ORDER BY created_at desc LIMIT 10;";
        let result = convert_surql_to_dbsp(sql);
        assert!(
            result.is_ok(),
            "Failed to parse subquery: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_subquery_deserializes_to_operator() {
        let sql = "SELECT *, (SELECT * FROM user WHERE id=$parent.author LIMIT 1)[0] AS author FROM thread ORDER BY created_at desc LIMIT 10;";
        let result = convert_surql_to_dbsp(sql).expect("Failed to parse SQL");

        // Try to deserialize the result into an Operator
        let operator: Result<Operator, _> = serde_json::from_value(result);
        assert!(
            operator.is_ok(),
            "Failed to deserialize to Operator: {:?}",
            operator.err()
        );

        // Verify the structure
        let op = operator.unwrap();
        match op {
            Operator::Limit { input, .. } => {
                match *input {
                    Operator::Project { projections, .. } => {
                        assert!(projections.len() > 0, "Expected projections");
                        // Check that we have a subquery projection
                        let has_subquery = projections.iter().any(|p| {
                            matches!(p, crate::engine::view::Projection::Subquery { .. })
                        });
                        assert!(has_subquery, "Expected at least one subquery projection");
                    }
                    _ => panic!("Expected Project operator inside Limit"),
                }
            }
            _ => panic!("Expected Limit operator at top level"),
        }
    }
}
