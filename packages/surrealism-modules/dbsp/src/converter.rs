use anyhow::{anyhow, Result};
use serde_json::{json, Value};

/// Simple regex-based SQL parser to replace surrealdb-core dependency which breaks on WASM build
/// Supports:
/// SELECT * FROM table WHERE ...
/// SELECT * FROM t1, t2 WHERE ...
/// ... LIMIT n
/// ... (SELECT ...) as field
pub fn convert_surql_to_dbsp(sql: &str) -> Result<Value> {
    let clean_sql = sql.trim();
    
    // Extract LIMIT
    let (mut query, limit) = extract_limit(clean_sql);
    query = query.trim();

    // Extract SELECT ... FROM
    // Use balanced search for FROM
    let from_idx = find_keyword_balanced(query, " FROM ").ok_or(anyhow!("Missing FROM clause"))?;
    
    let select_part = query[6..from_idx].trim(); // skip "SELECT "
    let rest = query[from_idx+6..].trim(); // skip " FROM "

    // Extract WHERE
    let (tables_part, where_part) = if let Some(where_idx) = find_keyword_balanced(rest, " WHERE ") {
        (&rest[..where_idx], Some(&rest[where_idx+7..]))
    } else {
        (rest, None)
    };

    // Parse Tables
    let tables: Vec<String> = tables_part.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    
    if tables.is_empty() {
        return Err(anyhow!("No tables found"));
    }

    let root_table = tables[0].clone();
    
    // Build Root Op
    let mut root_op = json!({
        "op": "scan",
        "table": root_table
    });

    // Parse WHERE for Filters and Joins (BEFORE Projections)
    if let Some(cond) = where_part {
        root_op = apply_conditions(root_op, cond, &tables, &root_table)?;
    }

    // Parse Projections (if not just *)
    if select_part != "*" {
        if let Ok(projs) = parse_projections(select_part) {
            root_op = json!({
                "op": "project",
                "input": root_op,
                "projections": projs
            });
        }
    }
    
    // Apply Limit
    if let Some(l) = limit {
        root_op = json!({
            "op": "limit",
            "limit": l,
            "input": root_op
        });
    }

    Ok(root_op)
}

fn find_keyword_balanced(s: &str, keyword: &str) -> Option<usize> {
    let s_upper = s.to_uppercase();
    let key_upper = keyword.to_uppercase();
    let mut depth = 0;
    
    // Scan manually
    // We iterate byte indices to be safe with slicing
    for (i, c) in s.char_indices() {
        if c == '(' { depth += 1; }
        else if c == ')' { if depth > 0 { depth -= 1; } }
        
        if depth == 0 {
            if s_upper[i..].starts_with(&key_upper) {
                return Some(i);
            }
        }
    }
    None
}

fn extract_limit(sql: &str) -> (&str, Option<usize>) {
    // We need balanced search for LIMIT too, technically, though usually at end.
    // Use find_keyword_balanced implies searching from start. limit is at end.
    // Try simple rfind first, but check if it's inside parens? 
    // Actually, simple rfind is dangerous if subquery has limit.
    // But convert_surql_to_dbsp is called recursively.
    // The LIMIT for THIS query must be at the very end of the string (after WHERE).
    // So we can check if the last " LIMIT " is at depth 0?
    // Let's perform a full scan to be safe, find LAST limit at depth 0.
    
    let s_upper = sql.to_uppercase();
    let mut depth = 0;
    let mut last_limit_idx = None;
    
    for (i, c) in sql.char_indices() {
        if c == '(' { depth += 1; }
        else if c == ')' { if depth > 0 { depth -= 1; } }
        
        if depth == 0 {
             if s_upper[i..].starts_with(" LIMIT ") {
                 last_limit_idx = Some(i);
             }
        }
    }
    
    if let Some(idx) = last_limit_idx {
         let val_str = sql[idx+7..].trim();
         if let Ok(n) = val_str.parse::<usize>() {
             return (&sql[..idx], Some(n));
         }
    }
    (sql, None)
}

fn parse_projections(proj_str: &str) -> Result<Vec<Value>> {
    let parts = split_balanced(proj_str, ',');
    let mut out = Vec::new();
    
    for p in parts {
        let p = p.trim();
        if p == "*" {
            out.push(json!({ "type": "all" }));
        } else if p.starts_with('(') && (p.to_uppercase().contains("SELECT")) {
             // Subquery
             let close_idx = p.rfind(')').unwrap_or(p.len());
             if close_idx == 0 { continue; } // Safety
             let sub_sql = &p[1..close_idx];
             
             let alias_part = if close_idx < p.len() {
                 p[close_idx+1..].trim()
             } else { "" };
             
             let alias = if alias_part.to_uppercase().starts_with("AS ") {
                 alias_part[3..].trim().to_string()
             } else if !alias_part.is_empty() {
                 alias_part.to_string()
             } else {
                 "subquery".to_string()
             };
             
             let sub_plan = convert_surql_to_dbsp(sub_sql)?;
             
             out.push(json!({
                 "type": "subquery",
                 "alias": alias,
                 "plan": sub_plan
             }));
        } else {
            out.push(json!({ "type": "field", "name": p }));
        }
    }
    Ok(out)
}

fn split_balanced(s: &str, delim: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth = 0;
    
    for c in s.chars() {
        if c == '(' { depth += 1; }
        else if c == ')' { if depth > 0 { depth -= 1; } }
        
        if c == delim && depth == 0 {
            parts.push(current.trim().to_string());
            current.clear();
        } else {
            current.push(c);
        }
    }
    if !current.trim().is_empty() {
        parts.push(current.trim().to_string());
    }
    parts
}

fn apply_conditions(mut op: Value, cond_str: &str, _tables: &[String], current_table: &str) -> Result<Value> {
    // Split by " AND " balanced? WHERE clause shouldn't have parens usually unless nested AND/OR.
    // SurQL doesn't support complex nested OR in our simple parser yet.
    // But " AND " is standard.
    // If subquery in WHERE? `WHERE id = (SELECT ...)`
    // We need balanced split for AND.
    // Re-use split_balanced but with string delimiter?
    // Let's implement split_balanced_str.
    
    let parts = split_balanced_str(cond_str, " AND ");
    
    for part in parts {
        let part = part.trim();
        if part.is_empty() { continue; }
        
        let eq_parts: Vec<&str> = part.split('=').map(|s| s.trim()).collect();
        if eq_parts.len() == 2 {
            let left = eq_parts[0];
            let right = eq_parts[1];
            
            let is_value = right.starts_with('\'') || right.starts_with('"') || right.chars().all(char::is_numeric) || right == "true" || right == "false";
            
            if is_value {
                // Filter
                let val_str = right.trim_matches('\'').trim_matches('"');
                let is_prefix = val_str.ends_with('*');
                let val_clean = val_str.trim_end_matches('*');
                
                let val_json: Value = if right == "true" { json!(true) } 
                                      else if right == "false" { json!(false) }
                                      else if let Ok(n) = right.parse::<i64>() { json!(n) }
                                      else { json!(val_clean) };

                let field = if left.contains('.') {
                    left.split('.').nth(1).unwrap()
                } else {
                    left
                };
                
                let predicate = if is_prefix {
                     json!({ "type": "prefix", "prefix": val_clean })
                } else {
                     json!({ "type": "eq", "field": field, "value": val_json })
                };

                op = json!({
                    "op": "filter",
                    "predicate": predicate,
                    "input": op
                });
            } else {
                // Join or Param?
                if right.starts_with('$') {
                    // Filter with Param
                     let field = if left.contains('.') { left.split('.').nth(1).unwrap() } else { left };
                     let param_name = right.trim_start_matches('$').trim_start_matches("parent.");
                     
                     let predicate = json!({ 
                         "type": "eq", 
                         "field": field, 
                         "value": { "$param": param_name } 
                     });
                     
                     op = json!({
                        "op": "filter",
                        "predicate": predicate,
                        "input": op
                    });
                } else {
                    // Join
                    let (l_tab, l_col) = extract_col(left);
                    let (r_tab, r_col) = extract_col(right);
                    
                    let other_table = if l_tab == Some(current_table) { r_tab } else { l_tab };
                    let (my_col, other_col) = if l_tab == Some(current_table) { (l_col, r_col) } else { (r_col, l_col) };
                    
                    if let Some(ot) = other_table {
                         let right_op = json!({ "op": "scan", "table": ot });
                         op = json!({
                            "op": "join",
                            "left": op,
                            "right": right_op,
                            "on": {
                                "left_field": my_col,
                                "right_field": other_col
                            }
                        });
                    }
                }
            }
        }
    }
    Ok(op)
}

fn split_balanced_str(s: &str, delim: &str) -> Vec<String> {
    // Naive implementation matching split_balanced logic roughly
    // Just replace " AND " with special char? No.
    // Manual scan.
    let mut parts = Vec::new();
    let mut last = 0;
    let mut depth = 0;
    let delim_len = delim.len();
    
    // We need to iterate char indices.
    // If delim is " AND " (space and space).
    // Simple logic:
    for (i, c) in s.char_indices() {
        if c == '(' { depth += 1; }
        else if c == ')' { if depth > 0 { depth -= 1; } }
        
        if depth == 0 {
             if s[i..].starts_with(delim) {
                 parts.push(s[last..i].to_string());
                 last = i + delim_len;
             }
        }
    }
    if last < s.len() {
        parts.push(s[last..].to_string());
    }
    parts
}

fn extract_col(s: &str) -> (Option<&str>, &str) {
    if let Some(idx) = s.find('.') {
        (Some(&s[..idx]), &s[idx+1..])
    } else {
        (None, s)
    }
}
