use anyhow::{anyhow, Result};
use serde_json::{json, Value};

/// Simple regex-based SQL parser to replace surrealdb-core dependency which breaks on WASM build
/// Supports:
/// SELECT * FROM table WHERE ...
/// ... ORDER BY field1 ASC, field2 DESC
/// ... LIMIT n
pub fn convert_surql_to_dbsp(sql: &str) -> Result<Value> {
    let clean_sql = sql.trim().trim_end_matches(';');
    
    // 1. naive parsing: SELECT <projections> FROM <tables> [WHERE <cond>] [ORDER BY <orders>] [LIMIT <n>]
    
    // Extract LIMIT
    let (mut query, limit) = extract_limit(clean_sql);
    query = query.trim();
    
    // Extract ORDER BY
    let (query_minus_order, order_by) = extract_order_by(query);
    query = query_minus_order.trim();

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
    
    // Apply Limit (and OrderBy attached to Limit if present)
    if let Some(l) = limit {
        let mut limit_op = json!({
            "op": "limit",
            "limit": l,
            "input": root_op
        });
        
        if let Some(orders) = order_by {
            limit_op.as_object_mut().unwrap().insert("order_by".to_string(), json!(orders));
        }
        root_op = limit_op;
    } else if order_by.is_some() {
        // Warning: ORDER BY without LIMIT is currently ignored in this simulated view engine
        // (unless we add a dummy Limit or Sort operator, but user request implies usually with limit)
        // For correctness, maybe we should add a Sort operator? 
        // But `lib.rs` doesn't have `Operator::Sort`. 
        // We'll proceed with just Limit support for now as that's the main impactful one.
    }

    Ok(root_op)
}

fn find_keyword_balanced(s: &str, keyword: &str) -> Option<usize> {
    let s_upper = s.to_uppercase();
    let key_upper = keyword.to_uppercase();
    let mut depth = 0;
    
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
    if let Some(idx) = find_keyword_balanced(sql, " LIMIT ") {
         let val_str = sql[idx+7..].trim().trim_end_matches(';');
         if let Ok(n) = val_str.parse::<usize>() {
             return (&sql[..idx], Some(n));
         }
    }
    (sql, None)
}

fn extract_order_by(sql: &str) -> (&str, Option<Vec<Value>>) {
    // ORDER BY f1 ASC, f2 DESC
    if let Some(idx) = find_keyword_balanced(sql, " ORDER BY ") {
        let order_str = sql[idx+10..].trim();
        let parts = split_balanced(order_str, ',');
        let mut orders = Vec::new();
        
        for p in parts {
             let p = p.trim();
             let p_upper = p.to_uppercase();
             let (field, dir) = if p_upper.ends_with(" DESC") {
                 (p[..p.len()-5].trim(), "DESC")
             } else if p_upper.ends_with(" ASC") {
                 (p[..p.len()-4].trim(), "ASC")
             } else {
                 (p, "ASC")
             };
             
             orders.push(json!({
                 "field": field,
                 "direction": dir
             }));
        }
        
        return (&sql[..idx], Some(orders));
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
             if close_idx == 0 { continue; } 
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

fn apply_conditions(mut op: Value, cond_str: &str, tables: &[String], current_table: &str) -> Result<Value> {
    // 1. Split by " OR "
    let or_parts = split_balanced_str(cond_str, " OR ");
    
    if or_parts.len() > 1 {
        // Multi-branch OR
        // Each branch is a set of ANDs
        // Usually OR implies a Filter composition: Filter(Or([Filter(A), Filter(B)]))
        // But what if one branch involves Joins? 
        // "user.id=post.author OR user.admin=true"
        // This is complex. Simulated DBSP generally expects Filters to be predicates on the Row.
        // Joins are structural. 
        // Assumption: OR is only used for pure Filters (predicates), not Joins.
        // If a Join is inside an OR, it's very hard to map to standard relational algebra without Unions basically.
        // We will assume OR contains only Predicates.
        
        let mut preds = Vec::new();
        for branch in or_parts {
            let p = parse_condition_expression(&branch)?;
            preds.push(p);
        }
        
        let or_pred = json!({
            "type": "or",
            "predicates": preds
        });
        
        op = json!({
            "op": "filter",
            "predicate": or_pred,
            "input": op
        });
        
    } else {
        // Single branch (ANDs)
        // Can contain Joins and Predicates mixed.
        // We must separate Joins from Filter Predicates?
        // Current logic iteratively wraps.
        // Filter -> Filter -> Join
        
        // We need to parse ANDs.
        let and_parts = split_balanced_str(cond_str, " AND ");
        
        // Collect pure predicates to maybe combine into an "AND" predicate?
        // Or just wrap sequentially as before.
        // Updating to wrapping sequentially for simplicity except for purely local predicates which can be grouped.
        // For backwards compat and structure, let's keep the iterative approach but support "And" type if needed.
        
        for part in and_parts {
            op = parse_single_condition(op, &part, tables, current_table)?;
        }
    }
    
    Ok(op)
}

fn parse_condition_expression(cond: &str) -> Result<Value> {
    // Recursive parser for predicates (no joins)
    // Handles AND/OR composition
    let or_parts = split_balanced_str(cond, " OR ");
    if or_parts.len() > 1 {
        let mut kids = Vec::new();
        for p in or_parts { kids.push(parse_condition_expression(&p)?); }
        return Ok(json!({ "type": "or", "predicates": kids }));
    }
    
    let and_parts = split_balanced_str(cond, " AND ");
    if and_parts.len() > 1 {
        let mut kids = Vec::new();
        for p in and_parts { kids.push(parse_condition_expression(&p)?); }
        return Ok(json!({ "type": "and", "predicates": kids }));
    }
    
    // Leaf: pure predicate
    let part = cond.trim();
    let eq_parts: Vec<&str> = part.split('=').map(|s| s.trim()).collect();
    if eq_parts.len() == 2 {
        let left = eq_parts[0];
        let right = eq_parts[1];
        let is_value = right.starts_with('\'') || right.starts_with('"') || right.chars().all(char::is_numeric) || right == "true" || right == "false";

        if is_value {
            let val_str = right.trim_matches('\'').trim_matches('"');
            let is_prefix = val_str.ends_with('*');
            let val_clean = val_str.trim_end_matches('*');
            
            let val_json: Value = if right == "true" { json!(true) } 
                                  else if right == "false" { json!(false) }
                                  else if let Ok(n) = right.parse::<i64>() { json!(n) }
                                  else { json!(val_clean) };

            let field = if left.contains('.') { left.split('.').nth(1).unwrap() } else { left };
            
            if is_prefix {
                 return Ok(json!({ "type": "prefix", "prefix": val_clean }));
            } else {
                 return Ok(json!({ "type": "eq", "field": field, "value": val_json }));
            }
        }
    }
    
    Err(anyhow!("Unsupported predicate format: {}", cond))
}

fn parse_single_condition(mut op: Value, part: &str, _tables: &[String], current_table: &str) -> Result<Value> {
    let part = part.trim();
    if part.is_empty() { return Ok(op); }
    
    let eq_parts: Vec<&str> = part.split('=').map(|s| s.trim()).collect();
    if eq_parts.len() == 2 {
        let left = eq_parts[0];
        let right = eq_parts[1];
        
        let is_value = right.starts_with('\'') || right.starts_with('"') || right.chars().all(char::is_numeric) || right == "true" || right == "false";
        
        if is_value {
            // It's a filter predicate. 
            // We can construct it via parse_condition_expression
            if let Ok(pred) = parse_condition_expression(part) {
                op = json!({
                    "op": "filter",
                    "predicate": pred,
                    "input": op
                });
            }
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
    Ok(op)
}

fn split_balanced_str(s: &str, delim: &str) -> Vec<String> {
    // Naive implementation matching split_balanced logic roughly
    let mut parts = Vec::new();
    let mut last = 0;
    let mut depth = 0;
    let delim_len = delim.len();
    
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
