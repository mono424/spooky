# Converter Module Documentation (`converter.rs`)

This document provides a detailed breakdown of the `converter.rs` module. This module is responsible for parsing **SurQL** (SurrealDB Query Language) strings and converting them into a **JSON-based Query Plan** (specifically a structure compatible with the `Operator` enum used in the query engine).

It uses the [nom](https://github.com/rust-bakery/nom) parser combinator library for robust and composable parsing.

---

## 1. Query Plan Structure & Logic

### Purpose
The **Query Plan** serves as an intermediate representation (IR) between the raw SurQL string and the execution engine's internal state. It is a tree structure where:
- **Leaves** are data sources (tables).
- **Nodes** are operators (filtering, joining, projecting, limiting).
- **Root** represents the final result set.

### Operator Order & Execution Flow
**Is the order important?**
**Yes, strictly.** The JSON structure represents the *execution tree* directly. The outer-most operator is the *last* to be executed.
- `Limit(Project(Filter(Scan)))` means:
    1.  **Scan** the table (get all rows).
    2.  **Filter** (discard rows).
    3.  **Project** (transform remaining rows).
    4.  **Limit** (take the first N).

**Can it be "ordered smart"? (Optimization)**
The *current* implementation builds the tree in a fixed, naive order based on SQL semantics:
`FROM` -> `WHERE` (Joins then Filters) -> `SELECT` (Projections) -> `LIMIT`.

However, the Query Engine *could* reorder this tree (Query Optimization) before execution. For example:
- **Predicate Pushdown**: Moving a `filter` deeper inside a `join` to reduce the dataset early.
- **Projection Pushdown**: Moving `project` down to scan to avoid carrying unused columns.

### Root Operator Calculation
The root is calculated bottom-up in `parse_full_query`:
1.  **Start** with a `Scan` of the table found in `FROM`.
2.  **Wrap** in logic `Filter`s or `Join`s if `WHERE` exists.
3.  **Wrap** in `Project` if `SELECT` fields are not just `*`.
4.  **Wrap** in `Limit` if `LIMIT` exists.
The final wrapper is returned as the **Root Operator**.

---

## 2. Public API

### `convert_surql_to_dbsp`
**Description:**
The main entry point. Cleans input and parses it.

**Input:**
- `sql`: `"SELECT name FROM user WHERE age > 10;"`

**Output:**
```json
{
  "op": "project",
  "projections": [{ "type": "field", "name": "name" }],
  "input": {
    "op": "filter",
    "predicate": { "type": "gt", "field": "age", "value": 10 },
    "input": { "op": "scan", "table": "user" }
  }
}
```

**Explanation:**
The function trims the trailing `;`, calls `parse_full_query`, and returns the JSON plan.

**Complexity:** $O(N)$
**Optimizations:** Zero-copy parsing (using `&str` instead of `String`).

---

## 3. Helper Functions

### `ws`
**Description:**
Wraps a parser to ignore surrounding whitespace.

**Input:** `  my_table  ` (and the inner parser expects "my_table")
**Output:** `"my_table"` (inner parser result)

**Explanation:**
It runs `multispace0` (consume spaces), runs the inner parser, then `multispace0` again.

**Complexity:** $O(W + P)$ (Whitespace length + Inner parser complexity)

---

### `parse_identifier`
**Description:**
Parses table names, columns, or aliases.

**Input:** `"user_table.id"`
**Output:** `"user_table.id"` (String)

**Explanation:**
It scans for valid identifier characters (alphanumeric, `_`, `.`, `:`) and returns them as a String.

**Complexity:** $O(L)$

---

## 4. Value Parsing

### `parse_string_literal`
**Description:**
Parses quotes and handles prefix wildcards.

**Example 1:**
- **Input:** `'hello world'`
- **Output:** `ParsedValue::Json(String("hello world"))`

**Example 2 (Prefix):**
- **Input:** `"user_*"`
- **Output:** `ParsedValue::Prefix("user_")`

**Explanation:**
It detects the quote usage. If the string ends in `*`, it returns a `Prefix` variant, used for prefix scans; otherwise, a standard JSON string.

**Complexity:** $O(L)$

---

### `parse_value_entry`
**Description:**
Parses individual values.

**Example 1:**
- **Input:** `123`
- **Output:** `ParsedValue::Json(Number(123))`

**Example 2:**
- **Input:** `$parent`
- **Output:** `ParsedValue::Json({ "$param": "parent" })`

**Explanation:**
It tries multiple parsers in order: logical boolean -> param variable -> number -> identifier -> string.

**Complexity:** $O(1)$ to $O(L)$

---

## 5. Logic & Predicate Parsing

### `parse_leaf_predicate`
**Description:**
Parses a single comparison condition.

**Example 1 (Filter):**
- **Input:** `age >= 18`
- **Output:**
```json
{ "type": "gte", "field": "age", "value": 18 }
```

**Example 2 (Join Candidate):**
- **Input:** `author = user.id`
- **Output:**
```json
{ "type": "__JOIN_CANDIDATE__", "left": "author", "right": "user.id" }
```

**Explanation:**
It parses `Left OP Right`. It checks the `Right` side: if it's a value, it's a standard filter (`type: gte`). If it's another identifier (like `user.id`), it flags it as a `__JOIN_CANDIDATE__`.

**Complexity:** $O(1)$

---

### `parse_where_logic` (& `parse_and`/`parse_or`)
**Description:**
Parses the full `WHERE` clause logic tree.

**Input:** `WHERE age > 18 AND active = true`
**Output:**
```json
{
  "type": "and",
  "predicates": [
    { "type": "gt", "field": "age", "value": 18 },
    { "type": "eq", "field": "active", "value": true }
  ]
}
```

**Explanation:**
1.  Parser sees `WHERE`.
2.  Enters `parse_or_expression`.
3.  Enters `parse_and_expression`.
4.  Parses `age > 18` and `active = true` as terms.
5.  Returns a composite `AND` object.

**Complexity:** $O(N)$ terms.

---

## 6. Main Query Clauses

### `parse_limit_clause`
**Input:** `LIMIT 50`
**Output:** `50` (usize)
**Complexity:** $O(1)$

### `parse_order_clause`
**Input:** `ORDER BY age DESC, name`
**Output:**
```json
[
  { "field": "age", "direction": "DESC" },
  { "field": "name", "direction": "ASC" }
]
```
**Complexity:** $O(N)$ fields.

---

## 7. Projections (SELECT List)

### `match_subquery_projection`
**Description:**
Parses scalar subqueries in the select list.

**Input:** `(SELECT name FROM user WHERE id = $parent.author)[0] AS author_name`
**Output:**
```json
{
  "type": "subquery",
  "alias": "author_name",
  "plan": { ... subquery plan ... }
}
```

**Explanation:**
Recursive call. It parses the inner query completely, captures the optional index `[0]`, and the alias.

**Complexity:** Recursive cost.

### `parse_field_projection`
**Input:** `my_field`
**Output:** `{ "type": "field", "name": "my_field" }`

### `parse_full_query`
**Description:**
Builds the entire Operator Tree.

**Input:** `SELECT name FROM user WHERE age > 20 LIMIT 5`
**Step-by-Step Execution:**
1.  **Parse SELECT**: `name` (Project List).
2.  **Parse FROM**: `user` (Table). -> **Current:** `{ op: scan, table: user }`
3.  **Parse WHERE**: `age > 20`.
    - Function `wrap_conditions` is called.
    - Result wraps Scan: `{ op: filter, predicate: {age > 20}, input: {scan...} }`.
4.  **Parse SELECT**: List is not empty/Wildcard.
    - Wraps current: `{ op: project, projections: [name], input: {filter...} }`.
5.  **Parse LIMIT**: `5`.
    - Wraps current: `{ op: limit, limit: 5, input: {project...} }`.
6.  **Return Root**.

**Input:** `SELECT * FROM user`
**Step-by-Step:**
1.  **Project List**: `*` (`{type: all}`).
2.  **Wrappers**:
    - No Where -> Skip.
    - Project is `type: all` -> Skip wrapping.
    - No Limit -> Skip.
3.  **Returns**: `{ op: scan, table: user }`.

**Complexity:** Linear $O(N)$.

---

## 8. Tree Building Logic

### `wrap_conditions`
**Description:**
Intelligently distributes `WHERE` clauses into `Filter` or `Join` operators.

**Input OP:** `{ op: scan, table: thread }`
**Predicate:** `author = user.id AND topic = 'tech'`

**Execution:**
1.  **Normalize**: Splits `AND` into `[author = user.id, topic = 'tech']`.
2.  **Classify**:
    - `author = user.id` -> `Right` is identifier -> **Join Candidate**.
    - `topic = 'tech'` -> `Right` is literal -> **Filter**.
3.  **Apply Joins**:
    - Wraps input scan with `Join`.
    - Left: `{scan thread}`
    - Right: `{scan user}` (Derived from `user.id`)
    - On: `left: author`, `right: id`.
4.  **Apply Filters**:
    - Wraps the Join with `Filter`.
    - Predicate: `topic = 'tech'`.
5.  **Output**: `Filter(Join(Scan(thread), Scan(user)))`.

**Complexity:** $O(P)$ predicates.
