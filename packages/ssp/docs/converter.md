# Converter Module Documentation (`converter.rs`)

This document provides a detailed breakdown of the `converter.rs` module. This module is responsible for parsing **SurQL** (SurrealDB Query Language) strings and converting them into a **JSON-based Query Plan** (specifically a structure compatible with the `Operator` enum used in the query engine).

It uses the [nom](https://github.com/rust-bakery/nom) parser combinator library for robust and composable parsing.

---

## 1. Public API

### `convert_surql_to_dbsp`
**Description:**
The main entry point for the module. It takes a raw SurQL query string, cleans it, and attempts to parse it into a JSON Query Plan (`serde_json::Value`).

**Usage:**
```rust
let sql = "SELECT * FROM users WHERE age > 18;";
let plan = convert_surql_to_dbsp(sql)?;
```

**Input:**
- `sql: &str` - The raw SQL query string.

**Output:**
- `Result<Value>` - A JSON object representing the operator tree on success, or an `anyhow::Error` on failure.

**Complexity (Big O):**
- **Time:** $O(N)$ where $N$ is the length of the SQL string. `nom` parses linearly.
- **Space:** $O(D)$ where $D$ is the depth of the resulting query tree (AST construction).

**Future Optimizations:**
- **Zero-copy parsing:** Currently, it allocates Strings for identifiers and values. Using `Cow<str>` or `&str` references in the AST until the final serialization step could reduce allocations.
- **Pre-allocation:** Pre-allocating vectors for fields/predicates if sizes are known or estimated.

---

## 2. Query Plan Structure

### Purpose
The **Query Plan** serves as an intermediate representation (IR) between the raw SurQL string and the execution engine's internal state. It is a tree structure where:
- **Leaves** are data sources (tables).
- **Nodes** are operators (filtering, joining, projecting, limiting).
- **Root** represents the final result set.

This JSON plan can be serialized, transmitted, or stored, allowing the query parsing logic to be decoupled from the execution engine (e.g., parsing on a server and executing on a client or edge node).

### Structure & Appearance
The plan is a recursive JSON object. Each node has an `op` field defining its operation type and specific fields for that operation.

#### Example Plan
**Source SQL:**
```sql
SELECT name, age FROM users WHERE age >= 18 LIMIT 10
```

**JSON Output:**
```json
{
  "op": "limit",
  "limit": 10,
  "input": {
    "op": "project",
    "projections": [
      { "type": "field", "name": "name" },
      { "type": "field", "name": "age" }
    ],
    "input": {
      "op": "filter",
      "predicate": {
        "type": "gte",
        "field": "age",
        "value": 18
      },
      "input": {
        "op": "scan",
        "table": "users"
      }
    }
  }
}
```

#### Node Types
- **`scan`**: The leaf node. `{ "op": "scan", "table": "users" }`
- **`filter`**: Filters rows. `{ "op": "filter", "predicate": {...}, "input": {...} }`
- **`join`**: Joins two inputs. `{ "op": "join", "left": {...}, "right": {...}, "on": {...} }`
- **`project`**: Selects/transforms fields. `{ "op": "project", "projections": [...], "input": {...} }`
- **`limit`**: Limits result count. `{ "op": "limit", "limit": 10, "input": {...} }`

---

## 3. Helper Functions

### `ws`
**Description:**
A combinator wrapper that ignores surrounding whitespace (spaces, tabs, newlines) around another parser.

**Input:**
- `inner: F` - The parser to wrap.
- `input: &str` - The input string.

**Output:**
- `IResult<&str, O>` - The result of the inner parser.

**Complexity:**
- **Time:** $O(W + P)$ where $W$ is the length of whitespace and $P$ is the cost of the inner parser.
- **Space:** $O(1)$.

---

### `parse_identifier`
**Description:**
Parses valid identifiers (table names, column names, aliases).
Rules:
- Starts with an alphabetic char or `_`.
- Followed by alphanumeric chars, `_`, `.`, or `:`.

**Input:**
- `input: &str`

**Output:**
- `IResult<&str, String>`

**Complexity:**
- **Time:** $O(L)$ where $L$ is the length of the identifier.
- **Space:** $O(L)$ (allocates a new String).

**Future Optimizations:**
- Return `&str` instead of `String` to avoid allocation during the parsing phase.

---

## 4. Value Parsing

### `ParsedValue` (Enum)
A helper enum used only during parsing to distinguish between raw JSON values, Identifiers (likely column references), and Prefixes.

### `parse_string_literal`
**Description:**
Parses single or double-quoted strings. Handles prefix wildcards (e.g., `'foo*'`).

**Input:**
- `input: &str`

**Output:**
- `IResult<&str, ParsedValue>`

**Complexity:**
- **Time:** $O(L)$ where $L$ is the length of the string literal.
- **Space:** $O(L)$.

### `parse_value_entry`
**Description:**
Parses a "value" which can be:
- Boolean (`true`, `false`)
- Parameter (`$param`)
- Number (Integer)
- String Literal
- Identifier

**Input:**
- `input: &str`

**Output:**
- `IResult<&str, ParsedValue>`

**Complexity:**
- **Time:** $O(1)$ to $O(L)$ depending on the token type.
- **Space:** $O(L)$ for strings/identifiers.

---

## 5. Logic & Predicate Parsing

### `parse_leaf_predicate`
**Description:**
Parses a basic comparison (e.g., `age > 18`, `name = 'Alice'`).
Recognized Operators: `>=`, `<=`, `!=`, `=`, `>`, `<`, `CONTAINS`, `INSIDE`.

**Input:**
- `input: &str`

**Output:**
- `IResult<&str, Value>` - A JSON object representing the predicate.
  - Normal: `{ "type": "eq", "field": "age", "value": 18 }`
  - Join Candidate: `{ "type": "__JOIN_CANDIDATE__", "left": "author", "right": "user.id" }` (if the right side is an Identifier, it's assumed to be a join key).

**Complexity:**
- **Time:** $O(1)$ (fixed size components).
- **Space:** $O(1)$.

### `parse_term`
**Description:**
Parses a term in a logic expression. It handles parentheses for grouping.
`term ::= ( expression ) | leaf_predicate`

**Input:**
- `input: &str`

**Output:**
- `IResult<&str, Value>`

**Complexity:**
- **Time:** Depends on recursion depth.

### `parse_and_expression` & `parse_or_expression`
**Description:**
Parses `AND` and `OR` chains.
- `AND` binds tighter than `OR` (standard precedence).
- `parse_or` calls `parse_and`, which calls `parse_term`.

**Input:**
- `input: &str`

**Output:**
- `IResult<&str, Value>` - Returns a composite predicate `{ "type": "and", "predicates": [...] }` or the single term if no operator is found.

**Complexity:**
- **Time:** $O(N)$ for the number of terms.
- **Space:** $O(N)$ for the vector of terms.

**Future Optimizations:**
- Flatten nested AND/ORs during parsing (e.g., `A AND (B AND C)` -> `A AND B AND C`) to simplify the evaluation engine.

### `parse_where_logic`
**Description:**
Parses the `WHERE` clause, expecting an `OR` expression (root of logic tree).

---

## 6. Main Query Clauses

### `parse_limit_clause`
**Description:**
Parses `LIMIT <number>`.

**Input:**
- `input: &str`

**Output:**
- `IResult<&str, usize>`

**Complexity:**
- **Time:** $O(D)$ digits.
- **Space:** $O(1)$.

### `parse_order_clause`
**Description:**
Parses `ORDER BY field [ASC|DESC], ...`.

**Input:**
- `input: &str`

**Output:**
- `IResult<&str, Vec<Value>>`

**Complexity:**
- **Time:** $O(N)$ fields.
- **Space:** $O(N)$.

---

## 7. Projections (SELECT List)

### `match_subquery_projection`
**Description:**
Parses a subquery inside a projection, usually capturing a scalar value or array element.
Format: `(SELECT ...)[index] AS alias`

**Input:**
- `input: &str`

**Output:**
- `IResult<&str, Value>` - JSON with `{ "type": "subquery", "plan": ..., "alias": ... }`.

**Complexity:**
- **Time:** Recursive call to `parse_full_query`. Cost is sum of subquery complexity.
- **Space:** Recursive stack depth.

### `parse_field_projection`
**Description:**
Parses a simple field or `*`.

**Input:**
- `input: &str`

**Output:**
- `IResult<&str, Value>`

### `parse_full_query`
**Description:**
The core parser. Parses `SELECT ... FROM ... [WHERE ...] [ORDER BY ...] [LIMIT ...]`.
It constructs the initial Operator Tree (Scan -> [Join/Filter] -> [Project] -> [Limit]).

**Input:**
- `input: &str`

**Output:**
- `IResult<&str, Value>` - The root of the Operator JSON tree.

**Structure Built:**
1.  **Scan**: Starts with `{ "op": "scan", "table": "..." }`.
2.  **Filter/Join**: Calls `wrap_conditions` to wrap the Scan.
3.  **Project**: Wraps with `{ "op": "project", ... }` if specific fields are selected.
4.  **Limit**: Wraps with `{ "op": "limit", ... }` if a limit exists.

**Complexity:**
- **Time:** Linear scan of the query string components.
- **Space:** Proportional to the complexity of the query structure.

**Future Optimizations:**
- **Operator Fusion**: Combine `scan` + `filter` into a single `Scan` operator that accepts a predicate (push-down optimization) immediately during parsing, instead of wrapping.

---

## 8. Tree Building Logic

### `wrap_conditions`
**Description:**
Takes a generic operator (usually a Scan) and a WHERE clause predicate. It intelligently splits the predicate into **Joins** and **Filters**.

**Logic:**
1.  **Normalization**: Flattens top-level `AND`s.
2.  **Classification**: Checks if a predicate is a "Join Candidate" (comparing a field to an identifier like `$parent` or another table's field) or a "Filter".
3.  **Join Application (Bottom-Up)**: Wraps the input in `join` operators for every join candidate found.
4.  **Filter Application**: Wraps the result in a `filter` operator if there are remaining logic predicates.

**Input:**
- `input_op: Value` - The operator to wrap (e.g., Scan).
- `predicate: Value` - The WHERE clause logic.

**Output:**
- `Value` - The new root operator.

**Complexity:**
- **Time:** $O(P)$ where $P$ is the number of predicates.
- **Space:** $O(P)$.

**Future Optimizations:**
- **Join Reordering**: Currently applies joins in order of appearance. A query optimizer could reorder these based on statistics to minimize intermediate row counts.
- **Join Types**: Currently assumes a standard join logic. Could infer Left/Inner joins based on syntax.
