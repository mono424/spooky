use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use surrealdb_core::sql::{
    Statement, Value as SqlValue, Expression, Operator as SqlOp, 
    Number, Part, Field
};
use surrealdb_core::syn;
use std::collections::HashSet;

#[derive(Debug, Clone)]
struct ParsedJoin {
    left_table: String,
    left_field: String,
    right_table: String,
    right_field: String,
}

#[derive(Debug, Clone)]
struct ParsedFilter {
    table: Option<String>,
    field: String,
    op: SqlOp,
    value: SqlValue,
}

/// Converts a SurrealQL SELECT statement into a DBSP Operator JSON tree.
pub fn convert_surql_to_dbsp(sql: &str) -> Result<Value> {
    let parsed = syn::parse(sql).map_err(|e| anyhow!("Parse error: {}", e))?;
    let stmt = parsed.0.into_iter().next().ok_or_else(|| anyhow!("No statement found"))?;
    process_statement(stmt)
}

fn process_statement(stmt: Statement) -> Result<Value> {
    match stmt {
        Statement::Select(select) => {
            // 1. Identify Tables
            let mut tables = Vec::new();
            for w in select.what {
                match w {
                    SqlValue::Table(t) => tables.push(t.0.to_string()),
                    _ => return Err(anyhow!("Only simple table selection supported (FROM table1, table2)")),
                }
            }
            if tables.is_empty() {
                return Err(anyhow!("No tables specified in FROM clause"));
            }

            // 2. Parse WHERE clause into disjoint conditions (Joins vs Filters)
            let (mut filters, joins) = if let Some(cond) = select.cond {
                parse_conditions(&cond.0)?
            } else {
                (Vec::new(), Vec::new())
            };

            // 3. Build Recursive Tree starting from the first table (Primary)
            let root_table = tables[0].clone();
            let mut visited = HashSet::new();
            
            let mut root_op = build_tree(&root_table, &tables, &joins, &mut filters, &mut visited)?;

            // 4. Apply Projections (SELECT expr)
            if !is_simple_wildcard(&select.expr) {
                root_op = apply_projections(root_op, &select.expr)?;
            }

            // 5. Apply Limit
            if let Some(limit) = select.limit {
                let limit_val = match limit.0 {
                    SqlValue::Number(n) => n.as_int() as usize,
                    _ => 10
                };
                root_op = json!({
                    "op": "limit",
                    "limit": limit_val,
                    "input": root_op
                });
            }

            Ok(root_op)
        },
        Statement::Value(val) => {
            if let SqlValue::Subquery(sub) = val {
                let sql = sub.to_string();
                let clean_sql = sql.trim_start_matches('(').trim_end_matches(')');
                // We call the main public entry point which calls parse + process
                convert_surql_to_dbsp(clean_sql)
            } else {
                Err(anyhow!("Unsupported value statement: {:?}", val))
            }
        },
        _ => {
            println!("Unsupported statement type: {:?}", stmt);
            Err(anyhow!("Only SELECT statements are supported"))
        }
    }
}

fn is_simple_wildcard(exprs: &[Field]) -> bool {
    if exprs.len() == 1 {
        match &exprs[0] {
            Field::All => return true,
            _ => return false,
        }
    }
    false
}

fn apply_projections(input: Value, exprs: &[Field]) -> Result<Value> {
    let mut projections = Vec::new();

    for field in exprs {
        match field {
            Field::All => {
                projections.push(json!({ "type": "all" }));
            },
            Field::Single { expr, alias } => {
                // Expr can be a Field (Idiom) or a Subquery
                match expr {
                    SqlValue::Idiom(idiom) => {
                        let name = idiom.to_string(); // Simple field name
                        projections.push(json!({ "type": "field", "name": name }));
                    },
                    SqlValue::Subquery(stmt) => {
                        // Recursively parse the subquery
                        // Subquery is Box<Statement>
                        let sub_sql = stmt.to_string(); // Or re-parse from struct?
                        // convert_surql_to_dbsp expects STR. 
                        // But we have Statement struct.
                        // We can modify convert_surql_to_dbsp to take Statement?
                        // Or just reconstruct? to_string() works.
                        let sub_op = convert_surql_to_dbsp(&sub_sql)?;
                        
                        let alias_name = if let Some(ident) = alias {
                            ident.to_string()
                        } else {
                            "subquery".to_string()
                        };

                        projections.push(json!({
                            "type": "subquery",
                            "alias": alias_name,
                            "plan": sub_op
                        }));
                    },
                    _ => continue // Skip unsupported expressions
                }
            },
            _ => continue // Skip other field types
        }
    }

    Ok(json!({
        "op": "project",
        "input": input,
        "projections": projections
    }))
}

fn parse_conditions(expr_val: &SqlValue) -> Result<(Vec<ParsedFilter>, Vec<ParsedJoin>)> {
    let mut filters = Vec::new();
    let mut joins = Vec::new();
    
    let mut exprs = Vec::new();
    flatten_and(expr_val, &mut exprs);

    for expr in exprs {
         if let SqlValue::Expression(e) = expr {
            if let Expression::Binary { l, o, r } = *e.clone() {
                let (l_table, l_field) = extract_field(&l).ok_or_else(|| anyhow!("Left side of condition must be a field"))?;
                
                if let Some((r_table, r_field)) = extract_field(&r) {
                    let l_t = l_table.clone().unwrap_or_default();
                    let r_t = r_table.clone().unwrap_or_default();
                    
                    if !l_t.is_empty() && !r_t.is_empty() && l_t != r_t {
                        joins.push(ParsedJoin {
                            left_table: l_t,
                            left_field: l_field,
                            right_table: r_t,
                            right_field: r_field,
                        });
                        continue;
                    }
                }

                filters.push(ParsedFilter {
                    table: l_table,
                    field: l_field,
                    op: o,
                    value: r,
                });
            }
        }
    }

    Ok((filters, joins))
}

fn flatten_and(val: &SqlValue, out: &mut Vec<SqlValue>) {
    if let SqlValue::Expression(expr) = val {
        if let Expression::Binary { l, o, r } = *expr.clone() {
             if let SqlOp::And = o {
                 flatten_and(&l, out);
                 flatten_and(&r, out);
                 return;
             }
        }
    }
    out.push(val.clone());
}

fn extract_field(val: &SqlValue) -> Option<(Option<String>, String)> {
    match val {
        SqlValue::Idiom(idiom) => {
            let parts = &idiom.0;
            let clean = |p: &Part| p.to_string().trim_start_matches('.').to_string();
            
            if parts.len() == 1 {
                 Some((None, clean(&parts[0])))
            } else if parts.len() >= 2 {
                 Some((Some(clean(&parts[0])), clean(&parts[1])))
            } else {
                None
            }
        },
        _ => None
    }
}

fn convert_value(val: &SqlValue) -> Option<(Value, bool)> {
    match val {
         SqlValue::Strand(s) => {
             let str_val = s.0.clone();
             if str_val.ends_with('*') {
                 Some((str_val.trim_end_matches('*').to_string().into(), true))
             } else {
                 Some((str_val.into(), false))
             }
         },
         SqlValue::Number(n) => {
              match n {
                 Number::Int(i) => Some((json!(i), false)),
                 Number::Float(f) => Some((json!(f), false)),
                 Number::Decimal(d) => Some((json!(d.to_string()), false)),
                 _ => None
             }
         },
         SqlValue::Bool(b) => Some((json!(b), false)),
         SqlValue::Idiom(idiom) => {
             // Check if it's a Param ($param...)
             let s = idiom.to_string();
             if s.contains("$parent") || s.contains("$") {
                  // Special handling for params
                  // Map $parent.author -> author via context?
                  // We encode it as a Param object
                  Some((json!({ "$param": s.trim_start_matches('$').trim_start_matches("parent.").to_string() }), false))
             } else {
                 None // Standard field?
             }
         },
         SqlValue::Param(p) => {
              Some((json!({ "$param": p.0.to_string() }), false))
         },
         _ => None 
    }
}

fn build_tree(
    current_table: &str, 
    all_tables: &[String], 
    joins: &[ParsedJoin], 
    filters: &mut Vec<ParsedFilter>, 
    visited: &mut HashSet<String>
) -> Result<Value> {
    visited.insert(current_table.to_string());

    let mut op = json!({
        "op": "scan",
        "table": current_table
    });

    let relevant_filters: Vec<_> = filters.iter().filter(|f| {
        f.table.as_deref() == Some(current_table) || f.table.is_none()
    }).collect();

    for f in relevant_filters {
         if let SqlOp::Equal = f.op {
             if let Some((val, is_prefix)) = convert_value(&f.value) {
                 let predicate = if is_prefix {
                     json!({ "type": "prefix", "prefix": val })
                 } else {
                     json!({ "type": "eq", "field": f.field, "value": val })
                 };

                 op = json!({
                     "op": "filter",
                     "predicate": predicate,
                     "input": op
                 });
             }
         }
    }

    for join in joins {
        let (other_table, my_field, other_field) = if join.left_table == current_table && !visited.contains(&join.right_table) {
             (&join.right_table, &join.left_field, &join.right_field)
        } else if join.right_table == current_table && !visited.contains(&join.left_table) {
             (&join.left_table, &join.right_field, &join.left_field)
        } else {
            continue;
        };

        let right_op = build_tree(other_table, all_tables, joins, filters, visited)?;
        
        op = json!({
            "op": "join",
            "left": op,
            "right": right_op,
            "on": {
                "left_field": my_field,
                "right_field": other_field
            }
        });
    }

    Ok(op)
}
