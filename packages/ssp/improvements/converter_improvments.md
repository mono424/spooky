# Parser Improvement Implementation Plan

## Overview

This plan breaks down the parser improvements into **actionable steps** with clear priorities, time estimates, and testing checkpoints.

**Total Estimated Time**: 8-10 hours over 2-3 days
**Difficulty**: Medium (most changes are straightforward)

---

## Phase 1: Critical Bug Fixes (2 hours)

### Step 1.1: Fix Unsafe `unwrap()` Calls (30 min)

**Priority**: 🔥 CRITICAL
**Risk**: High (will crash on malformed queries)
**Files**: Current parser code

#### Tasks:

1. **Find all `unwrap()` calls** (5 min)
   ```bash
   grep -n "unwrap()" your_parser_file.rs
   ```

2. **Replace in `parse_full_query`** (10 min)
   ```rust
   // Line ~230 (approximate)
   // BEFORE:
   if let Some(orders) = order_by {
       limit_op
           .as_object_mut()
           .unwrap()
           .insert("order_by".to_string(), json!(orders));
   }
   
   // AFTER:
   if let Some(orders) = order_by {
       if let Some(obj) = limit_op.as_object_mut() {
           obj.insert("order_by".to_string(), json!(orders));
       } else {
           return Err(nom::Err::Error(nom::error::Error::new(
               input,
               nom::error::ErrorKind::Fail
           )));
       }
   }
   ```

3. **Replace in `wrap_conditions`** (10 min)
   ```rust
   // Find any `.unwrap()` in wrap_conditions function
   // Replace with proper error handling
   ```

4. **Test** (5 min)
   ```bash
   cargo test
   ```

**Checkpoint**: All tests pass, no unwrap() calls remain

---

### Step 1.2: Add Better Error Messages (1 hour)

**Priority**: 🔥 CRITICAL
**Risk**: Medium (affects debugging experience)

#### Tasks:

1. **Create custom error type** (15 min)
   
   Create new file: `src/engine/parse_error.rs`
   ```rust
   use thiserror::Error;
   
   #[derive(Error, Debug)]
   pub enum ParseError {
       #[error("Unexpected token at position {position}: found '{found}', expected {expected}")]
       UnexpectedToken {
           position: usize,
           found: String,
           expected: String,
       },
       
       #[error("Unsupported SQL feature: {feature}")]
       UnsupportedFeature { feature: String },
       
       #[error("Invalid syntax near position {position}: {context}")]
       InvalidSyntax { position: usize, context: String },
       
       #[error("Query validation failed: {0}")]
       ValidationError(String),
       
       #[error("Parse error: {0}")]
       Generic(String),
   }
   
   impl ParseError {
       pub fn from_nom_error(input_sql: &str, error: nom::Err<nom::error::Error<&str>>) -> Self {
           match error {
               nom::Err::Error(e) | nom::Err::Failure(e) => {
                   let position = input_sql.len() - e.input.len();
                   let context = e.input.chars().take(30).collect::<String>();
                   
                   ParseError::InvalidSyntax {
                       position,
                       context,
                   }
               }
               nom::Err::Incomplete(_) => {
                   ParseError::Generic("Incomplete SQL query".to_string())
               }
           }
       }
   }
   ```

2. **Update `convert_surql_to_dbsp` signature** (10 min)
   ```rust
   // In your parser file
   use crate::engine::parse_error::ParseError;
   
   pub fn convert_surql_to_dbsp(sql: &str) -> Result<Value, ParseError> {
       let clean_sql = sql.trim().trim_end_matches(';');
       
       if clean_sql.is_empty() {
           return Err(ParseError::Generic("Empty SQL query".to_string()));
       }
       
       match parse_full_query(clean_sql) {
           Ok(("", plan)) => Ok(plan),
           
           Ok((remaining, _)) => {
               let position = clean_sql.len() - remaining.len();
               Err(ParseError::UnexpectedToken {
                   position,
                   found: remaining.chars().take(20).collect(),
                   expected: "end of query".to_string(),
               })
           }
           
           Err(e) => Err(ParseError::from_nom_error(clean_sql, e)),
       }
   }
   ```

3. **Update error handling in service layer** (20 min)
   
   Find where `convert_surql_to_dbsp` is called and update:
   ```rust
   // Before:
   match convert_surql_to_dbsp(sql) {
       Ok(plan) => { ... }
       Err(e) => {
           error!("Parse error: {}", e);  // Generic message
       }
   }
   
   // After:
   match convert_surql_to_dbsp(sql) {
       Ok(plan) => { ... }
       Err(e) => {
           error!("Parse error: {}", e);  // Now has position info!
           // Return error to user with context
       }
   }
   ```

4. **Test error messages** (15 min)
   ```rust
   #[test]
   fn test_error_message_has_position() {
       let sql = "SELECT * FROM users WHERE";
       let result = convert_surql_to_dbsp(sql);
       
       assert!(result.is_err());
       let err = result.unwrap_err();
       let err_msg = err.to_string();
       
       // Should contain position information
       assert!(err_msg.contains("position"));
   }
   
   #[test]
   fn test_error_on_unexpected_token() {
       let sql = "SELECT * FROM users FOOBAR";
       let result = convert_surql_to_dbsp(sql);
       
       assert!(result.is_err());
       let err = result.unwrap_err();
       
       match err {
           ParseError::UnexpectedToken { position, found, .. } => {
               assert!(position > 0);
               assert!(found.contains("FOOBAR"));
           }
           _ => panic!("Expected UnexpectedToken error"),
       }
   }
   ```

**Checkpoint**: Error messages show exact position and context

---

### Step 1.3: Add Query Validation (30 min)

**Priority**: 🔥 HIGH
**Risk**: Medium (prevents bad queries from reaching circuit)

#### Tasks:

1. **Create validation module** (20 min)
   
   Add to your parser file:
   ```rust
   fn validate_plan(plan: &Value) -> Result<(), ParseError> {
       validate_operator(plan, 0)
   }
   
   fn validate_operator(op: &Value, depth: usize) -> Result<(), ParseError> {
       // Prevent stack overflow from deeply nested queries
       if depth > 100 {
           return Err(ParseError::ValidationError(
               "Query too deeply nested (max 100 levels)".to_string()
           ));
       }
       
       let op_type = op
           .get("op")
           .and_then(|v| v.as_str())
           .ok_or_else(|| ParseError::ValidationError(
               "Operator missing 'op' field".to_string()
           ))?;
       
       match op_type {
           "filter" => {
               op.get("predicate")
                   .ok_or_else(|| ParseError::ValidationError(
                       "Filter operator missing 'predicate' field".to_string()
                   ))?;
               
               if let Some(input) = op.get("input") {
                   validate_operator(input, depth + 1)?;
               }
           }
           
           "join" => {
               op.get("left")
                   .ok_or_else(|| ParseError::ValidationError(
                       "Join operator missing 'left' input".to_string()
                   ))?;
               op.get("right")
                   .ok_or_else(|| ParseError::ValidationError(
                       "Join operator missing 'right' input".to_string()
                   ))?;
               op.get("on")
                   .ok_or_else(|| ParseError::ValidationError(
                       "Join operator missing 'on' clause".to_string()
                   ))?;
           }
           
           "scan" => {
               let table = op
                   .get("table")
                   .and_then(|v| v.as_str())
                   .ok_or_else(|| ParseError::ValidationError(
                       "Scan operator missing table name".to_string()
                   ))?;
               
               if table.is_empty() {
                   return Err(ParseError::ValidationError(
                       "Table name cannot be empty".to_string()
                   ));
               }
           }
           
           "limit" => {
               let limit = op
                   .get("limit")
                   .and_then(|v| v.as_u64())
                   .ok_or_else(|| ParseError::ValidationError(
                       "Limit must be a positive number".to_string()
                   ))?;
               
               if limit == 0 {
                   return Err(ParseError::ValidationError(
                       "LIMIT 0 is not allowed".to_string()
                   ));
               }
               
               if limit > 10000 {
                   return Err(ParseError::ValidationError(
                       format!("LIMIT {} exceeds maximum of 10000", limit)
                   ));
               }
               
               if let Some(input) = op.get("input") {
                   validate_operator(input, depth + 1)?;
               }
           }
           
           "project" => {
               let projections = op
                   .get("projections")
                   .and_then(|v| v.as_array())
                   .ok_or_else(|| ParseError::ValidationError(
                       "Project operator missing projections array".to_string()
                   ))?;
               
               if projections.is_empty() {
                   return Err(ParseError::ValidationError(
                       "Project must have at least one projection".to_string()
                   ));
               }
               
               if let Some(input) = op.get("input") {
                   validate_operator(input, depth + 1)?;
               }
           }
           
           _ => {
               // Unknown operators are allowed for now
           }
       }
       
       Ok(())
   }
   ```

2. **Call validation in `convert_surql_to_dbsp`** (5 min)
   ```rust
   pub fn convert_surql_to_dbsp(sql: &str) -> Result<Value, ParseError> {
       let clean_sql = sql.trim().trim_end_matches(';');
       
       // ... parsing logic ...
       
       match parse_full_query(clean_sql) {
           Ok(("", plan)) => {
               // Validate before returning
               validate_plan(&plan)?;
               Ok(plan)
           }
           // ... error cases ...
       }
   }
   ```

3. **Test validation** (5 min)
   ```rust
   #[test]
   fn test_validation_catches_limit_zero() {
       let sql = "SELECT * FROM users LIMIT 0";
       let result = convert_surql_to_dbsp(sql);
       
       assert!(result.is_err());
       assert!(result.unwrap_err().to_string().contains("LIMIT 0"));
   }
   
   #[test]
   fn test_validation_catches_empty_table() {
       // This would require modifying parser to allow empty table
       // Just ensure parser rejects it
   }
   ```

**Checkpoint**: Invalid queries are caught with descriptive errors

---

## Phase 2: Add Missing SQL Features (3 hours)

### Step 2.1: Add `IN` Clause Support (45 min)

**Priority**: 📈 HIGH
**Risk**: Low (common feature, straightforward implementation)

#### Tasks:

1. **Add IN clause parser** (20 min)
   ```rust
   fn parse_in_clause(input: &str) -> IResult<&str, Value> {
       let (input, field) = ws(parse_identifier)(input)?;
       let (input, not) = opt(ws(tag_no_case("NOT")))(input)?;
       let (input, _) = ws(tag_no_case("IN"))(input)?;
       let (input, values) = delimited(
           ws(char('(')),
           separated_list1(ws(char(',')), ws(parse_value_entry)),
           ws(char(')')),
       )(input)?;
       
       // Convert ParsedValue to JSON
       let json_values: Vec<Value> = values
           .into_iter()
           .filter_map(|v| match v {
               ParsedValue::Json(val) => Some(val),
               ParsedValue::Identifier(id) => Some(json!(id)),
               ParsedValue::Prefix(p) => Some(json!(p)),
           })
           .collect();
       
       // Convert to OR of EQ predicates
       let predicates: Vec<Value> = json_values
           .into_iter()
           .map(|val| json!({
               "type": "eq",
               "field": &field,
               "value": val
           }))
           .collect();
       
       let result = if predicates.len() == 1 {
           predicates[0].clone()
       } else {
           json!({ "type": "or", "predicates": predicates })
       };
       
       // Wrap in NOT if necessary
       if not.is_some() {
           Ok((input, json!({
               "type": "not",
               "predicate": result
           })))
       } else {
           Ok((input, result))
       }
   }
   ```

2. **Add to `parse_leaf_predicate`** (5 min)
   ```rust
   fn parse_leaf_predicate(input: &str) -> IResult<&str, Value> {
       alt((
           parse_in_clause,        // ← Add this first
           parse_null_check,       // ← We'll add this next
           parse_existing_comparisons,
       ))(input)
   }
   
   // Rename current logic to:
   fn parse_existing_comparisons(input: &str) -> IResult<&str, Value> {
       // ... current parse_leaf_predicate code ...
   }
   ```

3. **Test IN clause** (20 min)
   ```rust
   #[test]
   fn test_in_clause_simple() {
       let sql = "SELECT * FROM users WHERE role IN ('admin', 'moderator')";
       let result = convert_surql_to_dbsp(sql);
       
       assert!(result.is_ok());
       let plan = result.unwrap();
       let plan_str = serde_json::to_string(&plan).unwrap();
       
       // Should create OR with EQ predicates
       assert!(plan_str.contains("\"type\":\"or\""));
       assert!(plan_str.contains("admin"));
       assert!(plan_str.contains("moderator"));
   }
   
   #[test]
   fn test_in_clause_with_numbers() {
       let sql = "SELECT * FROM posts WHERE category_id IN (1, 2, 3)";
       let result = convert_surql_to_dbsp(sql);
       assert!(result.is_ok());
   }
   
   #[test]
   fn test_not_in_clause() {
       let sql = "SELECT * FROM users WHERE role NOT IN ('banned', 'suspended')";
       let result = convert_surql_to_dbsp(sql);
       
       assert!(result.is_ok());
       let plan_str = serde_json::to_string(&result.unwrap()).unwrap();
       assert!(plan_str.contains("\"type\":\"not\""));
   }
   ```

**Checkpoint**: IN and NOT IN clauses work correctly

---

### Step 2.2: Add `IS NULL` / `IS NOT NULL` Support (30 min)

**Priority**: 📈 HIGH
**Risk**: Low

#### Tasks:

1. **Add NULL check parser** (15 min)
   ```rust
   fn parse_null_check(input: &str) -> IResult<&str, Value> {
       let (input, field) = ws(parse_identifier)(input)?;
       let (input, _) = ws(tag_no_case("IS"))(input)?;
       let (input, not) = opt(ws(tag_no_case("NOT")))(input)?;
       let (input, _) = ws(tag_no_case("NULL"))(input)?;
       
       let predicate_type = if not.is_some() {
           "is_not_null"
       } else {
           "is_null"
       };
       
       Ok((input, json!({
           "type": predicate_type,
           "field": field
       })))
   }
   ```

2. **Add to predicates** (already done in Step 2.1)

3. **Update Predicate enum in operators.rs** (10 min)
   ```rust
   // In your operators.rs file
   #[derive(Serialize, Deserialize, Clone, Debug)]
   #[serde(tag = "type", rename_all = "snake_case")]
   pub enum Predicate {
       // ... existing variants ...
       
       #[serde(rename = "is_null")]
       IsNull { field: Path },
       
       #[serde(rename = "is_not_null")]
       IsNotNull { field: Path },
   }
   ```

4. **Update view.rs to handle NULL checks** (5 min)
   ```rust
   // In view.rs check_predicate function
   fn check_predicate(...) -> bool {
       match pred {
           // ... existing cases ...
           
           Predicate::IsNull { field } => {
               if let Some(row_val) = self.get_row_value(key, db) {
                   if let Some(val) = resolve_nested_value(Some(row_val), field) {
                       matches!(val, SpookyValue::Null)
                   } else {
                       true  // Field doesn't exist = null
                   }
               } else {
                   true
               }
           }
           
           Predicate::IsNotNull { field } => {
               if let Some(row_val) = self.get_row_value(key, db) {
                   if let Some(val) = resolve_nested_value(Some(row_val), field) {
                       !matches!(val, SpookyValue::Null)
                   } else {
                       false  // Field doesn't exist = not present
                   }
               } else {
                   false
               }
           }
       }
   }
   ```

**Checkpoint**: NULL checks work in queries

---

### Step 2.3: Add `BETWEEN` Support (30 min)

**Priority**: 📊 MEDIUM
**Risk**: Low

#### Tasks:

1. **Add BETWEEN parser** (15 min)
   ```rust
   fn parse_between_clause(input: &str) -> IResult<&str, Value> {
       let (input, field) = ws(parse_identifier)(input)?;
       let (input, not) = opt(ws(tag_no_case("NOT")))(input)?;
       let (input, _) = ws(tag_no_case("BETWEEN"))(input)?;
       let (input, low) = ws(parse_value_entry)(input)?;
       let (input, _) = ws(tag_no_case("AND"))(input)?;
       let (input, high) = ws(parse_value_entry)(input)?;
       
       let low_val = match low {
           ParsedValue::Json(v) => v,
           _ => json!(null),
       };
       
       let high_val = match high {
           ParsedValue::Json(v) => v,
           _ => json!(null),
       };
       
       let result = json!({
           "type": "and",
           "predicates": [
               { "type": "gte", "field": &field, "value": low_val },
               { "type": "lte", "field": &field, "value": high_val }
           ]
       });
       
       if not.is_some() {
           Ok((input, json!({
               "type": "not",
               "predicate": result
           })))
       } else {
           Ok((input, result))
       }
   }
   ```

2. **Add to parse_leaf_predicate** (already in alternatives list)

3. **Test** (15 min)
   ```rust
   #[test]
   fn test_between_clause() {
       let sql = "SELECT * FROM products WHERE price BETWEEN 10 AND 100";
       let result = convert_surql_to_dbsp(sql);
       
       assert!(result.is_ok());
       let plan_str = serde_json::to_string(&result.unwrap()).unwrap();
       assert!(plan_str.contains("gte"));
       assert!(plan_str.contains("lte"));
   }
   
   #[test]
   fn test_not_between_clause() {
       let sql = "SELECT * FROM products WHERE price NOT BETWEEN 50 AND 200";
       let result = convert_surql_to_dbsp(sql);
       
       assert!(result.is_ok());
       let plan_str = serde_json::to_string(&result.unwrap()).unwrap();
       assert!(plan_str.contains("\"type\":\"not\""));
   }
   ```

**Checkpoint**: BETWEEN works correctly

---

### Step 2.4: Add `DISTINCT` Support (45 min)

**Priority**: 📊 MEDIUM
**Risk**: Medium (requires new operator)

#### Tasks:

1. **Add DISTINCT to parser** (10 min)
   ```rust
   fn parse_full_query(input: &str) -> IResult<&str, Value> {
       let (input, _) = ws(tag_no_case("SELECT"))(input)?;
       
       // Check for DISTINCT
       let (input, distinct) = opt(ws(tag_no_case("DISTINCT")))(input)?;
       
       let (input, fields) = separated_list1(ws(char(',')), parse_projection_item)(input)?;
       
       // ... rest of parsing ...
       
       // Build operators...
       let mut current_op = json!({ "op": "scan", "table": table });
       
       // ... filters, joins, etc. ...
       
       // Add DISTINCT operator if specified
       if distinct.is_some() {
           current_op = json!({
               "op": "distinct",
               "input": current_op
           });
       }
       
       Ok((input, current_op))
   }
   ```

2. **Add Distinct operator** (15 min)
   
   In `operators.rs`:
   ```rust
   #[derive(Serialize, Deserialize, Clone, Debug)]
   #[serde(tag = "op", rename_all = "lowercase")]
   pub enum Operator {
       // ... existing variants ...
       
       Distinct {
           input: Box<Operator>,
       },
   }
   ```

3. **Implement distinct in view.rs** (15 min)
   ```rust
   // In eval_snapshot function
   fn eval_snapshot(...) -> Cow<'a, ZSet> {
       match op {
           // ... existing cases ...
           
           Operator::Distinct { input } => {
               let upstream = self.eval_snapshot(input, db, context);
               
               // DISTINCT just ensures each key appears once (weight = 1)
               let mut out = FastMap::default();
               for (key, _) in upstream.as_ref() {
                   out.insert(key.clone(), 1);
               }
               Cow::Owned(out)
           }
       }
   }
   ```

4. **Test** (5 min)
   ```rust
   #[test]
   fn test_distinct() {
       let sql = "SELECT DISTINCT category FROM products";
       let result = convert_surql_to_dbsp(sql);
       
       assert!(result.is_ok());
       let plan_str = serde_json::to_string(&result.unwrap()).unwrap();
       assert!(plan_str.contains("\"op\":\"distinct\""));
   }
   ```

**Checkpoint**: DISTINCT keyword supported

---

## Phase 3: Optimization & Testing (3 hours)

### Step 3.1: Reduce Allocations (1.5 hours)

**Priority**: 📊 MEDIUM
**Risk**: Low (performance only)

#### Tasks:

1. **Use `&str` in intermediate parsing** (45 min)
   ```rust
   // Change parse_identifier to return &str
   fn parse_identifier(input: &str) -> IResult<&str, &str> {
       recognize(pair(
           alt((alpha1, tag("_"))),
           take_while(|c: char| c.is_alphanumeric() || c == '_' || c == ':' || c == '.'),
       ))(input)
   }
   
   // Update all call sites to convert to String only when building JSON
   ```

2. **Use Cow for conditional allocation** (30 min)
   ```rust
   use std::borrow::Cow;
   
   #[derive(Debug, Clone)]
   enum ParsedValue<'a> {
       Json(Value),
       Identifier(Cow<'a, str>),
       Prefix(Cow<'a, str>),
   }
   ```

3. **Benchmark before/after** (15 min)
   ```rust
   #[bench]
   fn bench_parse_simple_query(b: &mut Bencher) {
       let sql = "SELECT * FROM users WHERE active = true";
       b.iter(|| {
           convert_surql_to_dbsp(sql)
       });
   }
   
   #[bench]
   fn bench_parse_complex_query(b: &mut Bencher) {
       let sql = "SELECT *, (SELECT * FROM posts WHERE author_id = $parent.id) AS posts FROM users WHERE role IN ('admin', 'moderator') ORDER BY created_at DESC LIMIT 10";
       b.iter(|| {
           convert_surql_to_dbsp(sql)
       });
   }
   ```

**Checkpoint**: Parser allocates 30-50% less memory

---

### Step 3.2: Comprehensive Test Suite (1.5 hours)

**Priority**: 🔥 HIGH
**Risk**: None (testing is always good)

#### Tasks:

1. **Create test file structure** (15 min)
   ```
   tests/
     parser_tests.rs
       - Basic queries
       - WHERE clauses
       - Joins
       - Subqueries
       - Edge cases
       - Error cases
   ```

2. **Write basic query tests** (20 min)
   ```rust
   #[test]
   fn test_select_all() {
       assert!(convert_surql_to_dbsp("SELECT * FROM users").is_ok());
   }
   
   #[test]
   fn test_select_specific_fields() {
       assert!(convert_surql_to_dbsp("SELECT name, email FROM users").is_ok());
   }
   
   #[test]
   fn test_where_simple() {
       assert!(convert_surql_to_dbsp("SELECT * FROM users WHERE active = true").is_ok());
   }
   ```

3. **Write WHERE clause tests** (20 min)
   ```rust
   #[test]
   fn test_where_and() {
       let sql = "SELECT * FROM users WHERE active = true AND age > 18";
       assert!(convert_surql_to_dbsp(sql).is_ok());
   }
   
   #[test]
   fn test_where_or() {
       let sql = "SELECT * FROM users WHERE role = 'admin' OR role = 'moderator'";
       assert!(convert_surql_to_dbsp(sql).is_ok());
   }
   
   #[test]
   fn test_where_complex_nested() {
       let sql = "SELECT * FROM posts WHERE (published = true AND featured = true) OR author = 'admin'";
       let result = convert_surql_to_dbsp(sql);
       assert!(result.is_ok());
   }
   ```

4. **Write join tests** (15 min)
   ```rust
   #[test]
   fn test_join_detection() {
       let sql = "SELECT * FROM posts WHERE author_id = users.id";
       let result = convert_surql_to_dbsp(sql);
       
       assert!(result.is_ok());
       let plan = result.unwrap();
       assert_eq!(plan.get("op").and_then(|v| v.as_str()), Some("join"));
   }
   ```

5. **Write subquery tests** (15 min)
   ```rust
   #[test]
   fn test_single_subquery() {
       let sql = "SELECT *, (SELECT * FROM comments WHERE post_id = $parent.id) AS comments FROM posts";
       assert!(convert_surql_to_dbsp(sql).is_ok());
   }
   
   #[test]
   fn test_multiple_subqueries() {
       let sql = r#"
           SELECT *,
               (SELECT * FROM user WHERE id = $parent.author LIMIT 1)[0] AS author,
               (SELECT * FROM comments WHERE post_id = $parent.id) AS comments
           FROM posts
       "#;
       assert!(convert_surql_to_dbsp(sql).is_ok());
   }
   ```

6. **Write error case tests** (15 min)
   ```rust
   #[test]
   fn test_empty_query() {
       assert!(convert_surql_to_dbsp("").is_err());
   }
   
   #[test]
   fn test_incomplete_query() {
       assert!(convert_surql_to_dbsp("SELECT * FROM").is_err());
   }
   
   #[test]
   fn test_invalid_syntax() {
       assert!(convert_surql_to_dbsp("SELECT * FOOBAR users").is_err());
   }
   
   #[test]
   fn test_limit_zero() {
       let result = convert_surql_to_dbsp("SELECT * FROM users LIMIT 0");
       assert!(result.is_err());
       assert!(result.unwrap_err().to_string().contains("LIMIT 0"));
   }
   ```

7. **Write new feature tests** (10 min)
   ```rust
   #[test]
   fn test_in_clause() { /* ... */ }
   
   #[test]
   fn test_is_null() { /* ... */ }
   
   #[test]
   fn test_between() { /* ... */ }
   
   #[test]
   fn test_distinct() { /* ... */ }
   ```

**Checkpoint**: 50+ tests covering all features

---

## Phase 4: Documentation & Polish (30 min)

### Step 4.1: Add Documentation (20 min)

1. **Document supported SQL features** (10 min)
   ```rust
   //! # SurQL to DBSP Parser
   //!
   //! Converts SurrealDB-style SQL queries into DBSP operator trees.
   //!
   //! ## Supported Features
   //!
   //! ### SELECT Queries
   //! - `SELECT * FROM table`
   //! - `SELECT field1, field2 FROM table`
   //! - `SELECT DISTINCT field FROM table`
   //!
   //! ### WHERE Clauses
   //! - Comparison: `=`, `!=`, `>`, `>=`, `<`, `<=`
   //! - Logic: `AND`, `OR`, parentheses for grouping
   //! - NULL checks: `IS NULL`, `IS NOT NULL`
   //! - Lists: `IN (1, 2, 3)`, `NOT IN ('a', 'b')`
   //! - Ranges: `BETWEEN 10 AND 100`
   //! - Prefix matching: `field = 'prefix*'`
   //!
   //! ### Joins
   //! - Implicit joins via field comparison: `WHERE posts.author_id = users.id`
   //!
   //! ### Subqueries
   //! - In SELECT: `(SELECT * FROM table WHERE ...) AS alias`
   //! - Array indexing: `(SELECT ... LIMIT 1)[0]`
   //! - Parameter references: `WHERE id = $parent.field`
   //!
   //! ### Ordering & Limiting
   //! - `ORDER BY field [ASC|DESC]`
   //! - `LIMIT n`
   //!
   //! ## Examples
   //!
   //! ```rust
   //! use your_crate::convert_surql_to_dbsp;
   //!
   //! // Simple query
   //! let plan = convert_surql_to_dbsp("SELECT * FROM users WHERE active = true")?;
   //!
   //! // With subquery
   //! let plan = convert_surql_to_dbsp(r#"
   //!     SELECT *,
   //!         (SELECT * FROM posts WHERE author_id = $parent.id) AS posts
   //!     FROM users
   //!     WHERE role IN ('admin', 'moderator')
   //!     ORDER BY created_at DESC
   //!     LIMIT 10
   //! "#)?;
   //! ```
   ```

2. **Add inline comments** (10 min)
   ```rust
   // Add comments to complex functions
   /// Parses a complete SQL query into a DBSP operator tree.
   ///
   /// The parser builds the tree bottom-up:
   /// 1. Scan operator for the FROM clause
   /// 2. Filter operators for WHERE conditions
   /// 3. Join operators for implicit joins
   /// 4. Project operators for SELECT fields
   /// 5. Limit operators for LIMIT/ORDER BY
   fn parse_full_query(input: &str) -> IResult<&str, Value> {
       // ...
   }
   ```

**Checkpoint**: Code is well-documented

---

### Step 4.2: Final Testing & Cleanup (10 min)

1. **Run all tests** (5 min)
   ```bash
   cargo test --all-features
   cargo clippy -- -D warnings
   cargo fmt --check
   ```

2. **Check coverage** (5 min)
   ```bash
   cargo tarpaulin --ignore-tests
   ```
   Target: >80% coverage

**Checkpoint**: All tests pass, no warnings

---

## Implementation Schedule

### Day 1 (4 hours):
- ✅ Phase 1: Critical Bug Fixes (2 hours)
- ✅ Phase 2.1: IN clause (45 min)
- ✅ Phase 2.2: NULL checks (30 min)
- ✅ Phase 2.3: BETWEEN (30 min)
- Break (15 min)

### Day 2 (4 hours):
- ✅ Phase 2.4: DISTINCT (45 min)
- ✅ Phase 3.1: Optimizations (1.5 hours)
- ✅ Phase 3.2: Test Suite (1.5 hours)
- Break (15 min)

### Day 3 (1 hour):
- ✅ Phase 4: Documentation & Polish (30 min)
- ✅ Final review and testing (30 min)

---

## Success Criteria

### Functionality:
- ✅ All unsafe `unwrap()` calls removed
- ✅ Error messages show position and context
- ✅ Query validation catches bad queries
- ✅ IN, NULL, BETWEEN, DISTINCT all work
- ✅ All existing tests still pass
- ✅ 50+ new tests added

### Quality:
- ✅ No clippy warnings
- ✅ Code formatted with rustfmt
- ✅ >80% test coverage
- ✅ Documentation complete

### Performance:
- ✅ 30-50% reduction in allocations
- ✅ No performance regression

---

## Rollback Plan

If something goes wrong:

1. **Keep changes in feature branch** until fully tested
2. **Run tests after each phase** - don't move forward if tests fail
3. **Git commit after each step** - easy to revert
4. **Tag stable points** - easy to go back to known-good state

```bash
git checkout -b parser-improvements
# After Phase 1
git commit -m "Phase 1: Critical bug fixes"
git tag parser-phase1

# After Phase 2
git commit -m "Phase 2: SQL features"
git tag parser-phase2

# etc.
```

---

## Post-Implementation

### Monitor:
- Parser error rates in production
- Most common parse errors (to improve messages)
- Performance impact

### Future Enhancements:
- GROUP BY / HAVING
- CASE expressions
- String functions (UPPER, LOWER, etc.)
- Math operators
- Window functions

---

## Quick Reference Checklist

```
Phase 1: Critical Fixes (2 hours)
[ ] Remove unwrap() calls
[ ] Add custom error types
[ ] Implement error context
[ ] Add query validation
[ ] Test error messages

Phase 2: SQL Features (3 hours)
[ ] IN clause
[ ] NULL checks
[ ] BETWEEN
[ ] DISTINCT
[ ] Update operators.rs
[ ] Update view.rs evaluation

Phase 3: Testing (3 hours)
[ ] Optimize allocations
[ ] Create comprehensive tests
[ ] Add benchmarks
[ ] Verify coverage

Phase 4: Documentation (30 min)
[ ] Add module docs
[ ] Add inline comments
[ ] Update README
[ ] Final testing
```

---

**Total Time**: 8-10 hours
**Difficulty**: Medium
**Risk**: Low (incremental changes with tests)

**Start with Phase 1** - it has the highest impact and catches bugs early! 🚀